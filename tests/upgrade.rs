//! `upgrade --check` / `--json` and the daily notice, against the real binary.
//! No network: the release API is a `file://` fixture; HOME, the cache and the
//! vault are temp paths. The opt-out variables (CI is set on GitHub Actions)
//! are removed from the child unless a test sets one on purpose.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const TEST_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cookie-use-up-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox {
            dir: dir.canonicalize().unwrap(),
        }
    }

    fn cache_file(&self) -> PathBuf {
        self.dir.join("cache/cookie-use/update-check.json")
    }

    fn seed_cache(&self, checked_at: u64, latest: &str) {
        std::fs::create_dir_all(self.cache_file().parent().unwrap()).unwrap();
        std::fs::write(
            self.cache_file(),
            format!(r#"{{"checked_at":{checked_at},"latest":"{latest}"}}"#),
        )
        .unwrap();
    }

    fn release(&self, tag: &str) -> String {
        let path = self.dir.join("release.json");
        std::fs::write(
            &path,
            format!(r#"{{"tag_name":"{tag}","prerelease":false,"draft":false}}"#),
        )
        .unwrap();
        format!("file://{}", path.display())
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_cookie-use"));
        c.args(args)
            .env("HOME", &self.dir)
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("COOKIE_USE_VAULT_KEY", TEST_KEY)
            .env("COOKIE_USE_VAULT", self.dir.join("vault.enc"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("GITHUB_TOKEN")
            .env_remove("CI")
            .env_remove("COOKIE_USE_NO_UPDATE_CHECK")
            .env_remove("USE_NO_UPDATE_CHECK");
        c
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn notice(latest: &str) -> String {
    format!("cookie-use {latest} is available (you have {CURRENT}). Upgrade: cookie-use upgrade")
}

#[test]
fn json_reports_update_and_installed_skills() {
    let sb = Sandbox::new();
    let skill = sb.dir.join(".agents/skills/cookie-use");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "placeholder").unwrap();
    std::fs::create_dir_all(sb.dir.join(".claude/plugins")).unwrap();
    std::fs::write(
        sb.dir.join(".claude/plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{"cookie-use@leeguooooo-plugins":[{"installPath":"/placeholder"}]}}"#,
    )
    .unwrap();

    let out = sb
        .cmd(&["upgrade", "--json"])
        .env("COOKIE_USE_RELEASE_API_URL", sb.release("v999.0.0"))
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["name"], "cookie-use");
    assert_eq!(v["current"], CURRENT);
    assert_eq!(v["latest"], "999.0.0");
    assert_eq!(v["update_available"], true);
    let channels: Vec<&str> = v["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["channel"].as_str().unwrap())
        .collect();
    assert!(channels.contains(&"claude-plugin"), "{v}");
    assert!(channels.contains(&"copied"), "{v}");
    // Nothing but the check cache changed; upgrade itself prints no notice.
    assert_eq!(
        std::fs::read(skill.join("SKILL.md")).unwrap(),
        b"placeholder"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("is available"));
    let cache: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(sb.cache_file()).unwrap()).unwrap();
    assert_eq!(cache["latest"], "999.0.0");
}

#[test]
fn check_up_to_date_exits_zero() {
    let sb = Sandbox::new();
    let out = sb
        .cmd(&["upgrade", "--check"])
        .env(
            "COOKIE_USE_RELEASE_API_URL",
            sb.release(&format!("v{CURRENT}")),
        )
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().next().unwrap(),
        format!("cookie-use {CURRENT} is up to date")
    );
}

#[test]
fn failed_check_exits_two() {
    let sb = Sandbox::new();
    let missing = format!("file://{}", sb.dir.join("missing.json").display());
    let out = sb
        .cmd(&["upgrade", "--check"])
        .env("COOKIE_USE_RELEASE_API_URL", &missing)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = sb
        .cmd(&["upgrade", "--json"])
        .env("COOKIE_USE_RELEASE_API_URL", &missing)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["error"].is_string());
    assert_eq!(v["update_available"], false);
}

#[test]
fn notice_is_one_stderr_line_and_stdout_stays_json() {
    let sb = Sandbox::new();
    sb.seed_cache(now(), "999.0.0");
    let out = sb.cmd(&["list", "--json"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.lines().filter(|l| *l == notice("999.0.0")).count(),
        1,
        "{stderr}"
    );
    let _: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
}

#[test]
fn opt_outs_and_meta_flags_suppress_the_notice() {
    let sb = Sandbox::new();
    sb.seed_cache(now(), "999.0.0");
    for var in ["CI", "COOKIE_USE_NO_UPDATE_CHECK", "USE_NO_UPDATE_CHECK"] {
        let out = sb.cmd(&["list"]).env(var, "1").output().unwrap();
        assert!(
            !String::from_utf8_lossy(&out.stderr).contains("is available"),
            "{var}"
        );
    }
    for args in [&["--version"][..], &["--help"], &["list", "--help"]] {
        let out = sb.cmd(args).output().unwrap();
        let all = [out.stdout, out.stderr].concat();
        assert!(
            !String::from_utf8_lossy(&all).contains("is available"),
            "{args:?}"
        );
    }
}

#[test]
fn fresh_cache_is_not_rechecked_and_stale_cache_is_bumped_even_offline() {
    let sb = Sandbox::new();
    let fresh = now() - 60;
    sb.seed_cache(fresh, "0.0.1");
    let before = std::fs::read_to_string(sb.cache_file()).unwrap();
    sb.cmd(&["list"]).output().unwrap();
    assert_eq!(std::fs::read_to_string(sb.cache_file()).unwrap(), before);

    sb.seed_cache(1, "0.0.1");
    // An unreachable API: the detached check fails, checked_at still moves.
    let missing = format!(
        "file://{}",
        Path::new(&sb.dir).join("missing.json").display()
    );
    sb.cmd(&["list"])
        .env("COOKIE_USE_RELEASE_API_URL", missing)
        .output()
        .unwrap();
    let cache: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(sb.cache_file()).unwrap()).unwrap();
    assert!(cache["checked_at"].as_u64().unwrap() > 1);
    assert_eq!(cache["latest"], "0.0.1");
}
