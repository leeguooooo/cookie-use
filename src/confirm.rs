//! Confirmation gate before dangerous actions.
//!
//! The dangerous action in cookie-use is *injecting a live session* into a
//! browser — not unlocking the vault. So we confirm at the point of injection.
//! On macOS this should be a Touch ID / biometric prompt (LocalAuthentication),
//! degrading to a TTY y/N prompt when biometrics are unavailable or stdin is
//! not a terminal (e.g. an agent driving the CLI with COOKIE_USE_YES=1).

use anyhow::Result;

/// Require confirmation for `action` (e.g. "apply session \"x\""). When `skip`
/// is true, returns Ok immediately. Prefers biometric auth, falls back to TTY.
pub fn require(_action: &str, _skip: bool) -> Result<()> {
    // STUB — implemented by the confirm worktree agent.
    Ok(())
}

/// A plain y/N TTY confirmation for destructive, non-injection actions (wipe).
pub fn confirm_tty(_action: &str) -> Result<()> {
    // STUB — implemented by the confirm worktree agent.
    Ok(())
}
