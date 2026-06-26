//! cookie-use — agent-friendly multi-account session manager.
//!
//! Sits on top of `chrome-use`: owns the account model, an encrypted vault, and
//! orchestration; delegates all browser/cookie I/O to chrome-use. Site-agnostic.

mod act_as;
mod chrome_use;
mod confirm;
mod crypto;
mod keychain;
mod runner;
mod share;
mod vault;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use std::collections::BTreeMap;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
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
    /// Import a session bundle produced by `share` into the vault.
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
    /// Update an account's liveness from its cookie expiry (generic heuristic).
    Check { id: String },
    /// Remove an account.
    Rm { id: String },
    /// Rename an account id.
    Rename { id: String, new_id: String },
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    if let Err(e) = run(cli) {
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
        } => cmd_add(&from_profile, &site, id, label, hint, with_localstorage, json),
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
        Cmd::Share { id, out, password } => {
            share::cmd_share(&Vault::open()?, &id, out.as_deref(), password.as_deref(), json)
        }
        Cmd::Redeem {
            bundle,
            password,
            id,
        } => {
            let mut vault = Vault::open()?;
            share::cmd_redeem(&mut vault, &bundle, password.as_deref(), id.as_deref(), json)
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
        Cmd::Check { id } => cmd_check(&id, json),
        Cmd::Rm { id } => cmd_rm(&id, json),
        Cmd::Revoke { id } => cmd_rm(&id, json),
        Cmd::Wipe { yes } => cmd_wipe(yes, json),
        Cmd::Rename { id, new_id } => cmd_rename(&id, &new_id, json),
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
        let url = format!("https://{}", primary_domain(site));
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
        .filter(|a| needle.as_deref().map(|s| account_matches(a, s)).unwrap_or(true))
        .collect();

    if json_mode {
        let items: Vec<Value> = accounts
            .iter()
            .map(|a| {
                json!({
                    "id": a.id, "site": a.site, "label": a.label,
                    "account_hint": a.account_hint, "status": a.status.to_string(),
                    "cookies": a.cookies.len(), "last_used_at": a.last_used_at,
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
fn account_matches(a: &Account, needle: &str) -> bool {
    if domain_matches(&a.site, needle) {
        return true;
    }
    let hay = |s: &str| s.to_lowercase().contains(needle);
    hay(&a.id)
        || a.label.as_deref().map(hay).unwrap_or(false)
        || a.account_hint.as_deref().map(hay).unwrap_or(false)
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
        chrome_use::clear(&target)?;
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
        (true, None, None) => Some(format!("https://{}", primary_domain(&account.site))),
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
    vault.save()?;
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
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({ "removed": n }))?
        );
    } else {
        println!("wiped vault ({n} account(s) removed)");
    }
    Ok(())
}

// --- helpers ---

#[allow(clippy::too_many_arguments)]
fn store(
    vault: &mut Vault,
    id: String,
    site: &str,
    cookies: Vec<Value>,
    local_storage: Option<serde_json::Map<String, Value>>,
    label: Option<String>,
    hint: Option<String>,
) -> Result<()> {
    let now = Utc::now();
    let created_at = vault.find(&id).map(|a| a.created_at).unwrap_or(now);
    let status = liveness(&cookies);
    vault.upsert(Account {
        id,
        site: site.to_string(),
        label,
        account_hint: hint,
        cookies,
        local_storage,
        created_at,
        updated_at: now,
        last_used_at: None,
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
fn primary_domain(site: &str) -> String {
    site.split(',')
        .next()
        .unwrap_or(site)
        .trim()
        .trim_start_matches('.')
        .to_string()
}

/// Slug for default ids: leading label of the primary domain ("chatgpt.com" -> "chatgpt").
fn site_base(site: &str) -> String {
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
        assert_eq!(normalize_site_filter("https://dash.cloudflare.com/"), "dash.cloudflare.com");
        assert_eq!(
            normalize_site_filter("https://dash.cloudflare.com/login?next=1"),
            "dash.cloudflare.com"
        );
        assert_eq!(normalize_site_filter("dash.cloudflare.com"), "dash.cloudflare.com");
        assert_eq!(normalize_site_filter("http://localhost:8001/app"), "localhost");
        assert_eq!(normalize_site_filter("  HTTPS://ChatGPT.com  "), "chatgpt.com");
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
