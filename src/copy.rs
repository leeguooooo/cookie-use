//! `copy` — overwrite one Chrome profile's login for a site with another's.
//!
//! Only the site's cookies move. In the destination, cookies for that site
//! that the source doesn't have are expired (so the old account is really
//! gone), every other site is left exactly as it was, and the destination's
//! previous login is saved to the vault first so the copy can be undone.

use crate::chrome_use::{self, LocalProfile, Target};
use crate::vault::Vault;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashSet;

pub struct CopyArgs<'a> {
    pub site: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    /// Leave destination cookies the source doesn't have (merge, don't replace).
    pub keep_extra: bool,
    pub backup: bool,
    pub confirm: bool,
    pub dry_run: bool,
    pub json: bool,
}

pub fn cmd_copy(args: CopyArgs) -> Result<()> {
    let from = chrome_use::resolve_profile(args.from)?;
    let to = chrome_use::resolve_profile(args.to)?;
    if from.dir == to.dir {
        return Err(anyhow!(
            "--from and --to are the same profile ({})",
            from.dir
        ));
    }
    // Writing into a chosen profile goes through chrome-use's extension relay,
    // which addresses a profile by its signed-in account.
    let email = to.email.clone().ok_or_else(|| {
        anyhow!(
            "profile \"{}\" ({}) isn't signed in to a Google account, so chrome-use can't \
             address it — sign it in, or open it and use `cookie-use use … --target session:<name>`",
            to.name,
            to.dir
        )
    })?;

    let src = chrome_use::export_from_profile(&from.dir, args.site)?;
    if src.is_empty() {
        return Err(anyhow!(
            "\"{}\" ({}) has no cookies for {} — is it logged in there?",
            from.name,
            from.dir,
            args.site
        ));
    }
    let dst = chrome_use::export_from_profile(&to.dir, args.site)?;
    let src_keys: HashSet<_> = src.iter().filter_map(cookie_key).collect();
    let stale: Vec<Value> = if args.keep_extra {
        Vec::new()
    } else {
        dst.iter()
            .filter(|c| {
                cookie_key(c)
                    .map(|k| !src_keys.contains(&k))
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    };

    if args.dry_run {
        return report(&args, &from, &to, src.len(), stale.len(), None, true);
    }

    if args.confirm {
        crate::confirm::require(
            &format!(
                "overwrite {}'s {} login with {}'s",
                to.name, args.site, from.name
            ),
            false,
        )?;
    }

    // Snapshot the destination's current login before touching it.
    let backup_id = if args.backup && !dst.is_empty() {
        let mut vault = Vault::open()?;
        let id = format!(
            "backup/{}/{}-{}",
            crate::site_base(args.site),
            slug(&to.name),
            Utc::now().format("%Y%m%d-%H%M%S")
        );
        crate::store(
            &mut vault,
            id.clone(),
            args.site,
            dst.clone(),
            None,
            Some(format!("{} before copy from {}", to.name, from.name)),
            to.email.clone(),
        )?;
        if let Some(a) = vault.find_mut(&id) {
            a.tags = vec!["backup".into()];
        }
        vault.save()?;
        Some(id)
    } else {
        None
    };

    let target = Target::Browser(email);
    let hint = || {
        format!(
            "is Chrome open in profile \"{}\" with the chrome-use extension connected? \
             (`chrome-use browsers` lists the connected ones)",
            to.name
        )
    };
    chrome_use::clear_site(&target, &stale).with_context(hint)?;
    chrome_use::apply(&src, &target, &chrome_use::ApplyOpts::default()).with_context(hint)?;

    report(&args, &from, &to, src.len(), stale.len(), backup_id, false)
}

fn report(
    args: &CopyArgs,
    from: &LocalProfile,
    to: &LocalProfile,
    copied: usize,
    removed: usize,
    backup_id: Option<String>,
    dry_run: bool,
) -> Result<()> {
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "site": args.site, "from": from.dir, "to": to.dir,
                "copied": copied, "removed": removed,
                "backup_id": backup_id, "dry_run": dry_run,
            }))?
        );
    } else {
        let verb = if dry_run { "would copy" } else { "copied" };
        println!(
            "{verb} {copied} {} cookie(s) from {} ({}) to {} ({}); {} stale cookie(s) for that site {}; other sites untouched",
            args.site,
            from.name,
            from.dir,
            to.name,
            to.dir,
            removed,
            if dry_run { "would be removed" } else { "removed" }
        );
        if let Some(id) = backup_id {
            println!("previous login saved as \"{id}\" — undo with: cookie-use use {id} --target browser:{}", to.email.as_deref().unwrap_or(""));
        }
    }
    Ok(())
}

fn cookie_key(c: &Value) -> Option<(String, String, String)> {
    Some((
        c.get("name")?.as_str()?.to_string(),
        // Exact domain: ".x.com" (domain cookie) and "x.com" (host-only) are
        // different cookies in Chrome, so one doesn't overwrite the other.
        c.get("domain")?.as_str()?.to_string(),
        c.get("path")
            .and_then(Value::as_str)
            .unwrap_or("/")
            .to_string(),
    ))
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_key_defaults_path_and_keeps_host_only_distinct() {
        let a = json!({"name":"sid","domain":".x.com","value":"1"});
        let b = json!({"name":"sid","domain":".x.com","path":"/","value":"2"});
        let host_only = json!({"name":"sid","domain":"x.com","path":"/"});
        assert_eq!(cookie_key(&a), cookie_key(&b));
        assert_ne!(cookie_key(&a), cookie_key(&host_only));
    }
}
