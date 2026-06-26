//! `run` command — open multiple stored accounts in isolated browser windows
//! simultaneously so the same site can be driven under several identities
//! side by side.

use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::vault::{Account, Vault};

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

/// Open one or more stored accounts in isolated browser windows simultaneously.
///
/// Account selection:
/// - `all == true`  → every account, optionally narrowed by `site` substring.
/// - `id == Some`   → exactly that account (error if missing).
/// - `site == Some` → every account whose `site` contains the substring.
/// - otherwise      → error.
///
/// Each selected account gets its own isolated browser session. They all stay
/// open after this function returns, achieving the "side by side" goal.
pub fn cmd_run(
    vault: &mut Vault,
    id: Option<&str>,
    site: Option<&str>,
    all: bool,
    json: bool,
) -> anyhow::Result<()> {
    // Clone accounts we need to open so we can mutate the vault afterwards.
    let selected: Vec<Account> = select(vault.accounts(), id, site, all)?
        .into_iter()
        .cloned()
        .collect();

    let mut opened = 0usize;
    let mut errors = 0usize;
    // Per-account outcome for --json (failures only hit stderr in text mode).
    let mut results: Vec<serde_json::Value> = Vec::new();

    for account in &selected {
        let session = iso_session(&account.id);
        let url = open_url_for(&account.site);

        match crate::chrome_use::apply_isolated_named(
            &account.cookies,
            &session,
            &url,
            account.local_storage.as_ref(),
        ) {
            Ok(()) => {
                if !json {
                    println!("opened \"{}\" → {}", account.id, session);
                }
                opened += 1;
                results.push(serde_json::json!({
                    "id": account.id, "session": session, "opened_url": url,
                    "ok": true, "error": serde_json::Value::Null,
                }));
                // Update last_used_at in the vault.
                if let Some(a) = vault.find_mut(&account.id) {
                    a.last_used_at = Some(Utc::now());
                }
            }
            Err(e) => {
                if !json {
                    eprintln!("warning: could not open \"{}\": {e:#}", account.id);
                }
                errors += 1;
                results.push(serde_json::json!({
                    "id": account.id, "session": session, "opened_url": serde_json::Value::Null,
                    "ok": false, "error": format!("{e:#}"),
                }));
            }
        }
    }

    // Persist last_used_at updates in one shot.
    if opened > 0 {
        vault.save()?;
    }

    if json {
        // Every selected account is reported (incl. failures) — exit stays 0 so
        // the GUI parses the per-account array rather than the exit code.
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({ "results": results }))?
        );
        return Ok(());
    }

    println!("opened {opened} account(s) in isolated windows");

    if opened == 0 && errors > 0 {
        Err(anyhow!("all {} account(s) failed to open", errors))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested below)
// ---------------------------------------------------------------------------

/// Select the accounts to open from `accounts` according to the CLI flags.
///
/// This is a pure function — no I/O — so it is easily unit-tested.
pub(crate) fn select<'a>(
    accounts: &'a [Account],
    id: Option<&str>,
    site: Option<&str>,
    all: bool,
) -> Result<Vec<&'a Account>> {
    if all {
        // All accounts, optionally narrowed to those whose site matches.
        let mut out: Vec<&Account> = accounts
            .iter()
            .filter(|a| site.map(|s| a.site.contains(s)).unwrap_or(true))
            .collect();
        if out.is_empty() {
            if let Some(s) = site {
                return Err(anyhow!("no accounts match site \"{s}\""));
            }
            return Err(anyhow!("vault is empty"));
        }
        // Stable ordering: by id, lexicographic.
        out.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(out);
    }

    if let Some(id) = id {
        return accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| vec![a])
            .ok_or_else(|| anyhow!("no account \"{id}\""));
    }

    if let Some(s) = site {
        let mut out: Vec<&Account> = accounts.iter().filter(|a| a.site.contains(s)).collect();
        if out.is_empty() {
            return Err(anyhow!("no accounts match site \"{s}\""));
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(out);
    }

    Err(anyhow!("specify an account id, --site <domain>, or --all"))
}

/// Build an isolated-session name from an account id.
///
/// Replaces every non-alphanumeric character with `'-'` so the name is
/// safe to use as a chrome-use session identifier, e.g.:
/// `"chatgpt/work-01"` → `"cookie-use-iso-chatgpt-work-01"`.
pub(crate) fn iso_session(id: &str) -> String {
    let slug: String = id
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    format!("cookie-use-iso-{slug}")
}

/// Derive the URL to open from a site spec.
///
/// The primary domain is the first comma-segment, trimmed and with any
/// leading `'.'` removed.  E.g.:
/// - `"chatgpt.com,openai.com"` → `"https://chatgpt.com"`
/// - `".example.com"`           → `"https://example.com"`
pub(crate) fn open_url_for(site: &str) -> String {
    let primary = site
        .split(',')
        .next()
        .unwrap_or(site)
        .trim()
        .trim_start_matches('.');
    format!("https://{primary}")
}

// ---------------------------------------------------------------------------
// Unit tests — pure helpers only; no browser launched
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ------------------------------------------------------------------
    // Test-account builder (only id and site matter for selection tests)
    // ------------------------------------------------------------------

    fn make_account(id: &str, site: &str) -> Account {
        Account {
            id: id.to_string(),
            site: site.to_string(),
            label: None,
            account_hint: None,
            cookies: vec![],
            local_storage: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_used_at: None,
            status: crate::vault::Status::Unknown,
            proxy: None,
            fingerprint: None,
        }
    }

    fn accounts() -> Vec<Account> {
        vec![
            make_account("chatgpt/work-01", "chatgpt.com,openai.com"),
            make_account("chatgpt/personal-02", "chatgpt.com,openai.com"),
            make_account("github/work", "github.com"),
        ]
    }

    // ------------------------------------------------------------------
    // select() tests
    // ------------------------------------------------------------------

    #[test]
    fn select_by_id_found() {
        let accs = accounts();
        let res = select(&accs, Some("github/work"), None, false).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].id, "github/work");
    }

    #[test]
    fn select_by_id_not_found() {
        let accs = accounts();
        let err = select(&accs, Some("does-not-exist"), None, false).unwrap_err();
        assert!(err.to_string().contains("no account"), "{err}");
    }

    #[test]
    fn select_by_site_substring() {
        let accs = accounts();
        let res = select(&accs, None, Some("chatgpt.com"), false).unwrap();
        assert_eq!(res.len(), 2);
        // Should be sorted by id.
        assert_eq!(res[0].id, "chatgpt/personal-02");
        assert_eq!(res[1].id, "chatgpt/work-01");
    }

    #[test]
    fn select_by_site_no_match() {
        let accs = accounts();
        let err = select(&accs, None, Some("nonexistent.com"), false).unwrap_err();
        assert!(err.to_string().contains("no accounts match"), "{err}");
    }

    #[test]
    fn select_all() {
        let accs = accounts();
        let res = select(&accs, None, None, true).unwrap();
        assert_eq!(res.len(), 3);
    }

    #[test]
    fn select_all_narrowed_by_site() {
        let accs = accounts();
        let res = select(&accs, None, Some("github"), true).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].id, "github/work");
    }

    #[test]
    fn select_all_site_no_match() {
        let accs = accounts();
        let err = select(&accs, None, Some("twitter.com"), true).unwrap_err();
        assert!(err.to_string().contains("no accounts match"), "{err}");
    }

    #[test]
    fn select_no_criteria_errors() {
        let accs = accounts();
        let err = select(&accs, None, None, false).unwrap_err();
        assert!(err.to_string().contains("specify an account id"), "{err}");
    }

    #[test]
    fn select_all_empty_vault() {
        let err = select(&[], None, None, true).unwrap_err();
        assert!(err.to_string().contains("vault is empty"), "{err}");
    }

    // ------------------------------------------------------------------
    // iso_session() tests
    // ------------------------------------------------------------------

    #[test]
    fn iso_session_slug_slashes_and_dashes() {
        assert_eq!(
            iso_session("chatgpt/work-01"),
            "cookie-use-iso-chatgpt-work-01"
        );
    }

    #[test]
    fn iso_session_alphanumeric_unchanged() {
        assert_eq!(iso_session("abc123"), "cookie-use-iso-abc123");
    }

    #[test]
    fn iso_session_special_chars_become_dash() {
        // Dots, colons, spaces all → '-'
        assert_eq!(
            iso_session("my.account:v2 x"),
            "cookie-use-iso-my-account-v2-x"
        );
    }

    // ------------------------------------------------------------------
    // open_url_for() tests
    // ------------------------------------------------------------------

    #[test]
    fn open_url_comma_list_takes_first() {
        assert_eq!(
            open_url_for("chatgpt.com,openai.com"),
            "https://chatgpt.com"
        );
    }

    #[test]
    fn open_url_leading_dot_stripped() {
        assert_eq!(open_url_for(".example.com"), "https://example.com");
    }

    #[test]
    fn open_url_single_domain() {
        assert_eq!(open_url_for("github.com"), "https://github.com");
    }

    #[test]
    fn open_url_whitespace_trimmed() {
        assert_eq!(
            open_url_for("  github.com , example.com"),
            "https://github.com"
        );
    }

    // ------------------------------------------------------------------
    // Edge-case: id takes precedence over site
    // ------------------------------------------------------------------

    #[test]
    fn select_id_wins_over_site() {
        // When both id and site are given (not --all), id wins.
        let accs = accounts();
        let res = select(&accs, Some("github/work"), Some("chatgpt.com"), false).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].id, "github/work");
    }
}
