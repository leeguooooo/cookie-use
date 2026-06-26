//! `cookie-use as <id> [--target <t>] -- <command…>`
//!
//! Applies an account's session into a browser target, then runs a child
//! command in an environment scoped to that session.  Designed for agents
//! that need to "act as" a specific stored account for a single task without
//! touching any other session in the vault.

use anyhow::{anyhow, Result};
use chrono::Utc;

// ─── public entry-point ──────────────────────────────────────────────────────

/// Apply account `id` to `target`, then run `command` as a child process.
///
/// The child inherits stdio and an extended environment with session
/// coordinates so it can drive the same browser session (e.g. a nested
/// `chrome-use` call).  The child's exit code is propagated: a non-zero
/// exit becomes an `Err`.
///
/// # Errors
///
/// - `command` is empty.
/// - `id` is not found in the vault.
/// - biometric / TTY gate rejected (unless `no_confirm`).
/// - `Target::parse` fails for `target`.
/// - `chrome_use::apply` fails (chrome-use not running, extension not
///   connected, etc.).
/// - The child process exits with a non-zero status.
pub fn cmd_as(
    vault: &mut crate::vault::Vault,
    id: &str,
    target: &str,
    command: &[String],
    no_confirm: bool,
    json: bool,
) -> Result<()> {
    validate(command)?;

    let account = vault
        .find(id)
        .ok_or_else(|| anyhow!("no account \"{id}\""))?
        .clone();

    crate::confirm::require(&format!("act as \"{id}\""), no_confirm)?;

    let parsed_target = crate::chrome_use::Target::parse(target)?;

    let open_url = format!("https://{}", primary_domain(&account.site));
    let opts = crate::chrome_use::ApplyOpts {
        rewrite_domain: None,
        open_url: Some(&open_url),
        local_storage: account.local_storage.as_ref(),
    };
    crate::chrome_use::apply(&account.cookies, &parsed_target, &opts)?;

    if let Some(a) = vault.find_mut(id) {
        a.last_used_at = Some(Utc::now());
    }
    vault.save()?;

    // Announce the applied session before handing stdout to the child (which
    // inherits our stdio). In --json this is the single structured line.
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "id": id, "session": parsed_target.session_name(), "ok": true,
            }))?
        );
    } else {
        let cmd_display = command.join(" ");
        println!("acting as \"{id}\" — running: {cmd_display}");
    }

    let env_vars = child_env(id, &account.site, target);
    let status = std::process::Command::new(&command[0])
        .args(&command[1..])
        .envs(env_vars)
        .status()
        .map_err(|e| anyhow!("failed to spawn {:?}: {e}", command[0]))?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(anyhow!("child process exited with code {code}"));
    }
    Ok(())
}

// ─── pure helpers (unit-testable) ────────────────────────────────────────────

/// Validate that a command was provided; returns `Err` if the slice is empty.
///
/// Factored out so the guard is testable without spawning a process.
pub(crate) fn validate(command: &[String]) -> Result<()> {
    if command.is_empty() {
        return Err(anyhow!(
            "provide a command to run after `--`, \
             e.g. `cookie-use as <id> -- chrome-use open ...`"
        ));
    }
    Ok(())
}

/// Build the extra environment variables a child process receives.
///
/// - `COOKIE_USE_ACCOUNT` — the account id.
/// - `COOKIE_USE_SITE`    — the account's site string.
/// - `COOKIE_USE_TARGET`  — the raw target string (e.g. `session:default`).
/// - `CHROME_USE_SESSION` — the session name the child should drive:
///   - `session:<name>` → `<name>`
///   - `isolated`       → `cookie-use-iso`
///
/// Factored as a pure function so it is testable without side effects.
pub(crate) fn child_env(id: &str, site: &str, target: &str) -> Vec<(String, String)> {
    let chrome_session = if let Some(name) = target.strip_prefix("session:") {
        name.to_string()
    } else {
        // "isolated" (or any unrecognised value) maps to the throwaway session
        // name used by chrome_use::apply.
        "cookie-use-iso".to_string()
    };

    vec![
        ("COOKIE_USE_ACCOUNT".into(), id.into()),
        ("COOKIE_USE_SITE".into(), site.into()),
        ("COOKIE_USE_TARGET".into(), target.into()),
        ("CHROME_USE_SESSION".into(), chrome_session),
    ]
}

/// First domain in a comma-separated site list, without a leading dot.
fn primary_domain(site: &str) -> String {
    site.split(',')
        .next()
        .unwrap_or(site)
        .trim()
        .trim_start_matches('.')
        .to_string()
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate ---

    #[test]
    fn validate_empty_command_is_err() {
        let result = validate(&[]);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("provide a command"),
            "error message should hint at `--`: {msg}"
        );
    }

    #[test]
    fn validate_nonempty_command_is_ok() {
        let cmd = vec!["chrome-use".to_string(), "open".to_string()];
        assert!(validate(&cmd).is_ok());
    }

    // --- child_env ---

    #[test]
    fn child_env_session_target_sets_chrome_session_name() {
        let env = child_env("chatgpt/work-01", "chatgpt.com", "session:default");
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();

        assert_eq!(map["COOKIE_USE_ACCOUNT"], "chatgpt/work-01");
        assert_eq!(map["COOKIE_USE_SITE"], "chatgpt.com");
        assert_eq!(map["COOKIE_USE_TARGET"], "session:default");
        assert_eq!(map["CHROME_USE_SESSION"], "default");
    }

    #[test]
    fn child_env_isolated_target_sets_iso_session() {
        let env = child_env("gh/alice", "github.com", "isolated");
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();

        assert_eq!(map["COOKIE_USE_ACCOUNT"], "gh/alice");
        assert_eq!(map["COOKIE_USE_SITE"], "github.com");
        assert_eq!(map["COOKIE_USE_TARGET"], "isolated");
        assert_eq!(map["CHROME_USE_SESSION"], "cookie-use-iso");
    }

    #[test]
    fn child_env_named_session_preserves_arbitrary_name() {
        let env = child_env("x/1", "x.com", "session:my-agent-42");
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map["CHROME_USE_SESSION"], "my-agent-42");
    }

    #[test]
    fn child_env_has_exactly_four_vars() {
        let env = child_env("a", "b.com", "session:s");
        assert_eq!(env.len(), 4);
    }

    // --- primary_domain ---

    #[test]
    fn primary_domain_strips_leading_dot() {
        // site stored with a leading dot (cookie domain style)
        assert_eq!(primary_domain(".example.com"), "example.com");
    }

    #[test]
    fn primary_domain_takes_first_of_comma_list() {
        assert_eq!(primary_domain("chatgpt.com,openai.com"), "chatgpt.com");
    }

    #[test]
    fn primary_domain_trims_whitespace() {
        assert_eq!(primary_domain(" github.com , api.github.com"), "github.com");
    }
}
