//! install.sh against a local file:// "release": the checksum is mandatory, a
//! pinned version must match what the new binary reports, and any failure
//! leaves the existing binary untouched. macOS only (install.sh is).
#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);
const OLD: &str = "#!/bin/sh\necho 'cookie-use 0.0.1'\n";

struct Release {
    dir: PathBuf,
}

impl Release {
    /// A release directory whose tarball holds a `cookie-use` reporting `version`.
    fn new(version: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cookie-use-inst-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir = {
            std::fs::create_dir_all(dir.join("pkg")).unwrap();
            std::fs::create_dir_all(dir.join("assets")).unwrap();
            std::fs::create_dir_all(dir.join("bin")).unwrap();
            dir.canonicalize().unwrap()
        };
        let bin = dir.join("pkg/cookie-use");
        std::fs::write(&bin, format!("#!/bin/sh\necho 'cookie-use {version}'\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tarball = dir.join("assets").join(asset());
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&tarball)
            .arg("-C")
            .arg(dir.join("pkg"))
            .arg("cookie-use")
            .status()
            .unwrap()
            .success());
        let sum = Command::new("shasum")
            .args(["-a", "256"])
            .arg(&tarball)
            .output()
            .unwrap();
        std::fs::write(sidecar(&dir), sum.stdout).unwrap();
        // The binary already installed.
        let old = dir.join("bin/cookie-use");
        std::fs::write(&old, OLD).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        Release { dir }
    }

    fn install(&self, pin: Option<&str>) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
        let mut c = Command::new("sh");
        c.arg(script)
            .env(
                "COOKIE_USE_DOWNLOAD_BASE",
                format!("file://{}", self.dir.join("assets").display()),
            )
            .env("COOKIE_USE_BIN_DIR", self.dir.join("bin"))
            .env("HOME", &self.dir)
            .env_remove("COOKIE_USE_VERSION");
        if let Some(v) = pin {
            c.env("COOKIE_USE_VERSION", v);
        }
        c.output().unwrap()
    }

    fn installed(&self) -> String {
        std::fs::read_to_string(self.dir.join("bin/cookie-use")).unwrap()
    }

    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(self.dir.join("bin"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "cookie-use")
            .collect()
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn asset() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "cookie-use-darwin-arm64.tar.gz"
    } else {
        "cookie-use-darwin-x64.tar.gz"
    }
}

fn sidecar(dir: &Path) -> PathBuf {
    dir.join("assets").join(format!("{}.sha256", asset()))
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn verified_release_replaces_the_binary() {
    let r = Release::new("9.9.9");
    let out = r.install(Some("v9.9.9"));
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(r.installed().contains("cookie-use 9.9.9"));
    assert!(r.leftovers().is_empty(), "{:?}", r.leftovers());
}

#[test]
fn checksum_mismatch_keeps_the_old_binary() {
    let r = Release::new("9.9.9");
    std::fs::write(sidecar(&r.dir), format!("{}  x\n", "0".repeat(64))).unwrap();
    let out = r.install(None);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("checksum mismatch"),
        "{}",
        stderr(&out)
    );
    assert_eq!(r.installed(), OLD);
    assert!(r.leftovers().is_empty());
}

#[test]
fn missing_checksum_is_a_failed_install() {
    let r = Release::new("9.9.9");
    std::fs::remove_file(sidecar(&r.dir)).unwrap();
    let out = r.install(None);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unverified"), "{}", stderr(&out));
    assert_eq!(r.installed(), OLD);
}

#[test]
fn pinned_version_must_match_the_downloaded_binary() {
    let r = Release::new("9.9.9");
    let out = r.install(Some("v1.2.3"));
    assert!(!out.status.success());
    assert!(stderr(&out).contains("expected 1.2.3"), "{}", stderr(&out));
    assert_eq!(r.installed(), OLD);

    let out = r.install(Some("latest; echo pwned"));
    assert!(!out.status.success());
    assert_eq!(r.installed(), OLD);
}

/// `cookie-use upgrade` end to end: the real CLI drives install.sh (file://)
/// against a temp install. Never touches a real binary.
fn upgrade(r: &Release, args: &[&str]) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    std::fs::write(
        r.dir.join("release.json"),
        r#"{"tag_name":"v9.9.9","prerelease":false,"draft":false}"#,
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_cookie-use"))
        .arg("upgrade")
        .args(args)
        .env("HOME", &r.dir)
        .env("XDG_CACHE_HOME", r.dir.join("cache"))
        .env("COOKIE_USE_APP_PATH", r.dir.join("NoApp.app"))
        .env("COOKIE_USE_UPGRADE_EXE", r.dir.join("bin/cookie-use"))
        .env(
            "COOKIE_USE_INSTALL_URL",
            format!("file://{}", script.display()),
        )
        .env(
            "COOKIE_USE_DOWNLOAD_BASE",
            format!("file://{}", r.dir.join("assets").display()),
        )
        .env(
            "COOKIE_USE_RELEASE_API_URL",
            format!("file://{}", r.dir.join("release.json").display()),
        )
        .env_remove("COOKIE_USE_VERSION")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .unwrap()
}

#[test]
fn upgrade_installs_the_verified_release_in_place() {
    let r = Release::new("9.9.9");
    let out = upgrade(&r, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", stderr(&out));
    assert!(stdout.contains("-> 9.9.9"), "{stdout}");
    assert!(r.installed().contains("cookie-use 9.9.9"));
}

#[test]
fn upgrade_with_a_bad_checksum_exits_two_and_keeps_the_old_binary() {
    let r = Release::new("9.9.9");
    std::fs::write(sidecar(&r.dir), format!("{}  x\n", "f".repeat(64))).unwrap();
    let out = upgrade(&r, &[]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("was left as it was"),
        "{}",
        stderr(&out)
    );
    assert_eq!(r.installed(), OLD);
}

#[test]
fn upgrade_to_a_pinned_tag_the_release_does_not_hold_is_refused() {
    let r = Release::new("9.9.9");
    let out = upgrade(&r, &["--tag", "v1.0.0"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(r.installed(), OLD);
}
