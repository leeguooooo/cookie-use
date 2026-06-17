//! Confirmation gate for dangerous (injection) actions.
//!
//! Two public entry-points:
//!
//! * [`require`] — full injection gate: tries Touch ID on macOS, falls back to
//!   TTY prompt, respects `COOKIE_USE_YES` and `--no-confirm` (`skip`).
//! * [`confirm_tty`] — plain destructive-action prompt over stdin/stderr.

use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::Command;

use anyhow::{anyhow, Result};

// ---------------------------------------------------------------------------
// Decision type — pure, testable
// ---------------------------------------------------------------------------

/// The chosen confirmation strategy, derived from environment without side-effects.
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Skip all prompts — proceed immediately.
    Allow,
    /// Attempt Touch ID; fall back to TTY on hardware/toolchain unavailability.
    Biometric,
    /// Non-interactive caller without a bypass — refuse.
    Deny,
}

/// Pure decision function.  All inputs are passed explicitly so the logic is
/// trivially unit-testable without touching IO or environment.
///
/// # Parameters
/// * `skip`    — caller passed `--no-confirm`.
/// * `yes_env` — env var `COOKIE_USE_YES` is set and non-empty.
/// * `is_tty`  — stdin is a real terminal.
pub fn decide(skip: bool, yes_env: bool, is_tty: bool) -> Decision {
    if skip {
        return Decision::Allow;
    }
    if yes_env {
        return Decision::Allow;
    }
    if is_tty {
        return Decision::Biometric;
    }
    Decision::Deny
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Injection gate: require explicit confirmation before writing a live session
/// into a browser context.
///
/// * `action` — human-readable description of what is being injected.
/// * `skip`   — if `true` (i.e. `--no-confirm` was passed), the gate opens
///   unconditionally.
///
/// Uses [`decide`] to pick a strategy, then executes it:
/// - `Allow`    → returns `Ok(())` immediately.
/// - `Biometric`→ tries Touch ID; falls back to [`confirm_tty`] when biometrics
///   are unavailable (no `swift` binary, unsupported hardware, …).
/// - `Tty`      → delegates to [`confirm_tty`].
/// - `Deny`     → returns a descriptive error.
pub fn require(action: &str, skip: bool) -> Result<()> {
    let yes_env = env::var("COOKIE_USE_YES")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let is_tty = io::stdin().is_terminal();

    match decide(skip, yes_env, is_tty) {
        Decision::Allow => Ok(()),
        Decision::Biometric => {
            match try_touch_id(action) {
                TouchIdResult::Approved => Ok(()),
                TouchIdResult::Denied => Err(anyhow!("authentication failed or cancelled")),
                // No swift / no biometric hardware → fall back to keyboard prompt.
                TouchIdResult::Unavailable => {
                    eprintln!("note: touch id unavailable, falling back to keyboard prompt");
                    confirm_tty(action)
                }
            }
        }
        Decision::Deny => Err(deny_error(action)),
    }
}

/// Plain destructive-action prompt over stdin/stderr.
///
/// Prints `<action> — proceed? [y/N] ` to stderr, reads a line from stdin,
/// and accepts `y` / `yes` (case-insensitive, trimmed).  Any other input
/// (including empty / EOF) returns an error.
///
/// Returns an error immediately when stdin is not a terminal, mirroring the
/// `Deny` path in [`require`].
pub fn confirm_tty(action: &str) -> Result<()> {
    if !io::stdin().is_terminal() {
        return Err(deny_error(action));
    }
    eprint!("{action} — proceed? [y/N] ");
    io::stderr().flush().ok();

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let ans = line.trim().to_lowercase();
    if ans == "y" || ans == "yes" {
        Ok(())
    } else {
        Err(anyhow!("aborted"))
    }
}

// ---------------------------------------------------------------------------
// Touch ID via Swift / LocalAuthentication
// ---------------------------------------------------------------------------

enum TouchIdResult {
    /// Authentication succeeded.
    Approved,
    /// User cancelled or biometric match failed.
    Denied,
    /// Swift not found, hardware unavailable, or any spawn error.
    Unavailable,
}

/// Shells out to a tiny Swift program that calls `LAContext.evaluatePolicy`.
///
/// Writes the Swift source to a temp file, runs `swift <file>`, reads stdout
/// for "OK" / "FAIL", and cleans up the temp file regardless of outcome.
fn try_touch_id(reason: &str) -> TouchIdResult {
    // Escape the reason string for embedding in Swift source.
    let safe_reason = reason.replace('\\', "\\\\").replace('"', "\\\"");

    let swift_src = format!(
        r#"import LocalAuthentication
import Foundation

let ctx = LAContext()
var err: NSError?
guard ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &err) else {{
    print("UNAVAILABLE")
    exit(2)
}}

let sem = DispatchSemaphore(value: 0)
var success = false
ctx.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics,
                   localizedReason: "{safe_reason}") {{ ok, _ in
    success = ok
    sem.signal()
}}
sem.wait()
if success {{
    print("OK")
    exit(0)
}} else {{
    print("FAIL")
    exit(1)
}}
"#
    );

    // Write to a temp file.
    let tmp_path = env::temp_dir().join(format!("cookie_use_touchid_{}.swift", std::process::id()));
    if fs::write(&tmp_path, &swift_src).is_err() {
        return TouchIdResult::Unavailable;
    }

    let outcome = run_swift(&tmp_path.to_string_lossy());

    // Clean up — best effort.
    let _ = fs::remove_file(&tmp_path);

    outcome
}

fn run_swift(path: &str) -> TouchIdResult {
    let result = Command::new("swift").arg(path).output();

    let output = match result {
        Ok(o) => o,
        Err(_) => return TouchIdResult::Unavailable, // swift not found / spawn error
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();

    if trimmed == "UNAVAILABLE" {
        return TouchIdResult::Unavailable;
    }
    if trimmed.contains("UNAVAILABLE") {
        return TouchIdResult::Unavailable;
    }

    match output.status.code() {
        Some(0) if trimmed == "OK" => TouchIdResult::Approved,
        Some(1) => TouchIdResult::Denied,
        Some(2) => TouchIdResult::Unavailable,
        // Any other exit code or unexpected output → treat as unavailable so
        // the caller can fall back gracefully.
        _ => TouchIdResult::Unavailable,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deny_error(action: &str) -> anyhow::Error {
    anyhow!(
        "refusing to inject \"{action}\" without confirmation in a non-interactive shell; \
         pass --no-confirm or set COOKIE_USE_YES=1"
    )
}

// ---------------------------------------------------------------------------
// Tests — pure decision matrix only (no IO)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_always_allows() {
        assert_eq!(decide(true, false, false), Decision::Allow);
        assert_eq!(decide(true, true, false), Decision::Allow);
        assert_eq!(decide(true, false, true), Decision::Allow);
        assert_eq!(decide(true, true, true), Decision::Allow);
    }

    #[test]
    fn yes_env_allows_when_not_skipped() {
        // skip=false, yes_env=true → Allow regardless of tty
        assert_eq!(decide(false, true, false), Decision::Allow);
        assert_eq!(decide(false, true, true), Decision::Allow);
    }

    #[test]
    fn tty_without_bypass_chooses_biometric() {
        assert_eq!(decide(false, false, true), Decision::Biometric);
    }

    #[test]
    fn non_tty_without_bypass_denies() {
        assert_eq!(decide(false, false, false), Decision::Deny);
    }
}
