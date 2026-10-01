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

// --- fingerprint ----------------------------------------------------------

#[test]
fn fingerprint_hashes_values_caches_and_never_leaks() {
    let sb = Sandbox::new();
    // Import a high-entropy value (>= 8 chars) so it isn't excluded.
    let cookie_file = sb.path("fp.cookies");
    std::fs::write(&cookie_file, "sid=super-secret-session-token-1234").unwrap();
    let imp = sb
        .cmd()
        .args(["import", "--file"])
        .arg(&cookie_file)
        .args(["--site", "acme.com", "--id", "acme/fp"])
        .output()
        .unwrap();
    assert!(imp.status.success(), "import: {}", stderr_of(&imp));

    // Single-id JSON: one account object with a hashed cookie, no value leak.
    let out = sb
        .cmd()
        .args(["fingerprint", "acme/fp", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "fingerprint: {}", stderr_of(&out));
    let s = stdout_of(&out);
    assert!(s.contains("\"sha256\""), "no sha256: {s}");
    assert!(s.contains("\"httpOnly\""), "no httpOnly: {s}");
    assert!(
        !s.contains("super-secret-session-token-1234"),
        "fingerprint leaked a cookie value: {s}"
    );

    // A plaintext cache now sits next to the vault (readable without a decrypt).
    let cache = sb.path("fingerprints.json");
    assert!(cache.exists(), "no fingerprint cache written");
    let cache_txt = std::fs::read_to_string(&cache).unwrap();
    assert!(
        !cache_txt.contains("super-secret-session-token-1234"),
        "cache leaked a cookie value"
    );
    assert!(cache_txt.contains("\"sha256\""), "cache: {cache_txt}");

    // `--all` uses the cache and wraps in {"accounts":[…]} like `list`.
    let all = sb
        .cmd()
        .args(["fingerprint", "--all", "--json"])
        .output()
        .unwrap();
    assert!(
        all.status.success(),
        "fingerprint --all: {}",
        stderr_of(&all)
    );
    let a = stdout_of(&all);
    assert!(a.contains("\"accounts\""), "all missing wrapper: {a}");
    assert!(a.contains("acme/fp"), "all missing id: {a}");
}

#[test]
fn fingerprint_excludes_low_entropy_values() {
    let sb = Sandbox::new();
    // seed() writes session=abc123; token=xyz789 — both values are < 8 chars.
    sb.seed("acme/lo", "acme.com");
    let out = sb
        .cmd()
        .args(["fingerprint", "acme/lo", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "fingerprint: {}", stderr_of(&out));
    assert!(
        stdout_of(&out).contains("\"cookies\":[]"),
        "low-entropy values should yield no fingerprinted cookies: {}",
        stdout_of(&out)
    );
}

#[test]
fn fingerprint_all_skips_and_flags_uncached_accounts() {
    let sb = Sandbox::new();
    sb.seed("acme/a", "acme.com");
    // Simulate an account stored before fingerprints existed by dropping the
    // auto-written cache. `--all` must SKIP it (not compute) and warn on stderr.
    std::fs::remove_file(sb.path("fingerprints.json")).ok();
    let out = sb
        .cmd()
        .args(["fingerprint", "--all", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fingerprint --all: {}",
        stderr_of(&out)
    );
    assert!(
        stderr_of(&out).contains("no fingerprint yet"),
        "expected an uncached warning on stderr: {}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).contains("\"accounts\":[]"),
        "uncached account must be skipped, not computed: {}",
        stdout_of(&out)
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

fn list_json(sb: &Sandbox, filter: Option<&str>) -> serde_json::Value {
    let mut c = sb.cmd();
    c.arg("list");
    if let Some(f) = filter {
        c.arg(f);
    }
    let out = c.arg("--json").output().unwrap();
    assert!(out.status.success(), "list failed: {}", stderr_of(&out));
    serde_json::from_str(&stdout_of(&out)).unwrap()
}

#[test]
fn edit_sets_metadata_that_survives_recapture() {
    let sb = Sandbox::new();
    sb.seed("x/qa-admin", "x.com");

    let out = sb
        .cmd()
        .args([
            "edit",
            "x/qa-admin",
            "--label",
            "QA admin",
            "--note",
            "2FA on work phone",
        ])
        .args(["--tags", "Prod, admin,prod", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "edit failed: {}", stderr_of(&out));
    let edited: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    assert_eq!(edited["tags"], serde_json::json!(["prod", "admin"]));

    let row = &list_json(&sb, None)["accounts"][0];
    assert_eq!(row["label"], "QA admin");
    assert_eq!(row["note"], "2FA on work phone");
    assert_eq!(row["tags"], serde_json::json!(["prod", "admin"]));
    assert!(row.get("live_until").is_some() && row.get("updated_at").is_some());

    // Tags and notes are searchable through the list filter.
    assert_eq!(
        list_json(&sb, Some("admin"))["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        list_json(&sb, Some("work phone"))["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Re-capturing the same id refreshes cookies but keeps the user's metadata.
    sb.seed("x/qa-admin", "x.com");
    let row = &list_json(&sb, None)["accounts"][0];
    assert_eq!(row["label"], "QA admin");
    assert_eq!(row["tags"], serde_json::json!(["prod", "admin"]));

    // "" clears a field.
    let out = sb
        .cmd()
        .args(["edit", "x/qa-admin", "--note", "", "--tags", ""])
        .output()
        .unwrap();
    assert!(out.status.success());
    let row = &list_json(&sb, None)["accounts"][0];
    assert!(row["note"].is_null());
    assert_eq!(row["tags"], serde_json::json!([]));
}

#[test]
fn edit_without_fields_is_an_error() {
    let sb = Sandbox::new();
    sb.seed("x/a", "x.com");
    let out = sb.cmd().args(["edit", "x/a"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("nothing to edit"));
}

/// A minimal in-process CookieCloud server: `POST /update` stores the body per
/// uuid, `GET /get/<uuid>` returns it — the whole protocol cookie-use relies on.
fn mock_cookiecloud() -> String {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let store: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let store = store.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut stream = stream;
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, path) = (
                    parts.next().unwrap_or("").to_string(),
                    parts.next().unwrap_or("").to_string(),
                );
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let lower = h.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    if lower.starts_with("expect:") {
                        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
                    }
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let (code, reply) = if method == "POST" && path == "/update" {
                    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let uuid = v["uuid"].as_str().unwrap().to_string();
                    let saved = serde_json::json!({"encrypted": v["encrypted"], "crypto_type": v["crypto_type"]});
                    store.lock().unwrap().insert(uuid, saved.to_string());
                    (200, r#"{"action":"done"}"#.to_string())
                } else if let Some(uuid) = path.strip_prefix("/get/") {
                    match store.lock().unwrap().get(uuid) {
                        Some(s) => (200, s.clone()),
                        None => (404, "Not Found".into()),
                    }
                } else {
                    (404, "Not Found".into())
                };
                let resp = format!(
                    "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                stream.write_all(resp.as_bytes()).unwrap();
            });
        }
    });
    format!("http://{addr}")
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    assert!(out.status.success(), "failed: {}", stderr_of(out));
    serde_json::from_str(&stdout_of(out)).unwrap()
}

#[test]
fn cloud_sync_moves_accounts_tags_and_deletes_between_machines() {
    let server = mock_cookiecloud();
    let (a, b) = (Sandbox::new(), Sandbox::new());
    a.seed("x/alice", "x.com");
    a.seed("y/bob", "y.com");
    a.cmd()
        .args(["edit", "x/alice", "--tags", "prod"])
        .output()
        .unwrap();

    let cfg = json_of(
        &a.cmd()
            .args(["cloud", "setup", "--endpoint", &server, "--json"])
            .output()
            .unwrap(),
    );
    let (uuid, pw) = (
        cfg["uuid"].as_str().unwrap(),
        cfg["password"].as_str().unwrap(),
    );
    let r = json_of(&a.cmd().args(["cloud", "sync", "--json"]).output().unwrap());
    assert_eq!(r["pushed"], true);

    b.cmd()
        .args([
            "cloud",
            "setup",
            "--endpoint",
            &server,
            "--uuid",
            uuid,
            "--password",
            pw,
        ])
        .output()
        .unwrap();
    let r = json_of(&b.cmd().args(["cloud", "sync", "--json"]).output().unwrap());
    assert_eq!(r["added"].as_array().unwrap().len(), 2);
    let rows = list_json(&b, None);
    let alice = rows["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "x/alice")
        .unwrap()
        .clone();
    assert_eq!(alice["tags"], serde_json::json!(["prod"]));

    // A delete on B reaches A; an edit on B wins on A.
    b.cmd().args(["rm", "y/bob"]).output().unwrap();
    b.cmd()
        .args(["edit", "x/alice", "--note", "from b"])
        .output()
        .unwrap();
    b.cmd().args(["cloud", "sync"]).output().unwrap();
    let r = json_of(&a.cmd().args(["cloud", "pull", "--json"]).output().unwrap());
    assert_eq!(r["removed"], serde_json::json!(["y/bob"]));
    let rows = list_json(&a, None);
    assert_eq!(rows["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(rows["accounts"][0]["note"], "from b");

    // The wrong password is refused rather than merging garbage.
    let c = Sandbox::new();
    c.cmd()
        .args([
            "cloud",
            "setup",
            "--endpoint",
            &server,
            "--uuid",
            uuid,
            "--password",
            "wrong",
        ])
        .output()
        .unwrap();
    let out = c.cmd().args(["cloud", "pull"]).output().unwrap();
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("wrong uuid/password"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn export_bundle_moves_many_accounts_and_merges_by_recency() {
    let (a, b) = (Sandbox::new(), Sandbox::new());
    a.seed("x/one", "x.com");
    a.seed("x/two", "x.com");
    a.seed("y/three", "y.com");
    let bundle = a.path("all.cusession");
    let out = a
        .cmd()
        .args([
            "export",
            "--site",
            "x.com",
            "--password",
            "pw-123456",
            "--out",
        ])
        .arg(&bundle)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(json_of(&out)["accounts"], 2);
    // Cleartext index for previews; no cookie value anywhere outside the ciphertext.
    let raw = std::fs::read_to_string(&bundle).unwrap();
    assert!(raw.contains("\"x/one\"") && !raw.contains("abc123"));

    let r = json_of(
        &b.cmd()
            .args(["redeem"])
            .arg(&bundle)
            .args(["--password", "pw-123456", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(r["added"].as_array().unwrap().len(), 2);
    // Redeeming the same bundle again changes nothing.
    let r = json_of(
        &b.cmd()
            .args(["redeem"])
            .arg(&bundle)
            .args(["--password", "pw-123456", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(r["unchanged"], 2);
    let out = b
        .cmd()
        .args(["redeem"])
        .arg(&bundle)
        .args(["--password", "nope"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn copy_refuses_same_or_ambiguous_profiles() {
    let sb = Sandbox::new();
    let chrome = sb.path("chrome");
    std::fs::create_dir_all(&chrome).unwrap();
    std::fs::write(
        chrome.join("Local State"),
        r#"{"profile":{"info_cache":{"Default":{"name":"Leo","user_name":"leo@x.com"},
            "Profile 7":{"name":"wind","user_name":"w7@x.com"},"Profile 9":{"name":"wind","user_name":"w9@x.com"}}}}"#,
    )
    .unwrap();
    let run = |from: &str, to: &str| {
        sb.cmd()
            .env("COOKIE_USE_CHROME_DIR", &chrome)
            .args([
                "copy",
                "--site",
                "x.com",
                "--from",
                from,
                "--to",
                to,
                "--dry-run",
            ])
            .output()
            .unwrap()
    };
    let out = run("Leo", "Default");
    assert!(
        stderr_of(&out).contains("same profile"),
        "{}",
        stderr_of(&out)
    );
    let out = run("wind", "Leo");
    assert!(
        stderr_of(&out).contains("Profile 7") && stderr_of(&out).contains("Profile 9"),
        "{}",
        stderr_of(&out)
    );
}
