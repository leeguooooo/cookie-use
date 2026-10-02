//! `verify` — does a saved session still sign in?
//!
//! Cookie expiry says nothing about a session the site has revoked (logged
//! out, password changed, bound to the original browser). So we replay it: in
//! one throwaway off-screen browser, open the site anonymously, then again
//! with the account's cookies (and localStorage), and compare. Landing on a
//! sign-in page means the login doesn't work; landing somewhere an anonymous
//! visit doesn't means it does. Site-agnostic — no per-site rules.

use crate::chrome_use::{self, PageState, Target};
use crate::vault::{Account, Status, Vault, Verification};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Does this page look like a sign-in page?
fn login_like(p: &PageState) -> bool {
    if p.has_password {
        return true;
    }
    let u = p.url.to_lowercase();
    let path = u.split(['?', '#']).next().unwrap_or(&u);
    [
        "login",
        "signin",
        "sign-in",
        "sign_in",
        "/auth",
        "sso",
        "oauth",
        "authorize",
        "accounts.google.com",
        "/session/new",
    ]
    .iter()
    .any(|k| path.contains(k))
}

fn path_of(url: &str) -> &str {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/')
}

/// Classify one replay against the anonymous visit to the same URL.
pub fn judge(anon: Option<&PageState>, with: &PageState) -> (&'static str, String) {
    if login_like(with) {
        return (
            "invalid",
            format!("lands on a sign-in page ({})", path_of(&with.url)),
        );
    }
    let Some(anon) = anon else {
        return ("valid", "doesn't land on a sign-in page".into());
    };
    if login_like(anon) {
        return (
            "valid",
            "signed in (an anonymous visit gets a sign-in page)".into(),
        );
    }
    if path_of(&anon.url) != path_of(&with.url) {
        return (
            "valid",
            format!("lands on {} instead of the public page", path_of(&with.url)),
        );
    }
    if anon.title != with.title {
        return ("valid", "the page differs from an anonymous visit".into());
    }
    ("unknown", "the page looks the same signed in or out".into())
}

struct Job {
    id: String,
    url: String,
    cookies: Vec<Value>,
    local_storage: Option<Map<String, Value>>,
    session_at: DateTime<Utc>,
}

fn jobs_from(vault: &Vault, ids: &[String], site: Option<&str>) -> Result<Vec<Job>> {
    let picked: Vec<&Account> = if ids.is_empty() {
        vault
            .accounts()
            .iter()
            .filter(|a| !a.tags.iter().any(|t| t == "backup"))
            .filter(|a| site.map(|s| crate::site_matches(a, s)).unwrap_or(true))
            .collect()
    } else {
        ids.iter()
            .map(|id| {
                vault
                    .find(id)
                    .ok_or_else(|| anyhow::anyhow!("no account \"{id}\""))
            })
            .collect::<Result<_>>()?
    };
    Ok(picked
        .into_iter()
        .map(|a| Job {
            id: a.id.clone(),
            url: format!("https://{}", crate::vault::landing_host(&a.site)),
            cookies: a.cookies.clone(),
            local_storage: a.local_storage.clone(),
            session_at: a.session_ts(),
        })
        .collect())
}

fn check_one(job: &Job, anon_cache: &mut HashMap<String, Option<PageState>>) -> Verification {
    // Anonymous baseline for this URL (cached across accounts on the same site).
    let anon = anon_cache
        .entry(job.url.clone())
        .or_insert_with(|| {
            let _ = chrome_use::clear_verify_browser();
            chrome_use::visit(&job.url).ok()
        })
        .clone();

    let (result, reason, url) = (|| {
        chrome_use::clear_verify_browser()?;
        let target = Target::Session(chrome_use::VERIFY_SESSION.to_string());
        let opts = chrome_use::ApplyOpts {
            rewrite_domain: None,
            open_url: Some(&job.url),
            local_storage: job.local_storage.as_ref(),
        };
        chrome_use::apply(&job.cookies, &target, &opts)?;
        let with = chrome_use::page_state()?;
        let (r, reason) = judge(anon.as_ref(), &with);
        Ok::<_, anyhow::Error>((r.to_string(), reason, with.url))
    })()
    .unwrap_or_else(|e| {
        (
            "unknown".into(),
            format!("couldn't open the site: {e:#}"),
            String::new(),
        )
    });

    Verification {
        at: Utc::now(),
        result,
        reason,
        url,
        session_at: job.session_at,
    }
}

pub fn cmd_verify(ids: &[String], site: Option<&str>, json_mode: bool) -> Result<()> {
    // Read the jobs, then release the vault: the browser part is slow, and the
    // app / agents must be able to use the vault meanwhile. Results are written
    // back per account, skipping any whose session changed while we checked.
    let jobs = jobs_from(&Vault::open()?, ids, site)?;
    if jobs.is_empty() {
        anyhow::bail!(
            "nothing to verify{}",
            site.map(|s| format!(" for \"{s}\"")).unwrap_or_default()
        );
    }

    chrome_use::launch_offscreen()?;
    let mut anon_cache: HashMap<String, Option<PageState>> = HashMap::new();
    let mut rows: Vec<(String, Verification)> = Vec::new();
    for job in &jobs {
        if !json_mode {
            eprint!("checking {} … ", job.id);
        }
        let v = check_one(job, &mut anon_cache);
        if !json_mode {
            eprintln!("{}", v.result);
        }
        rows.push((job.id.clone(), v));
    }
    chrome_use::close_verify_browser();

    // Write results back, each only if that account's session is unchanged.
    let mut vault = Vault::open()?;
    for (id, v) in &rows {
        if let Some(a) = vault.find_mut(id) {
            if a.session_ts() == v.session_at {
                a.status = match v.result.as_str() {
                    "valid" => Status::Live,
                    "invalid" => Status::Expired,
                    _ => a.status,
                };
                a.verified = Some(v.clone());
            }
        }
    }
    vault.save()?;

    let invalid = rows.iter().filter(|(_, v)| v.result == "invalid").count();
    let valid = rows.iter().filter(|(_, v)| v.result == "valid").count();
    if json_mode {
        let items: Vec<Value> = rows
            .iter()
            .map(
                |(id, v)| json!({ "id": id, "result": v.result, "reason": v.reason, "url": v.url }),
            )
            .collect();
        println!(
            "{}",
            serde_json::to_string(
                &json!({ "checked": rows.len(), "valid": valid, "invalid": invalid, "results": items })
            )?
        );
    } else {
        for (id, v) in &rows {
            if v.result != "valid" {
                println!(
                    "  {:<28} {:<8} {}",
                    crate::truncate(id, 28),
                    v.result,
                    v.reason
                );
            }
        }
        println!(
            "checked {}: {valid} valid, {invalid} need a fresh login, {} unknown",
            rows.len(),
            rows.len() - valid - invalid
        );
    }
    Ok(())
}
