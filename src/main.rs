//! cookie-use — agent-friendly multi-account session manager.
//!
//! Sits on top of `chrome-use`: owns the account model, an encrypted vault, and
//! orchestration; delegates all browser/cookie I/O to chrome-use. Site-agnostic.

mod chrome_use;
mod crypto;
mod keychain;
mod vault;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
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
        #[arg(long)]
        site: Option<String>,
        #[arg(long)]
        json: bool,
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
    },
    /// Update an account's liveness from its cookie expiry (generic heuristic).
    Check { id: String },
    /// Remove an account.
    Rm { id: String },
    /// Rename an account id.
    Rename { id: String, new_id: String },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Add {
            from_profile,
            site,
            id,
            label,
            hint,
            with_localstorage,
        } => cmd_add(&from_profile, &site, id, label, hint, with_localstorage),
        Cmd::Import {
            file,
            site,
            id,
            label,
            hint,
        } => cmd_import(&file, &site, &id, label, hint),
        Cmd::List { site, json } => cmd_list(site.as_deref(), json),
        Cmd::Show { id } => cmd_show(&id),
        Cmd::Use {
            id,
            target,
            no_open,
            rewrite_domain,
            open_url,
            no_localstorage,
        } => cmd_apply(ApplyArgs {
            id: &id,
            target: &target,
            open: !no_open,
            clear_first: false,
            rewrite_domain: rewrite_domain.as_deref(),
            open_url: open_url.as_deref(),
            inject_localstorage: !no_localstorage,
        }),
        Cmd::Switch {
            id,
            target,
            no_open,
            rewrite_domain,
            open_url,
            no_localstorage,
        } => cmd_apply(ApplyArgs {
            id: &id,
            target: &target,
            open: !no_open,
            clear_first: true,
            rewrite_domain: rewrite_domain.as_deref(),
            open_url: open_url.as_deref(),
            inject_localstorage: !no_localstorage,
        }),
        Cmd::Check { id } => cmd_check(&id),
        Cmd::Rm { id } => cmd_rm(&id),
        Cmd::Rename { id, new_id } => cmd_rename(&id, &new_id),
    }
}

fn cmd_add(
    profile: &str,
    site: &str,
    id: Option<String>,
    label: Option<String>,
    hint: Option<String>,
    with_localstorage: bool,
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
                println!("captured {} localStorage item(s) from {url}", ls.len());
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
    println!("added \"{id}\" ({site}) from profile \"{profile}\"");
    Ok(())
}

fn cmd_import(
    file: &str,
    site: &str,
    id: &str,
    label: Option<String>,
    hint: Option<String>,
) -> Result<()> {
    let raw = std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?;
    let cookies = parse_cookie_file(&raw, site)?;
    if cookies.is_empty() {
        return Err(anyhow!("no cookies found in {file}"));
    }
    let mut vault = Vault::open()?;
    store(&mut vault, id.to_string(), site, cookies, None, label, hint)?;
    vault.save()?;
    println!("imported \"{id}\" ({site}) from {file}");
    Ok(())
}

fn cmd_list(site_filter: Option<&str>, json_mode: bool) -> Result<()> {
    let vault = Vault::open()?;
    let accounts: Vec<&Account> = vault
        .accounts()
        .iter()
        .filter(|a| site_filter.map(|s| a.site.contains(s)).unwrap_or(true))
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
        println!("no accounts yet — add one with `cookie-use add --from-profile <p> --site <d>`");
        return Ok(());
    }
    println!(
        "{:<22} {:<24} {:<8} {:<6} HINT/LABEL",
        "ID", "SITE", "STATUS", "COOKIES"
    );
    for a in accounts {
        let hint = a
            .account_hint
            .as_deref()
            .or(a.label.as_deref())
            .unwrap_or("");
        println!(
            "{:<22} {:<24} {:<8} {:<6} {}",
            truncate(&a.id, 22),
            truncate(&a.site, 24),
            a.status,
            a.cookies.len(),
            hint
        );
    }
    Ok(())
}

fn cmd_show(id: &str) -> Result<()> {
    let vault = Vault::open()?;
    let a = vault
        .find(id)
        .ok_or_else(|| anyhow!("no account \"{id}\""))?;
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
    Ok(())
}

struct ApplyArgs<'a> {
    id: &'a str,
    target: &'a str,
    open: bool,
    clear_first: bool,
    rewrite_domain: Option<&'a str>,
    open_url: Option<&'a str>,
    inject_localstorage: bool,
}

fn cmd_apply(args: ApplyArgs) -> Result<()> {
    let target = chrome_use::Target::parse(args.target)?;
    let mut vault = Vault::open()?;
    let account = vault
        .find(args.id)
        .ok_or_else(|| anyhow!("no account \"{}\"", args.id))?
        .clone();

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
    println!(
        "applied \"{}\" ({})",
        args.id,
        if args.clear_first { "switched" } else { "use" }
    );
    Ok(())
}

fn cmd_check(id: &str) -> Result<()> {
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
    println!("{id}: {status}");
    Ok(())
}

fn cmd_rm(id: &str) -> Result<()> {
    let mut vault = Vault::open()?;
    vault.remove(id)?;
    vault.save()?;
    println!("removed \"{id}\"");
    Ok(())
}

fn cmd_rename(id: &str, new_id: &str) -> Result<()> {
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
    println!("renamed \"{id}\" -> \"{new_id}\"");
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
