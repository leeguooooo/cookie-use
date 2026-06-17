//! Binary-level integration tests. Each runs the real `cookie-use` binary
//! against an ISOLATED vault, with no Keychain and no browser, via two env
//! vars — `COOKIE_USE_VAULT_KEY` (a fixed key that bypasses the Keychain) and
//! `COOKIE_USE_VAULT` (a unique temp vault path) — so they are safe to run
//! anywhere, including headless CI.

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Fixed 32-byte key (`"0123456789abcdef0123456789abcdef"`), base64.
const TEST_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A throwaway vault path + bundle dir, unique per test, cleaned on drop.
struct Sandbox {
    dir: std::path::PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cookie-use-it-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }

    fn vault(&self) -> std::path::PathBuf {
        self.dir.join("vault.enc")
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.join(name)
    }

    /// A `cookie-use` invocation wired to this sandbox's isolated vault.
    fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_cookie-use"));
        c.env("COOKIE_USE_VAULT_KEY", TEST_KEY)
            .env("COOKIE_USE_VAULT", self.vault());
        c
    }

    /// Seed an account by importing a cookie-header file (no browser needed).
    fn seed(&self, id: &str, site: &str) {
        let cookie_file = self.path(&format!("{}.cookies", id.replace('/', "_")));
        std::fs::write(&cookie_file, "session=abc123; token=xyz789").unwrap();
        let out = self
            .cmd()
            .args(["import", "--file"])
            .arg(&cookie_file)
            .args(["--site", site, "--id", id])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "seed import failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}
fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn version_reports_current() {
    let out = Command::new(env!("CARGO_BIN_EXE_cookie-use"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        stdout_of(&out).contains(env!("CARGO_PKG_VERSION")),
        "version: {}",
        stdout_of(&out)
    );
}

#[test]
fn import_list_show_roundtrip() {
    let sb = Sandbox::new();
    sb.seed("acme/work", "acme.com");

    let list = sb.cmd().arg("list").output().unwrap();
    assert!(list.status.success());
    assert!(stdout_of(&list).contains("acme/work"));

    let show = sb.cmd().args(["show", "acme/work"]).output().unwrap();
    assert!(show.status.success());
    let s = stdout_of(&show);
    assert!(s.contains("acme.com"), "show missing site: {s}");
    // Trust banner from the show enhancement.
    assert!(
        s.contains("local-only"),
        "show missing local-only banner: {s}"
    );
    // Never leak a cookie value.
    assert!(!s.contains("xyz789"), "show leaked a cookie value!");
}

#[test]
fn share_redeem_roundtrip() {
    let sb = Sandbox::new();
    sb.seed("acme/prod", "acme.com");
    let bundle = sb.path("prod.cusession");

    let share = sb
        .cmd()
        .args(["share", "acme/prod", "--password", "hunter2", "--out"])
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(share.status.success(), "share: {}", stderr_of(&share));

    // The bundle must not contain the plaintext cookie value.
    let bytes = std::fs::read(&bundle).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("xyz789"),
        "bundle leaked a cookie value!"
    );

    // Wrong password is rejected.
    let bad = sb
        .cmd()
        .args(["redeem"])
        .arg(&bundle)
        .args(["--password", "WRONG", "--id", "acme/x"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(stderr_of(&bad).contains("wrong password"));

    // Correct password redeems under a new id.
    let good = sb
        .cmd()
        .args(["redeem"])
        .arg(&bundle)
        .args(["--password", "hunter2", "--id", "acme/copy"])
        .output()
        .unwrap();
    assert!(good.status.success(), "redeem: {}", stderr_of(&good));

    let list = sb.cmd().arg("list").output().unwrap();
    let s = stdout_of(&list);
    assert!(
        s.contains("acme/prod") && s.contains("acme/copy"),
        "list: {s}"
    );
}

#[test]
fn wipe_clears_the_vault() {
    let sb = Sandbox::new();
    sb.seed("acme/one", "acme.com");
    sb.seed("acme/two", "acme.com");

    let wipe = sb.cmd().args(["wipe", "--yes"]).output().unwrap();
    assert!(wipe.status.success(), "wipe: {}", stderr_of(&wipe));
    assert!(!sb.vault().exists(), "vault file should be gone after wipe");

    let list = sb.cmd().arg("list").output().unwrap();
    assert!(stdout_of(&list).contains("no accounts"));
}

// --- the confirm-gate regression (the inverted-boolean bug) ---------------

#[test]
fn as_default_refuses_injection_noninteractive() {
    // Regression: `as` once skipped the gate by default (inverted boolean).
    // With no --no-confirm and no COOKIE_USE_YES, a non-interactive run MUST
    // refuse to inject rather than proceed.
    let sb = Sandbox::new();
    sb.seed("acme/agent", "acme.com");

    let out = sb
        .cmd()
        .args([
            "as",
            "acme/agent",
            "--target",
            "isolated",
            "--",
            "echo",
            "hi",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("refusing to inject"),
        "expected refusal, got: {}",
        stderr_of(&out)
    );
}

#[test]
fn as_no_confirm_passes_the_gate() {
    // With --no-confirm the gate must NOT fire. (It then fails later trying to
    // reach chrome-use, which is fine — we only assert the gate was bypassed.)
    let sb = Sandbox::new();
    sb.seed("acme/agent", "acme.com");

    let out = sb
        .cmd()
        .args([
            "as",
            "acme/agent",
            "--target",
            "isolated",
            "--no-confirm",
            "--",
            "echo",
            "hi",
        ])
        .output()
        .unwrap();
    assert!(
        !stderr_of(&out).contains("refusing to inject"),
        "gate should have been bypassed by --no-confirm, got: {}",
        stderr_of(&out)
    );
}

#[test]
fn as_empty_command_is_rejected() {
    let sb = Sandbox::new();
    sb.seed("acme/agent", "acme.com");
    let out = sb
        .cmd()
        .args(["as", "acme/agent", "--no-confirm"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("provide a command"));
}
