//! cookie-use — agent-friendly multi-account session manager.
//!
//! Sits on top of `chrome-use`: owns the account model, an encrypted vault, and
//! orchestration; delegates all browser/cookie I/O to chrome-use. Site-agnostic.

mod act_as;
mod chrome_use;
mod cloud;
mod confirm;
mod copy;
mod crypto;
mod fingerprint;
mod keychain;
mod runner;
mod share;
mod upgrade;
mod vault;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use vault::{Account, Status, Vault};

#[derive(Parser)]
#[command(
    name = "cookie-use",
    version,
    about = "Manage many logged-in sessions for any website"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Emit machine-readable JSON instead of human text (works on every
    /// subcommand). Never prints cookie values — only counts/metadata.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum CloudCmd {
    /// Save the server and credentials (uuid/password are generated if omitted;
    /// reuse the same three on every computer, or in the CookieCloud extension).
    Setup {
        /// CookieCloud server URL, e.g. https://cookiecloud.example.com
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        uuid: Option<String>,
        #[arg(long)]
        password: Option<String>,
        /// aes-128-cbc-fixed (default) or legacy (older CookieCloud extensions).
        #[arg(long, default_value = "aes-128-cbc-fixed")]
        crypto: String,
        /// Don't also publish the latest login per site for the CookieCloud extension.
        #[arg(long)]
        no_browser_compat: bool,
    },
    /// Show the sync settings (the password stays hidden; see `secret`).
    Status,
    /// Print the uuid and password, to set up another computer.
    Secret,
    /// Pull from the server, merge (newer copy of each account wins), push back.
    Sync {
        /// Overwrite data that the CookieCloud extension uploaded under this uuid.
        #[arg(long)]
        force: bool,
    },
    /// Pull and merge only; don't upload.
    Pull,
    /// List domains the CookieCloud browser extension uploaded under this uuid.
    Domains,
    /// Create an account from the CookieCloud extension's data for a site.
    Import {
        /// Domain(s), comma-separated.
        site: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        label: Option<String>,
    },
    /// Forget the sync settings (the server copy stays).
    Disconnect,
}

#[derive(Subcommand)]
enum Cmd {
    /// Import a logged-in session from a Chrome profile into the vault.
    Add {
        /// Source Chrome profile (directory name, display name, or "auto").
        #[arg(long = "from-profile")]
        from_profile: String,
        /// Domain(s) for the session, comma-separated (e.g. "chatgpt.com,openai.com").
        #[arg(long)]
        site: String,
        /// Vault id (default: "<site>/<n>").
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        hint: Option<String>,
        /// Also capture the primary origin's localStorage (one in-browser read
        /// via a throwaway browser). Useful for SPAs that keep token/user info
        /// in localStorage rather than cookies.
        #[arg(long = "with-localstorage")]
        with_localstorage: bool,
    },
    /// Import a session from a JSON cookie array or a Cookie-header file.
    Import {
        #[arg(long)]
        file: String,
        #[arg(long)]
        site: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        hint: Option<String>,
    },
    /// List stored accounts.
    List {
        /// Filter by website — a domain or full URL
        /// (e.g. `dash.cloudflare.com` or `https://dash.cloudflare.com/`).
        /// Forgiving: base-domain ↔ subdomain match, partial terms, and also
        /// searches account id / label. Lists everything, grouped by site, when omitted.
        site: Option<String>,
        /// Deprecated alias for the positional SITE argument.
        #[arg(long = "site", value_name = "SITE", conflicts_with = "site")]
        site_flag: Option<String>,
    },
    /// Show an account's metadata (never prints cookie values).
    Show { id: String },
    /// Apply an account's session into a browser target.
    Use {
        id: String,
        /// session:<name> (default) or isolated.
        #[arg(long, default_value = "session:default")]
        target: String,
        /// Don't open the site after applying.
        #[arg(long = "no-open")]
        no_open: bool,
        /// Rewrite cookie domains to this host on apply (e.g. "localhost"), so
        /// the session can be reused on a different origin for local testing.
        #[arg(long = "rewrite-domain")]
        rewrite_domain: Option<String>,
        /// Open this exact URL after applying instead of the account's site
        /// (e.g. "http://localhost:8001"). Needed when rewriting to a dev host.
        #[arg(long = "open-url")]
        open_url: Option<String>,
        /// Skip injecting the account's captured localStorage (injected by
        /// default when present and a page is opened).
        #[arg(long = "no-localstorage")]
        no_localstorage: bool,
        /// Skip the biometric/TTY confirmation before injecting the session.
        #[arg(long = "no-confirm")]
        no_confirm: bool,
    },
    /// Clear the target's cookies, then apply the account (clean switch).
    Switch {
        id: String,
        #[arg(long, default_value = "session:default")]
        target: String,
        #[arg(long = "no-open")]
        no_open: bool,
        #[arg(long = "rewrite-domain")]
        rewrite_domain: Option<String>,
        #[arg(long = "open-url")]
        open_url: Option<String>,
        #[arg(long = "no-localstorage")]
        no_localstorage: bool,
        /// Skip the biometric/TTY confirmation before injecting the session.
        #[arg(long = "no-confirm")]
        no_confirm: bool,
    },
    /// Replay a session onto a local dev origin for cross-origin QA testing.
    /// Sugar over `use --rewrite-domain <host> --open-url http://<host:port>`.
    Replay {
        id: String,
        /// Dev origin to replay onto, e.g. "localhost:8001" or "127.0.0.1:3000".
        #[arg(long = "to")]
        to: String,
        #[arg(long, default_value = "session:default")]
        target: String,
        #[arg(long = "no-confirm")]
        no_confirm: bool,
    },
    /// Export a password-encrypted session bundle to hand to a teammate.
    /// They redeem it with `cookie-use redeem` (which installs cookie-use).
    Share {
        id: String,
        /// Output bundle path (default: "<id-slug>.cusession").
        #[arg(long)]
        out: Option<String>,
        /// Bundle password. Prompted on the TTY if omitted.
        #[arg(long)]
        password: Option<String>,
    },
    /// Overwrite one Chrome profile's login for a site with another profile's.
    /// Only that site's cookies change; the destination's previous login is
    /// saved to the vault first (tag "backup") so it can be restored.
    Copy {
        /// Website (domain or comma list, e.g. "dash.cloudflare.com,cloudflare.com").
        #[arg(long)]
        site: String,
        /// Source profile: directory ("Profile 3"), display name, or email.
        #[arg(long)]
        from: String,
        /// Destination profile (must be open with the chrome-use extension).
        #[arg(long)]
        to: String,
        /// Keep destination cookies for the site that the source doesn't have.
        #[arg(long)]
        keep_extra: bool,
        /// Don't save the destination's previous login.
        #[arg(long)]
        no_backup: bool,
        /// Show what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Skip the biometric/TTY confirmation.
        #[arg(long)]
        no_confirm: bool,
    },
    /// Sync the vault between computers through a CookieCloud server
    /// (self-hosted or public; wire-compatible with CookieCloud).
    Cloud {
        #[command(subcommand)]
        action: CloudCmd,
    },
    /// Export many accounts into one password-encrypted bundle, to move them to
    /// another computer (`cookie-use redeem <file>` there). Everything by default.
    Export {
        /// Specific account ids (default: all, or those matching --site).
        ids: Vec<String>,
        /// Only accounts for this website (domain or URL, forgiving match).
        #[arg(long)]
        site: Option<String>,
        /// Output path (default: "cookie-use-<timestamp>.cusession").
        #[arg(long)]
        out: Option<String>,
        /// Bundle password. Prompted on the TTY if omitted.
        #[arg(long)]
        password: Option<String>,
    },
    /// Import a session bundle produced by `share` or `export` into the vault.
    /// Multi-account bundles merge: per account the newer copy wins.
    Redeem {
        /// Path to a .cusession bundle.
        bundle: String,
        #[arg(long)]
        password: Option<String>,
        /// Store under this id instead of the bundle's original id.
        #[arg(long)]
        id: Option<String>,
    },
    /// Open one or more accounts in isolated browser windows simultaneously.
    Run {
        /// A single account id. Omit and pass --site/--all for many.
        id: Option<String>,
        /// Open every account whose site matches this filter.
        #[arg(long)]
        site: Option<String>,
        /// Open every account in the vault (optionally narrowed by --site).
        #[arg(long)]
        all: bool,
    },
    /// Run a command in an environment scoped to an account's session
    /// (agent-friendly: lets an agent act as a specific account per task).
    As {
        id: String,
        #[arg(long, default_value = "session:default")]
        target: String,
        #[arg(long = "no-confirm")]
        no_confirm: bool,
        /// The command to run after the session is applied. Must follow `--`;
        /// everything past `--` is captured verbatim (hyphenated flags included),
        /// while cookie-use's own flags stay parseable before it.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Delete a single account from the vault.
    Revoke { id: String },
    /// Delete the entire vault (all accounts). Irreversible.
    Wipe {
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Export a hash-only fingerprint of an account's session cookies, so a
    /// separate tool can verify "is the live session this account?" without ever
    /// seeing a cookie value. Reads a plaintext cache when present (no decrypt).
    Fingerprint {
        /// Account id. Omit and pass --all to fingerprint every cached account.
        id: Option<String>,
        /// Fingerprint every account that already has a cached fingerprint,
        /// skipping (and listing on stderr) those without one.
        #[arg(long)]
        all: bool,
    },
    /// Update an account's liveness from its cookie expiry (generic heuristic).
    Check { id: String },
    /// Remove an account.
    Rm { id: String },
    /// Rename an account id.
    Rename { id: String, new_id: String },
    /// Edit an account's label, hint, note or tags. Pass "" to clear a field.
    Edit {
        id: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        hint: Option<String>,
        #[arg(long)]
        note: Option<String>,
        /// Replace the tags, comma-separated (e.g. "prod,admin"; "" clears).
        #[arg(long)]
        tags: Option<String>,
    },
    /// Upgrade the CLI from its GitHub release (`--check` only looks; `--skills` also refreshes the skill).
    ///
    /// The release tarball is sha256-verified and swapped in atomically; any
    /// failure keeps the current binary. brew / cargo / source installs are
    /// refused with the right command instead. Never opens the vault.
    /// Exit 0 ok, 2 check / download / verification failed, 1 refused or unfinished.
    Upgrade {
        /// Change nothing: print `cookie-use <current> -> <latest>` or
        /// `cookie-use <current> is up to date` (`--json` for the same as JSON).
        #[arg(long)]
        check: bool,
        /// Also refresh the cookie-use skill (Claude Code plugin, its git
        /// checkout); without it the skill copies are only listed.
        #[arg(long)]
        skills: bool,
        /// Install this release instead of the latest (e.g. v0.4.0; allows downgrade).
        #[arg(long, value_name = "vX.Y.Z")]
        tag: Option<String>,
    },
    /// Daily update check, run detached by the notice (internal).
    #[command(name = "__update-check", hide = true)]
    UpdateCheck,
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    // clap has already handled (and exited for) --version and --help here.
    match cli.cmd {
        Cmd::Upgrade { check, skills, tag } => upgrade::run_upgrade(check, json, skills, tag),
        Cmd::UpdateCheck => return upgrade::run_update_check(),
        // In --json mode the notice waits until the command succeeded: on
        // failure stderr must be exactly the error envelope the GUI decodes.
        _ => upgrade::maybe_notify_update(!json),
    }
    let result = run(cli);
    if json && result.is_ok() {
        upgrade::print_notice();
    }
    if let Err(e) = result {
        if json {
            // Uniform error envelope on stderr so a GUI can parse failures
            // (exit codes stay coarse — always 1).
            let msg = format!("{e:#}");
            eprintln!(
                "{}",
                serde_json::to_string(&json!({ "error": msg }))
                    .unwrap_or_else(|_| format!("{{\"error\":{:?}}}", format!("{e:#}")))
            );
        } else {
            eprintln!("error: {e:#}");
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.cmd {
        Cmd::Add {
            from_profile,
            site,
            id,
            label,
            hint,
            with_localstorage,
        } => cmd_add(
            &from_profile,
            &site,
            id,
            label,
            hint,
            with_localstorage,
            json,
        ),
        Cmd::Import {
            file,
            site,
            id,
            label,
            hint,
        } => cmd_import(&file, &site, &id, label, hint, json),
        Cmd::List { site, site_flag } => cmd_list(site.or(site_flag).as_deref(), json),
        Cmd::Show { id } => cmd_show(&id, json),
        Cmd::Use {
            id,
            target,
            no_open,
            rewrite_domain,
            open_url,
            no_localstorage,
            no_confirm,
        } => cmd_apply(ApplyArgs {
            id: &id,
            target: &target,
            open: !no_open,
            clear_first: false,
            rewrite_domain: rewrite_domain.as_deref(),
            open_url: open_url.as_deref(),
            inject_localstorage: !no_localstorage,
            confirm: !no_confirm,
            json,
        }),
        Cmd::Switch {
            id,
            target,
            no_open,
            rewrite_domain,
            open_url,
            no_localstorage,
            no_confirm,
        } => cmd_apply(ApplyArgs {
            id: &id,
            target: &target,
            open: !no_open,
            clear_first: true,
            rewrite_domain: rewrite_domain.as_deref(),
            open_url: open_url.as_deref(),
            inject_localstorage: !no_localstorage,
            confirm: !no_confirm,
            json,
        }),
        Cmd::Replay {
            id,
            to,
            target,
            no_confirm,
        } => cmd_replay(&id, &to, &target, !no_confirm, json),
        Cmd::Share { id, out, password } => share::cmd_share(
            &Vault::open()?,
            &id,
            out.as_deref(),
            password.as_deref(),
            json,
        ),
        Cmd::Copy {
            site,
            from,
            to,
            keep_extra,
            no_backup,
            dry_run,
            no_confirm,
        } => copy::cmd_copy(copy::CopyArgs {
            site: &site,
            from: &from,
            to: &to,
            keep_extra,
            backup: !no_backup,
            confirm: !no_confirm,
            dry_run,
            json,
        }),
        Cmd::Cloud { action } => match action {
            CloudCmd::Setup {
                endpoint,
                uuid,
                password,
                crypto,
                no_browser_compat,
            } => cloud::cmd_setup(&endpoint, uuid, password, &crypto, !no_browser_compat, json),
            CloudCmd::Status => cloud::cmd_status(json),
            CloudCmd::Secret => cloud::cmd_show_secret(json),
            CloudCmd::Sync { force } => cloud::cmd_sync(false, force, json),
            CloudCmd::Pull => cloud::cmd_sync(true, false, json),
            CloudCmd::Domains => cloud::cmd_domains(json),
            CloudCmd::Import { site, id, label } => cloud::cmd_import(&site, &id, label, json),
            CloudCmd::Disconnect => cloud::cmd_disconnect(json),
        },
        Cmd::Export {
            ids,
            site,
            out,
            password,
        } => share::cmd_export(
            &Vault::open()?,
            &ids,
            site.as_deref(),
            out.as_deref(),
            password.as_deref(),
            json,
        ),
        Cmd::Redeem {
            bundle,
            password,
            id,
        } => {
            let mut vault = Vault::open()?;
            share::cmd_redeem(
                &mut vault,
                &bundle,
                password.as_deref(),
                id.as_deref(),
                json,
            )
        }
        Cmd::Run { id, site, all } => {
            let mut vault = Vault::open()?;
            runner::cmd_run(&mut vault, id.as_deref(), site.as_deref(), all, json)
        }
        Cmd::As {
            id,
            target,
            no_confirm,
            command,
        } => {
            let mut vault = Vault::open()?;
            act_as::cmd_as(&mut vault, &id, &target, &command, no_confirm, json)
        }
        Cmd::Fingerprint { id, all } => cmd_fingerprint(id.as_deref(), all, json),
        Cmd::Check { id } => cmd_check(&id, json),
        Cmd::Rm { id } => cmd_rm(&id, json),
        Cmd::Revoke { id } => cmd_rm(&id, json),
        Cmd::Wipe { yes } => cmd_wipe(yes, json),
        Cmd::Rename { id, new_id } => cmd_rename(&id, &new_id, json),
        Cmd::Edit {
            id,
            label,
            hint,
            note,
            tags,
        } => cmd_edit(&id, label, hint, note, tags, json),
        Cmd::Upgrade { .. } | Cmd::UpdateCheck => unreachable!("dispatched in main"),
    }
}

fn cmd_add(
    profile: &str,
    site: &str,
    id: Option<String>,
    label: Option<String>,
    hint: Option<String>,
    with_localstorage: bool,
    json: bool,
) -> Result<()> {
    let cookies = chrome_use::export_from_profile(profile, site)?;
    if cookies.is_empty() {
        return Err(anyhow!(
            "no cookies for {site} in profile \"{profile}\" — is it logged in there?"
        ));
    }
    let local_storage = if with_localstorage {
        let url = format!("https://{}", vault::landing_host(site));
        match chrome_use::capture_local_storage(&cookies, &url) {
            Ok(ls) if !ls.is_empty() => {
                // Human-only note (stdout) — suppressed in --json so the only
                // stdout line is the JSON object.
                if !json {
                    println!("captured {} localStorage item(s) from {url}", ls.len());
                }
                Some(ls)
            }
            Ok(_) => {
                eprintln!("note: no localStorage found at {url}");
                None
            }
            // Cookie capture already succeeded — don't fail the whole add.
            Err(e) => {
                eprintln!("warning: localStorage capture failed, storing cookies only: {e:#}");
                None
            }
        }
    } else {
        None
    };
    let ls_count = local_storage.as_ref().map(|m| m.len()).unwrap_or(0);
    let mut vault = Vault::open()?;
    let id = id.unwrap_or_else(|| next_id(&vault, site));
    store(
        &mut vault,
        id.clone(),
        site,
        cookies,
        local_storage,
        label,
        hint,
    )?;
    vault.save()?;
    refresh_fingerprint(&vault, &id);
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "id": id, "site": site, "localstorage_captured": ls_count,
            }))?
        );
    } else {
        println!("added \"{id}\" ({site}) from profile \"{profile}\"");
    }
    Ok(())
}

fn cmd_import(
    file: &str,
    site: &str,
    id: &str,
    label: Option<String>,
    hint: Option<String>,
    json: bool,
) -> Result<()> {
    let raw = std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?;
    let cookies = parse_cookie_file(&raw, site)?;
    if cookies.is_empty() {
        return Err(anyhow!("no cookies found in {file}"));
    }
    let mut vault = Vault::open()?;
    store(&mut vault, id.to_string(), site, cookies, None, label, hint)?;
    vault.save()?;
    refresh_fingerprint(&vault, id);
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({ "id": id, "site": site }))?
        );
    } else {
        println!("imported \"{id}\" ({site}) from {file}");
    }
    Ok(())
}

fn cmd_list(site_filter: Option<&str>, json_mode: bool) -> Result<()> {
    let vault = Vault::open()?;
    // 过滤器接受域名或完整 URL（https://dash.cloudflare.com/login → dash.cloudflare.com）。
    let needle = site_filter.map(normalize_site_filter);
    let accounts: Vec<&Account> = vault
        .accounts()
        .iter()
        .filter(|a| {
            needle
                .as_deref()
                .map(|s| account_matches(a, s))
                .unwrap_or(true)
        })
        .collect();

    if json_mode {
        let items: Vec<Value> = accounts
            .iter()
            .map(|a| {
                json!({
                    "id": a.id, "site": a.site, "label": a.label,
                    "account_hint": a.account_hint, "status": a.status.to_string(),
                    "cookies": a.cookies.len(), "last_used_at": a.last_used_at,
                    "note": a.note, "tags": a.tags,
                    "live_until": live_until(&a.cookies)
                        .and_then(|exp| chrono::DateTime::from_timestamp(exp, 0))
                        .map(|dt| dt.to_rfc3339()),
                    "updated_at": a.updated_at,
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&json!({ "accounts": items }))?);
        return Ok(());
    }

    if accounts.is_empty() {
        match needle.as_deref() {
            Some(s) => println!("no saved accounts matching site \"{s}\""),
            None => println!(
                "no accounts yet — add one with `cookie-use add --from-profile <p> --site <d>`"
            ),
        }
        return Ok(());
    }

    // 按网站(site 字符串)分组列出,组内按 id 排序 —— "根据网站列出已保存的 cookie"。
    let mut groups: BTreeMap<&str, Vec<&Account>> = BTreeMap::new();
    for a in &accounts {
        groups.entry(a.site.as_str()).or_default().push(a);
    }
    for (site, mut accts) in groups {
        accts.sort_by(|x, y| x.id.cmp(&y.id));
        println!("{site}");
        for a in accts {
            let hint = a
                .account_hint
                .as_deref()
                .or(a.label.as_deref())
                .unwrap_or("");
            println!(
                "  {:<24} {:<8} {:>4}  {}",
                truncate(&a.id, 24),
                a.status,
                a.cookies.len(),
                hint
            );
        }
    }
    Ok(())
}

/// Normalize a website filter to a bare lowercase host: strips scheme, path,
/// query and port, so `https://dash.cloudflare.com/login` → `dash.cloudflare.com`.
fn normalize_site_filter(s: &str) -> String {
    let s = s.trim();
    let s = s.split_once("://").map(|(_, rest)| rest).unwrap_or(s);
    let host = s.split(['/', '?', ':']).next().unwrap_or(s);
    host.trim().to_lowercase()
}

/// Does an account match a (normalized, lowercase) search term? Website-first
/// but forgiving: matches the base domain ↔ a subdomain in either direction,
/// loose substrings on a domain, and falls back to id / label / hint so a user
/// can also search by a memorable name (`leo`, `wind`). `cloudflare.com` matches
/// an account stored as `cloudflare.com,dash.cloudflare.com` and vice-versa.
/// Site filter shared by commands outside this file (export, cloud).
pub(crate) fn site_matches(a: &Account, filter: &str) -> bool {
    account_matches(a, &normalize_site_filter(filter))
}

fn account_matches(a: &Account, needle: &str) -> bool {
    if domain_matches(&a.site, needle) {
        return true;
    }
    let hay = |s: &str| s.to_lowercase().contains(needle);
    hay(&a.id)
        || a.label.as_deref().map(hay).unwrap_or(false)
        || a.account_hint.as_deref().map(hay).unwrap_or(false)
        || a.note.as_deref().map(hay).unwrap_or(false)
        || a.tags.iter().any(|t| hay(t))
}

/// Does a comma-joined `site` string match a normalized needle, domain-aware?
/// Matches base-domain ↔ subdomain in either direction, plus loose substrings.
fn domain_matches(site: &str, needle: &str) -> bool {
    site.to_lowercase().split(',').any(|dom| {
        let dom = dom.trim();
        !dom.is_empty()
            && (dom == needle
                || dom.ends_with(&format!(".{needle}")) // needle is a parent of a stored subdomain
                || needle.ends_with(&format!(".{dom}")) // needle is a subdomain of a stored domain
                || dom.contains(needle)) // loose substring (partial term)
    })
}

fn cmd_show(id: &str, json: bool) -> Result<()> {
    let vault = Vault::open()?;
    let a = vault
        .find(id)
        .ok_or_else(|| anyhow!("no account \"{id}\""))?;

    // Sorted unique cookie domains (names/domains only — never values).
    let mut domains: Vec<String> = a
        .cookies
        .iter()
        .filter_map(|c| c.get("domain").and_then(|d| d.as_str()).map(String::from))
        .collect();
    domains.sort();
    domains.dedup();
    let soonest = soonest_expiry(&a.cookies);

    if json {
        // Soonest cookie expiry as rfc3339, or null for session-only sessions.
        let expires = soonest
            .and_then(|exp| chrono::DateTime::from_timestamp(exp, 0))
            .map(|dt| dt.to_rfc3339());
        // localStorage KEYS only — never values (they can hold tokens).
        let ls_keys: Vec<&str> = a
            .local_storage
            .as_ref()
            .map(|ls| {
                let mut keys: Vec<&str> = ls.keys().map(String::as_str).collect();
                keys.sort_unstable();
                keys
            })
            .unwrap_or_default();
        println!(
            "{}",
            serde_json::to_string(&json!({
                "id": a.id,
                "site": a.site,
                "label": a.label,
                "hint": a.account_hint,
                "note": a.note,
                "tags": a.tags,
                "status": a.status.to_string(),
                "cookies": a.cookies.len(),
                "domains": domains,
                "expires": expires,
                "session_only": soonest.is_none(),
                "local_storage": ls_keys,
                "created_at": a.created_at,
                "updated_at": a.updated_at,
                "last_used_at": a.last_used_at,
            }))?
        );
        return Ok(());
    }

    println!("id:          {}", a.id);
    println!("site:        {}", a.site);
    if let Some(l) = &a.label {
        println!("label:       {l}");
    }
    if let Some(h) = &a.account_hint {
        println!("hint:        {h}");
    }
    println!("status:      {}", a.status);
    println!("cookies:     {}", a.cookies.len());
    if let Some(ls) = &a.local_storage {
        // Keys only — never values (they can hold tokens).
        let mut keys: Vec<&str> = ls.keys().map(String::as_str).collect();
        keys.sort_unstable();
        println!("localStorage: {} ({})", ls.len(), keys.join(", "));
    }
    println!("created:     {}", a.created_at.to_rfc3339());
    println!("updated:     {}", a.updated_at.to_rfc3339());
    if let Some(t) = a.last_used_at {
        println!("last used:   {}", t.to_rfc3339());
    }
    // Names + domains only — never values.
    let mut domains: Vec<String> = a
        .cookies
        .iter()
        .filter_map(|c| c.get("domain").and_then(|d| d.as_str()).map(String::from))
        .collect();
    domains.sort();
    domains.dedup();
    println!("domains:     {}", domains.join(", "));
    // Soonest cookie expiry, so the user can see how fresh the session is.
    if let Some(exp) = soonest_expiry(&a.cookies) {
        if let Some(dt) = chrono::DateTime::from_timestamp(exp, 0) {
            println!("expires:     {} (soonest cookie)", dt.to_rfc3339());
        }
    } else {
        println!("expires:     session cookies only (no fixed expiry)");
    }
    // Trust posture, stated plainly: nothing leaves the machine.
    println!("storage:     local-only, AES-256-GCM (~/.cookie-use/vault.enc)");
    Ok(())
}

/// When the liveness heuristic flips to expired: the latest positive cookie
/// expiry (unix seconds). `None` for session-only cookie sets.
fn live_until(cookies: &[Value]) -> Option<i64> {
    cookies
        .iter()
        .filter_map(|c| c.get("expires").and_then(|e| e.as_f64()))
        .filter(|e| *e > 0.0)
        .map(|e| e as i64)
        .max()
}

/// Earliest positive cookie expiry (unix seconds), if any cookie carries one.
fn soonest_expiry(cookies: &[Value]) -> Option<i64> {
    cookies
        .iter()
        .filter_map(|c| c.get("expires").and_then(|e| e.as_f64()))
        .filter(|e| *e > 0.0)
        .map(|e| e as i64)
        .min()
}

struct ApplyArgs<'a> {
    id: &'a str,
    target: &'a str,
    open: bool,
    clear_first: bool,
    rewrite_domain: Option<&'a str>,
    open_url: Option<&'a str>,
    inject_localstorage: bool,
    /// Require a biometric/TTY confirmation before injecting the session.
    confirm: bool,
    /// Emit a machine-readable JSON result instead of the human line.
    json: bool,
}

fn cmd_apply(args: ApplyArgs) -> Result<()> {
    let target = chrome_use::Target::parse(args.target)?;
    let mut vault = Vault::open()?;
    let account = vault
        .find(args.id)
        .ok_or_else(|| anyhow!("no account \"{}\"", args.id))?
        .clone();

    // The dangerous action is injecting a live session — gate it, not vault read.
    if args.confirm {
        confirm::require(&format!("apply session \"{}\"", args.id), false)?;
    }

    if args.clear_first {
        // Scoped sign-out: only cookies already known for this site (from any
        // stored account sharing its domains), never the whole browser.
        chrome_use::clear_site(&target, &site_cookies(&vault, &account))?;
    }

    // Resolve which URL (if any) to open after applying. An explicit --open-url
    // wins. When rewriting the domain we can't guess the dev host's scheme/port,
    // so we skip auto-opening the (now-wrong) production URL and say so.
    let open_url = match (args.open, args.open_url, args.rewrite_domain) {
        (_, Some(url), _) => Some(url.to_string()),
        (false, None, _) => None,
        (true, None, Some(_)) => {
            eprintln!(
                "note: --rewrite-domain set without --open-url; skipping auto-open \
                 (pass --open-url http://<host>:<port> to open the dev origin)"
            );
            None
        }
        (true, None, None) => Some(format!("https://{}", vault::landing_host(&account.site))),
    };

    let local_storage = if args.inject_localstorage {
        account.local_storage.as_ref()
    } else {
        None
    };
    // How many localStorage items actually get injected: only when a page is
    // opened (origin-scoped) and injection wasn't disabled.
    let ls_injected = match (open_url.as_deref(), local_storage) {
        (Some(_), Some(items)) => items.len(),
        _ => 0,
    };
    let opts = chrome_use::ApplyOpts {
        rewrite_domain: args.rewrite_domain,
        open_url: open_url.as_deref(),
        local_storage,
    };
    chrome_use::apply(&account.cookies, &target, &opts)?;

    if let Some(a) = vault.find_mut(args.id) {
        a.last_used_at = Some(Utc::now());
    }
    vault.save()?;
    // Keep the plaintext fingerprint cache warm on apply (cookies are unchanged,
    // but this covers accounts stored before the cache existed).
    refresh_fingerprint(&vault, args.id);
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "id": args.id,
                "session": target.session_name(),
                "opened_url": open_url,
                "cookies": account.cookies.len(),
                "localstorage": ls_injected,
                "ok": true,
            }))?
        );
    } else {
        println!(
            "applied \"{}\" ({})",
            args.id,
            if args.clear_first { "switched" } else { "use" }
        );
    }
    Ok(())
}

/// Export hash-only fingerprints. Single-id prints one account object (like
/// `show`); `--all` prints `{"accounts":[…]}` (like `list`).
fn cmd_fingerprint(id: Option<&str>, all: bool, json: bool) -> Result<()> {
    match (id, all) {
        (Some(id), false) => fingerprint_one(id, json),
        (None, true) => fingerprint_all(json),
        (Some(_), true) => Err(anyhow!("pass either an <id> or --all, not both")),
        (None, false) => Err(anyhow!(
            "provide an account id (cookie-use fingerprint <id>) or --all"
        )),
    }
}

fn fingerprint_one(id: &str, json: bool) -> Result<()> {
    let mut cache = fingerprint::Cache::open()?;
    let fp = if let Some(cached) = cache.get(id) {
        // Cache hit — no Keychain, no vault decrypt.
        cached.clone()
    } else {
        // First fingerprint for this account: decrypt the vault once, compute,
        // and store it so subsequent reads (and chrome-use) need no decrypt.
        let vault = Vault::open()?;
        let account = vault
            .find(id)
            .ok_or_else(|| anyhow!("no account \"{id}\""))?;
        let fp = fingerprint::compute(account);
        cache.insert(fp.clone());
        cache.save()?;
        fp
    };
    if json {
        println!("{}", serde_json::to_string(&fp)?);
    } else {
        print_fingerprint_human(&fp);
    }
    Ok(())
}

fn fingerprint_all(json: bool) -> Result<()> {
    let cache = fingerprint::Cache::open()?;
    // Enumerate the account universe once so we can flag accounts that have no
    // cached fingerprint yet — the vault has no plaintext account index, and a
    // read never triggers the Touch ID gate (that gate is injection-only), so
    // this is a single promptless decrypt, not N per-account prompts. Accounts
    // without a cache are SKIPPED (never computed here), matching the contract.
    // If the vault can't be opened at all, fall back to whatever is cached.
    let ids: Vec<String> = match Vault::open() {
        Ok(v) => v.accounts().iter().map(|a| a.id.clone()).collect(),
        Err(_) => cache.ids(),
    };

    let mut present: Vec<&fingerprint::AccountFingerprint> = Vec::new();
    for id in &ids {
        match cache.get(id) {
            Some(fp) => present.push(fp),
            None => eprintln!("{id}: no fingerprint yet — run: cookie-use fingerprint {id}"),
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({ "accounts": present }))?
        );
    } else if present.is_empty() {
        println!("no fingerprints cached yet — run: cookie-use fingerprint <id>");
    } else {
        for (i, fp) in present.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_fingerprint_human(fp);
        }
    }
    Ok(())
}

/// Human summary: counts only — never a value, never a hash.
fn print_fingerprint_human(fp: &fingerprint::AccountFingerprint) {
    let http_only = fp.cookies.iter().filter(|c| c.http_only).count();
    let secure = fp.cookies.iter().filter(|c| c.secure).count();
    println!("id:       {}", fp.id);
    println!("site:     {}", fp.site);
    println!("cookies:  {} fingerprinted", fp.cookies.len());
    println!("httpOnly: {http_only}");
    println!("secure:   {secure}");
    println!("computed: {}", fp.computed_at.to_rfc3339());
}

fn cmd_check(id: &str, json: bool) -> Result<()> {
    let mut vault = Vault::open()?;
    let status = {
        let a = vault
            .find(id)
            .ok_or_else(|| anyhow!("no account \"{id}\""))?;
        liveness(&a.cookies)
    };
    if let Some(a) = vault.find_mut(id) {
        a.status = status;
    }
    vault.save()?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({ "id": id, "status": status.to_string() }))?
        );
    } else {
        println!("{id}: {status}");
    }
    Ok(())
}

fn cmd_rm(id: &str, json: bool) -> Result<()> {
    let mut vault = Vault::open()?;
    vault.remove(id)?;
    vault.save()?;
    fingerprint::forget(id);
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({ "id": id, "removed": true }))?
        );
    } else {
        println!("removed \"{id}\"");
    }
    Ok(())
}

fn cmd_rename(id: &str, new_id: &str, json: bool) -> Result<()> {
    let mut vault = Vault::open()?;
    if vault.find(new_id).is_some() {
        return Err(anyhow!("\"{new_id}\" already exists"));
    }
    let a = vault
        .find_mut(id)
        .ok_or_else(|| anyhow!("no account \"{id}\""))?;
    a.id = new_id.to_string();
    a.updated_at = Utc::now();
    vault.mark_deleted(id);
    vault.save()?;
    // The cached fingerprint is keyed by (and stamped with) the old id; drop it
    // so it recomputes lazily under the new id on the next `fingerprint`.
    fingerprint::forget(id);
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({ "id": id, "new_id": new_id }))?
        );
    } else {
        println!("renamed \"{id}\" -> \"{new_id}\"");
    }
    Ok(())
}

fn cmd_edit(
    id: &str,
    label: Option<String>,
    hint: Option<String>,
    note: Option<String>,
    tags: Option<String>,
    json: bool,
) -> Result<()> {
    if label.is_none() && hint.is_none() && note.is_none() && tags.is_none() {
        return Err(anyhow!(
            "nothing to edit — pass --label, --hint, --note and/or --tags"
        ));
    }
    let mut vault = Vault::open()?;
    let a = vault
        .find_mut(id)
        .ok_or_else(|| anyhow!("no account \"{id}\""))?;
    // "" clears a field; anything else replaces it.
    let field = |v: String| {
        let v = v.trim().to_string();
        (!v.is_empty()).then_some(v)
    };
    if let Some(v) = label {
        a.label = field(v);
    }
    if let Some(v) = hint {
        a.account_hint = field(v);
    }
    if let Some(v) = note {
        a.note = field(v);
    }
    if let Some(v) = tags {
        a.tags = parse_tags(&v);
    }
    a.updated_at = Utc::now();
    let out = json!({
        "id": a.id, "label": a.label, "hint": a.account_hint,
        "note": a.note, "tags": a.tags,
    });
    vault.save()?;
    if json {
        println!("{}", serde_json::to_string(&out)?);
    } else {
        println!("updated \"{id}\"");
    }
    Ok(())
}

/// Comma-separated tags → trimmed, lowercase, de-duplicated, order kept.
fn parse_tags(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw.split(',').map(|t| t.trim().to_lowercase()) {
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// QA cross-origin sugar: replay a captured session onto a local dev origin.
/// Equivalent to `use --rewrite-domain <host> --open-url http://<host:port>`,
/// so a prod login can be exercised against localhost in one obvious command.
fn cmd_replay(id: &str, to: &str, target: &str, confirm: bool, json: bool) -> Result<()> {
    // `to` may be "localhost:8001", "127.0.0.1:3000", or a full http(s) URL.
    let stripped = to
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let host = stripped.split(':').next().unwrap_or(stripped).to_string();
    if host.is_empty() {
        return Err(anyhow!(
            "invalid --to \"{to}\" (expected host[:port] or URL)"
        ));
    }
    let open_url = if to.starts_with("http://") || to.starts_with("https://") {
        to.to_string()
    } else {
        format!("http://{stripped}")
    };
    cmd_apply(ApplyArgs {
        id,
        target,
        open: true,
        clear_first: false,
        rewrite_domain: Some(&host),
        open_url: Some(&open_url),
        inject_localstorage: true,
        confirm,
        json,
    })
}

/// Delete the entire vault. Destructive; confirms unless `--yes`.
fn cmd_wipe(yes: bool, json: bool) -> Result<()> {
    let vault = Vault::open()?;
    let n = vault.accounts().len();
    if !yes {
        confirm::confirm_tty(&format!("delete the ENTIRE vault ({n} account(s))"))?;
    }
    vault.delete_file()?;
    // Don't leave hashes of wiped accounts behind in the plaintext sidecar.
    let _ = fingerprint::Cache::delete_file();
    if json {
        println!("{}", serde_json::to_string(&json!({ "removed": n }))?);
    } else {
        println!("wiped vault ({n} account(s) removed)");
    }
    Ok(())
}

// --- helpers ---

/// Refresh an account's plaintext fingerprint cache, best-effort: a cache-write
/// failure must never fail the underlying add/import/apply, so we only warn.
fn refresh_fingerprint(vault: &Vault, id: &str) {
    if let Some(a) = vault.find(id) {
        if let Err(e) = fingerprint::refresh_cache(a) {
            eprintln!("warning: could not update fingerprint cache for \"{id}\": {e:#}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn store(
    vault: &mut Vault,
    id: String,
    site: &str,
    cookies: Vec<Value>,
    local_storage: Option<serde_json::Map<String, Value>>,
    label: Option<String>,
    hint: Option<String>,
) -> Result<()> {
    let now = Utc::now();
    // Re-capturing an existing id refreshes its session but keeps what the user
    // wrote about it (label / hint / note / tags) unless new values are given.
    let prev = vault.find(&id).cloned();
    let created_at = prev.as_ref().map(|a| a.created_at).unwrap_or(now);
    let status = liveness(&cookies);
    vault.upsert(Account {
        id,
        site: site.to_string(),
        label: label.or_else(|| prev.as_ref().and_then(|a| a.label.clone())),
        account_hint: hint.or_else(|| prev.as_ref().and_then(|a| a.account_hint.clone())),
        note: prev.as_ref().and_then(|a| a.note.clone()),
        tags: prev.as_ref().map(|a| a.tags.clone()).unwrap_or_default(),
        cookies,
        local_storage,
        created_at,
        updated_at: now,
        last_used_at: prev.as_ref().and_then(|a| a.last_used_at),
        status,
        proxy: None,
        fingerprint: None,
    });
    Ok(())
}

/// Generic liveness from cookie expiry — no site-specific logic. If every
/// cookie that carries an expiry is already past, the session is expired;
/// otherwise we treat it as live (best effort; real per-site probes come later).
fn liveness(cookies: &[Value]) -> Status {
    if cookies.is_empty() {
        return Status::Expired;
    }
    let now = Utc::now().timestamp() as f64;
    let mut saw_expiry = false;
    let mut any_future = false;
    for c in cookies {
        if let Some(exp) = c.get("expires").and_then(|e| e.as_f64()) {
            if exp > 0.0 {
                saw_expiry = true;
                if exp > now {
                    any_future = true;
                }
            }
        } else {
            // Session cookie (no expiry) — can't be judged stale by time.
            any_future = true;
        }
    }
    if saw_expiry && !any_future {
        Status::Expired
    } else {
        Status::Live
    }
}

/// Parse an imported cookie file: a JSON array of cookie objects, or a bare
/// `name=value; ...` Cookie header (domain taken from --site).
fn parse_cookie_file(raw: &str, site: &str) -> Result<Vec<Value>> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        let arr: Vec<Value> = serde_json::from_str(trimmed).context("parsing JSON cookie array")?;
        return Ok(arr);
    }
    let domain = format!(".{}", primary_domain(site));
    let mut out = Vec::new();
    for piece in trimmed.split(';') {
        let piece = piece.trim();
        if let Some((name, value)) = piece.split_once('=') {
            let name = name.trim();
            if !name.is_empty() {
                out.push(json!({
                    "name": name, "value": value.trim(),
                    "domain": domain, "path": "/"
                }));
            }
        }
    }
    Ok(out)
}

/// First domain in a comma list, without a leading dot.
/// Every stored cookie that belongs to `account`'s site: its own, plus those of
/// any other account whose domains overlap (base domain ↔ subdomain). This is
/// the set a clean switch expires, so the previous account's login is dropped
/// without touching unrelated sites.
fn site_cookies(vault: &Vault, account: &Account) -> Vec<Value> {
    let hosts = site_hosts(&account.site);
    vault
        .accounts()
        .iter()
        .filter(|a| {
            a.id == account.id
                || site_hosts(&a.site)
                    .iter()
                    .any(|h| hosts.iter().any(|k| hosts_overlap(h, k)))
        })
        .flat_map(|a| a.cookies.iter().cloned())
        .collect()
}

fn site_hosts(site: &str) -> Vec<String> {
    site.split(',')
        .map(|d| d.trim().trim_start_matches('.').to_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

fn hosts_overlap(a: &str, b: &str) -> bool {
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

fn primary_domain(site: &str) -> String {
    site.split(',')
        .next()
        .unwrap_or(site)
        .trim()
        .trim_start_matches('.')
        .to_string()
}

/// Slug for default ids: leading label of the primary domain ("chatgpt.com" -> "chatgpt").
pub(crate) fn site_base(site: &str) -> String {
    let d = primary_domain(site);
    d.split('.').next().unwrap_or(&d).to_string()
}

fn next_id(vault: &Vault, site: &str) -> String {
    let base = site_base(site);
    let mut n = 1;
    loop {
        let candidate = format!("{base}/{n:02}");
        if vault.find(&candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::{domain_matches, normalize_site_filter};

    #[test]
    fn normalize_strips_scheme_path_query_port_and_lowercases() {
        assert_eq!(
            normalize_site_filter("https://dash.cloudflare.com/"),
            "dash.cloudflare.com"
        );
        assert_eq!(
            normalize_site_filter("https://dash.cloudflare.com/login?next=1"),
            "dash.cloudflare.com"
        );
        assert_eq!(
            normalize_site_filter("dash.cloudflare.com"),
            "dash.cloudflare.com"
        );
        assert_eq!(
            normalize_site_filter("http://localhost:8001/app"),
            "localhost"
        );
        assert_eq!(
            normalize_site_filter("  HTTPS://ChatGPT.com  "),
            "chatgpt.com"
        );
    }

    #[test]
    fn domain_matching_is_forgiving() {
        let site = "cloudflare.com,dash.cloudflare.com";
        // user just types the base domain
        assert!(domain_matches(site, "cloudflare.com"));
        // exact subdomain
        assert!(domain_matches(site, "dash.cloudflare.com"));
        // partial term
        assert!(domain_matches(site, "cloudflare"));
        // base domain finds an account stored only as a subdomain
        assert!(domain_matches("dash.cloudflare.com", "cloudflare.com"));
        // subdomain query finds an account stored as the base domain
        assert!(domain_matches("cloudflare.com", "dash.cloudflare.com"));
        // non-match
        assert!(!domain_matches(site, "chatgpt.com"));
        assert!(!domain_matches("chatgpt.com,openai.com", "cloudflare.com"));
    }
}
