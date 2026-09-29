//! `cookie-use upgrade` and the daily "new version" notice, following the
//! *-use family convention
//! (https://github.com/leeguooooo/plugins/blob/main/docs/upgrade.md):
//!
//! - `upgrade` re-runs install.sh (the sha256-verified GitHub Release binary,
//!   swapped in by rename) into the running binary's directory, pinned to the
//!   target version. A brew / cargo / npm / source-build binary is refused.
//! - Skills are opt-in: `--skills` refreshes cookie-use's own copies; without
//!   it they are only listed. `--tag vX.Y.Z` installs one exact release.
//! - `upgrade --check` / `upgrade --json` change nothing and report current vs
//!   latest, the install channel, the installed skills and the GUI app.
//! - Exit 0 on success (upgraded, already current, or a check that ran), 2 when
//!   the check, download or verification failed, 1 when refused or unfinished.
//! - Other commands check GitHub at most once a day (in a detached child) and,
//!   while the cached release is newer, print one stderr line per run.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const NAME: &str = "cookie-use";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const PLUGIN_ID: &str = "cookie-use@leeguooooo-plugins";
const INSTALL_URL: &str = "https://raw.githubusercontent.com/leeguooooo/cookie-use/main/install.sh";
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/leeguooooo/cookie-use/releases/latest";

/// Overrides [`LATEST_RELEASE_API`]. Tests point it at a `file://` fixture so
/// no test touches the network; curl reads `file://` URLs natively.
const RELEASE_API_ENV: &str = "COOKIE_USE_RELEASE_API_URL";

/// Hidden subcommand that runs the daily check in a detached child.
pub const UPDATE_CHECK_CMD: &str = "__update-check";

const UPDATE_CHECK_INTERVAL_SECS: u64 = 86_400;
const NOTICE_CHECK_TIMEOUT_SECS: u64 = 2;
const EXPLICIT_CHECK_TIMEOUT_SECS: u64 = 10;

/// Any of these disables both the daily check and the notice.
const CHECK_OPT_OUT_VARS: &[&str] = &["CI", "COOKIE_USE_NO_UPDATE_CHECK", "USE_NO_UPDATE_CHECK"];

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Versions and the release lookup
// ---------------------------------------------------------------------------

/// `1.2.3`, `v1.2.3`, `1.2.3-rc.1` -> `(1, 2, 3, is_release)`. A pre-release
/// sorts below the release with the same core (`1.2.3-rc.1 < 1.2.3`); build
/// metadata is ignored.
fn parse_version(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split('+').next().unwrap_or(v);
    let (core, pre) = match v.split_once('-') {
        Some((core, pre)) => (core, !pre.is_empty()),
        None => (v, false),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch, !pre))
}

/// Is `latest` strictly newer than `current`? Pre-releases of the same core
/// are not ordered against each other (never "newer").
fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// `X.Y.Z` from a GitHub `releases/latest` response.
fn parse_latest_release(body: &[u8]) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("unexpected response from GitHub: {e}"))?;
    if json.get("prerelease").and_then(|v| v.as_bool()) == Some(true)
        || json.get("draft").and_then(|v| v.as_bool()) == Some(true)
    {
        return Err("the latest release is marked prerelease/draft".to_string());
    }
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            let msg = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("no tag_name in response");
            format!("GitHub API: {msg}")
        })?;
    let version = tag.trim().trim_start_matches('v').to_string();
    if parse_version(&version).is_none() {
        return Err(format!("latest release tag is not a version: {tag}"));
    }
    Ok(version)
}

/// Ask GitHub for the newest non-prerelease release through `curl` (install.sh
/// needs it anyway). `GITHUB_TOKEN` goes through curl's stdin config, never
/// argv, so it does not show up in `ps`.
fn fetch_latest_version(timeout_secs: u64) -> Result<String, String> {
    let url = std::env::var(RELEASE_API_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| LATEST_RELEASE_API.to_string());
    let token = std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty());

    let mut cmd = Command::new("curl");
    cmd.args([
        "-fsSL",
        "--max-time",
        &timeout_secs.to_string(),
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        &format!("User-Agent: {NAME}/{CURRENT_VERSION}"),
    ]);
    if token.is_some() {
        cmd.args(["-K", "-"]);
    }
    cmd.arg(&url)
        .stdin(if token.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if let (Some(token), Some(mut stdin)) = (token, child.stdin.take()) {
        let _ = writeln!(
            stdin,
            "header = \"Authorization: Bearer {}\"",
            token.trim().replace('"', "")
        );
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("curl failed: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if why.is_empty() {
            format!("could not reach {url}")
        } else {
            why
        });
    }
    parse_latest_release(&out.stdout)
}

// ---------------------------------------------------------------------------
// Daily check cache and notice
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
struct UpdateCache {
    #[serde(default)]
    checked_at: u64,
    #[serde(default)]
    latest: String,
}

/// `${XDG_CACHE_HOME:-~/.cache}/cookie-use/update-check.json`.
fn cache_path_from(xdg_cache_home: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    let base = xdg_cache_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join(NAME).join("update-check.json")
}

fn cache_path() -> PathBuf {
    cache_path_from(std::env::var_os("XDG_CACHE_HOME"), dirs::home_dir())
}

fn read_cache(path: &Path) -> UpdateCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_cache(path: &Path, cache: &UpdateCache) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Write a sibling temp file and rename it over the cache, so a concurrent
    // reader never sees a truncated file (which would read as "never checked"
    // and spawn another check).
    if let Ok(body) = serde_json::to_string(cache) {
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn check_due(cache: &UpdateCache, now: u64) -> bool {
    now.saturating_sub(cache.checked_at) >= UPDATE_CHECK_INTERVAL_SECS
}

fn update_check_disabled(get: impl Fn(&str) -> Option<OsString>) -> bool {
    CHECK_OPT_OUT_VARS.iter().any(|k| get(k).is_some())
}

fn notice_line(latest: &str, current: &str) -> Option<String> {
    is_newer(latest, current).then(|| {
        format!("{NAME} {latest} is available (you have {current}). Upgrade: {NAME} upgrade")
    })
}

fn write_notice(err: &mut dyn Write, latest: &str, current: &str) {
    if let Some(line) = notice_line(latest, current) {
        let _ = writeln!(err, "{line}");
    }
}

/// Called after argument parsing for every command except `upgrade` (clap
/// has already exited for `--version` / `--help`). Prints the notice to
/// **stderr** from the cache (unless `print` is false), and when the cache is
/// a day old refreshes it in a detached child so the command never waits on
/// the network.
pub fn maybe_notify_update(print: bool) {
    if update_check_disabled(|k| std::env::var_os(k)) {
        return;
    }
    let path = cache_path();
    let cache = read_cache(&path);
    if print {
        write_notice(&mut std::io::stderr(), &cache.latest, CURRENT_VERSION);
    }

    let now = now_secs();
    if check_due(&cache, now) {
        // Bump checked_at first: a failed or offline check is then not
        // retried on every call, and concurrent runs don't all spawn one.
        write_cache(
            &path,
            &UpdateCache {
                checked_at: now,
                latest: cache.latest.clone(),
            },
        );
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe)
                .arg(UPDATE_CHECK_CMD)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}

/// The notice alone, from the cache, with no refresh. `--json` runs print it
/// only after the command succeeded, so on failure stderr holds nothing but
/// the `{"error": ...}` envelope the GUI decodes.
pub fn print_notice() {
    if update_check_disabled(|k| std::env::var_os(k)) {
        return;
    }
    write_notice(
        &mut std::io::stderr(),
        &read_cache(&cache_path()).latest,
        CURRENT_VERSION,
    );
}

/// The detached `__update-check` child: fetch with a 2 s timeout and cache.
pub fn run_update_check() {
    if let Ok(latest) = fetch_latest_version(NOTICE_CHECK_TIMEOUT_SECS) {
        write_cache(
            &cache_path(),
            &UpdateCache {
                checked_at: now_secs(),
                latest,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Installed skills
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Channel {
    ClaudePlugin,
    Git,
    Copied,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SkillInstall {
    channel: Channel,
    path: String,
    update: String,
}

struct SkillScan {
    claude_dir: PathBuf,
    skill_dirs: Vec<PathBuf>,
}

impl SkillScan {
    fn from_env() -> Self {
        let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
        let claude_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"));
        SkillScan {
            skill_dirs: vec![
                home.join(".agents/skills"),
                claude_dir.join("skills"),
                home.join(".codex/skills"),
            ],
            claude_dir,
        }
    }
}

/// `(plugin key, install path)` when Claude Code has the plugin installed.
fn claude_plugin(claude_dir: &Path) -> Option<(String, String)> {
    let file = claude_dir.join("plugins/installed_plugins.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
    let map = json
        .get("plugins")
        .and_then(|v| v.as_object())
        .or_else(|| json.as_object())?;
    let prefix = format!("{NAME}@");
    let (key, entry) = map.iter().find(|(k, _)| k.starts_with(&prefix))?;
    let install_path = entry
        .as_array()
        .and_then(|a| a.first())
        .or(Some(entry))
        .and_then(|e| e.get("installPath"))
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| file.display().to_string());
    Some((key.clone(), install_path))
}

/// The git work tree holding `dir`, if it is a checkout of this project (its
/// origin mentions cookie-use). A skill folder inside some unrelated
/// repository, e.g. a dotfiles repo, is not ours to pull.
fn git_checkout_root(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let root = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let origin = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["remote", "get-url", "origin"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
        .unwrap_or_default();
    origin.contains(NAME).then_some(root)
}

/// Every installed copy of the skill, one entry per real location.
fn detect_skills(scan: &SkillScan) -> Vec<SkillInstall> {
    let mut found = Vec::new();
    if let Some((key, path)) = claude_plugin(&scan.claude_dir) {
        found.push(SkillInstall {
            channel: Channel::ClaudePlugin,
            path,
            update: format!("claude plugin update {key}"),
        });
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    for base in &scan.skill_dirs {
        let Ok(real) = base.join(NAME).canonicalize() else {
            continue;
        };
        if !real.is_dir() {
            continue;
        }
        if let Some(root) = git_checkout_root(&real) {
            if !seen.contains(&root) {
                seen.push(root.clone());
                found.push(SkillInstall {
                    channel: Channel::Git,
                    update: format!("git -C {} pull --ff-only", root.display()),
                    path: root.display().to_string(),
                });
            }
            continue;
        }
        if real.join("SKILL.md").is_file() && !seen.contains(&real) {
            seen.push(real.clone());
            found.push(SkillInstall {
                channel: Channel::Copied,
                path: real.display().to_string(),
                update: format!("npx skills update {NAME}"),
            });
        }
    }
    found
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
        .unwrap_or(false)
}

fn channel_label(c: Channel) -> &'static str {
    match c {
        Channel::ClaudePlugin => "claude-plugin",
        Channel::Git => "git",
        Channel::Copied => "copied",
    }
}

/// Without `--skills`: say where each copy is and how to refresh it, touch nothing.
fn list_skills(skills: &[SkillInstall]) {
    for s in skills {
        println!(
            "skill ({}) {}: not refreshed; pass --skills or run: {}",
            channel_label(s.channel),
            s.path,
            s.update
        );
    }
}

/// `--skills`: refresh cookie-use's own copies only. Returns false when one
/// that could be refreshed here failed.
fn refresh_skills(skills: &[SkillInstall]) -> bool {
    let mut ok = true;
    if skills.is_empty() {
        println!("skill: no installed copy of the cookie-use skill found");
    }
    for skill in skills {
        match skill.channel {
            Channel::Git => {
                match Command::new("git")
                    .args(["-C", &skill.path, "pull", "--ff-only", "-q"])
                    .stdin(Stdio::null())
                    // Never block on a credential prompt.
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        println!("skill (git) {}: pulled", skill.path)
                    }
                    Ok(o) => {
                        ok = false;
                        eprintln!(
                            "skill (git) {}: not updated (local changes or diverged?), not forcing: {}",
                            skill.path,
                            String::from_utf8_lossy(&o.stderr).trim()
                        )
                    }
                    Err(e) => {
                        ok = false;
                        eprintln!(
                            "skill (git) {}: not updated, could not run git: {e}",
                            skill.path
                        )
                    }
                }
            }
            Channel::ClaudePlugin => {
                println!("skill (claude-plugin): {}", skill.update);
                if !on_path("claude") {
                    println!("skill (claude-plugin): claude is not on PATH; run the command above");
                    continue;
                }
                let done = Command::new("claude")
                    .args(["plugin", "update", PLUGIN_ID])
                    .stdin(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if done {
                    println!("skill (claude-plugin): updated; restart Claude Code or run /reload-plugins");
                } else {
                    ok = false;
                    eprintln!(
                        "skill (claude-plugin): `{}` failed; run it yourself",
                        skill.update
                    );
                }
            }
            // Copied folders may carry local edits and belong to the skills
            // tool that made them; never re-copy behind the user's back.
            Channel::Copied => println!(
                "skill (copied) {}: run `{}` to refresh it",
                skill.path, skill.update
            ),
        }
    }
    ok
}

// ---------------------------------------------------------------------------
// Install channel
// ---------------------------------------------------------------------------

/// How the running binary got here. Only a release binary (install.sh or an
/// unpacked tarball) is upgraded in place; anything a package manager or a
/// build owns is left to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct InstallChannel {
    channel: &'static str,
    path: String,
    upgradable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

fn install_channel(exe: &Path, cargo_bin: Option<&Path>) -> InstallChannel {
    let path = exe.display().to_string();
    let dir = exe.parent().unwrap_or_else(|| Path::new("/"));
    let comps: Vec<String> = dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mk = |channel, hint: Option<String>| InstallChannel {
        channel,
        path: path.clone(),
        upgradable: hint.is_none(),
        hint,
    };
    if comps.iter().any(|c| c == "Cellar")
        || path.starts_with("/opt/homebrew/")
        || path.starts_with("/home/linuxbrew/")
    {
        return mk("brew", Some(format!("brew upgrade {NAME}")));
    }
    if comps.iter().any(|c| c == "node_modules") {
        return mk(
            "npm",
            Some(format!(
                "installed by npm; upgrade it with npm (e.g. npm update -g {NAME})"
            )),
        );
    }
    let in_target = comps.len() >= 2
        && comps[..comps.len() - 1].iter().any(|c| c == "target")
        && matches!(
            comps.last().map(String::as_str),
            Some("debug" | "release" | "deps")
        );
    if in_target {
        return mk(
            "source",
            Some("a source build; `git pull` the checkout and `cargo build --release`".to_string()),
        );
    }
    if cargo_bin.is_some_and(|c| dir == c) {
        return mk(
            "cargo",
            Some(format!(
                "cargo install --locked --force --git https://github.com/leeguooooo/{NAME}"
            )),
        );
    }
    mk("release", None)
}

fn cargo_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
        .map(|c| c.join("bin"))
        .and_then(|p| p.canonicalize().ok())
}

/// The binary `upgrade` replaces: this one. `COOKIE_USE_UPGRADE_EXE` points
/// it elsewhere for tests (a temp install), so no test touches a real binary.
fn running_exe() -> PathBuf {
    if let Some(p) = std::env::var_os("COOKIE_USE_UPGRADE_EXE").filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .map(|p| p.canonicalize().unwrap_or(p))
        .unwrap_or_else(|_| PathBuf::from(NAME))
}

/// The CookieUse.app GUI is its own install (install-app.sh) with its own
/// version; `upgrade` reports it but never touches it.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct AppInstall {
    path: String,
    version: Option<String>,
    update: String,
}

const APP_INSTALL: &str =
    "curl -fsSL https://raw.githubusercontent.com/leeguooooo/cookie-use/main/install-app.sh | sh";

fn detect_app() -> Option<AppInstall> {
    let app = std::env::var_os("COOKIE_USE_APP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Applications/CookieUse.app"));
    let plist = app.join("Contents/Info.plist");
    if !plist.is_file() {
        return None;
    }
    let version = Command::new("plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(&plist)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty());
    Some(AppInstall {
        path: app.display().to_string(),
        version,
        update: APP_INSTALL.to_string(),
    })
}

// ---------------------------------------------------------------------------
// `upgrade`
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CheckReport {
    name: &'static str,
    current: String,
    latest: Option<String>,
    update_available: bool,
    skills: Vec<SkillInstall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    install_channel: Option<InstallChannel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<AppInstall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn build_report(
    current: &str,
    latest: Result<String, String>,
    skills: Vec<SkillInstall>,
) -> CheckReport {
    let (latest, error) = match latest {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e)),
    };
    CheckReport {
        name: NAME,
        current: current.to_string(),
        update_available: latest.as_deref().is_some_and(|l| is_newer(l, current)),
        latest,
        skills,
        target: None,
        install_channel: None,
        app: None,
        error,
    }
}

fn check_line(report: &CheckReport) -> String {
    if let Some(target) = &report.target {
        return if target == &report.current {
            format!("{NAME} {} is already {target}", report.current)
        } else {
            format!("{NAME} {} -> {target} (pinned)", report.current)
        };
    }
    match &report.latest {
        Some(latest) if report.update_available => {
            format!("{NAME} {} -> {latest}", report.current)
        }
        Some(_) => format!("{NAME} {} is up to date", report.current),
        None => format!(
            "{NAME} {}: could not check the latest release: {}",
            report.current,
            report.error.as_deref().unwrap_or("unknown error")
        ),
    }
}

fn record_latest(latest: &str) {
    write_cache(
        &cache_path(),
        &UpdateCache {
            checked_at: now_secs(),
            latest: latest.to_string(),
        },
    );
}

/// `--tag` value -> bare `X.Y.Z`. Only a plain release version is accepted: it
/// ends up in a download URL and an environment variable.
fn parse_tag(tag: &str) -> Result<String, String> {
    let bare = tag.trim().trim_start_matches('v');
    let mut parts = bare.split('.');
    let ok = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    }) && parts.next().is_none();
    if ok {
        Ok(bare.to_string())
    } else {
        Err(format!("--tag must look like v1.2.3 (got {tag})"))
    }
}

/// The version a binary reports (`cookie-use X.Y.Z`).
fn installed_version(exe: &Path) -> Option<String> {
    let out = Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_string)
}

/// Runs install.sh (downloaded to a temp file first, so a truncated download
/// cannot be run as a partial script) into `dir`, pinned to `version` when known.
fn run_installer(dir: &Path, version: Option<&str>) -> Result<(), String> {
    let url = std::env::var("COOKIE_USE_INSTALL_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| INSTALL_URL.to_string());
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg("set -eu; t=$(mktemp); trap 'rm -f \"$t\"' EXIT; curl -fsSL \"$1\" -o \"$t\"; sh \"$t\"")
        .arg("sh")
        .arg(&url)
        .env("COOKIE_USE_BIN_DIR", dir)
        .stdin(Stdio::null());
    match version {
        Some(v) => cmd.env("COOKIE_USE_VERSION", format!("v{v}")),
        None => cmd.env_remove("COOKIE_USE_VERSION"),
    };
    let status = cmd.status().map_err(|e| format!("could not run sh: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("install.sh failed ({status})"))
    }
}

/// The skill and app part of a real (non-check) upgrade, then exit. A skill
/// refresh that was asked for and failed turns a 0 into 1.
fn finish(skills: &[SkillInstall], opt_in: bool, app: Option<&AppInstall>, code: i32) -> ! {
    let skills_ok = if opt_in {
        refresh_skills(skills)
    } else {
        list_skills(skills);
        true
    };
    if let Some(a) = app {
        println!(
            "app {} {}: not touched by upgrade (separate install); update with: {}",
            a.path,
            a.version.as_deref().unwrap_or("?"),
            a.update
        );
    }
    exit(if code == 0 && !skills_ok { 1 } else { code });
}

/// `cookie-use upgrade [--check] [--json] [--skills] [--tag vX.Y.Z]`. Exits
/// the process: 0 on success (upgraded, already current, a check that ran),
/// 2 when the check, download or verification failed, 1 when the upgrade was
/// refused (package-manager/source install) or did not finish.
///
/// Never opens the vault, reads the key, touches the Keychain, or talks to a
/// browser: the only state it writes is the binary and the update-check cache.
pub fn run_upgrade(check: bool, json: bool, skills_opt_in: bool, tag: Option<String>) -> ! {
    let pinned = match tag.as_deref().map(parse_tag).transpose() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{NAME} upgrade: {e}");
            exit(2);
        }
    };
    let skills = detect_skills(&SkillScan::from_env());
    let exe = running_exe();
    let channel = install_channel(&exe, cargo_bin().as_deref());
    let app = detect_app();
    let latest = fetch_latest_version(EXPLICIT_CHECK_TIMEOUT_SECS);
    if let Ok(v) = &latest {
        record_latest(v);
    }

    if check || json {
        let mut report = build_report(CURRENT_VERSION, latest, skills);
        report.target = pinned.clone();
        report.install_channel = Some(channel);
        report.app = app;
        // A pinned target does not need the latest-release lookup.
        let failed = report.error.is_some() && report.target.is_none();
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        } else {
            if failed {
                eprintln!("{}", check_line(&report));
            } else {
                println!("{}", check_line(&report));
            }
            if let Some(c) = &report.install_channel {
                if let Some(hint) = &c.hint {
                    println!(
                        "  installed via {} ({}): upgrade with {hint}",
                        c.channel, c.path
                    );
                }
            }
            for s in &report.skills {
                println!(
                    "  skill ({}) {}  refresh: {}",
                    channel_label(s.channel),
                    s.path,
                    s.update
                );
            }
            if let Some(a) = &report.app {
                println!(
                    "  app {} {}  (separate install; update: {})",
                    a.path,
                    a.version.as_deref().unwrap_or("?"),
                    a.update
                );
            }
        }
        exit(if failed { 2 } else { 0 });
    }

    // What to install: the pinned tag, else the latest release when newer.
    let target: Option<String> = match (&pinned, &latest) {
        (Some(t), _) => (t != CURRENT_VERSION).then(|| t.clone()),
        (None, Ok(v)) => is_newer(v, CURRENT_VERSION).then(|| v.clone()),
        // install.sh downloads through releases/latest/download, which is not
        // rate-limited like the API, so a failed check need not stop us; the
        // exact-version check is then skipped (the checksum still applies).
        (None, Err(e)) => {
            eprintln!("cli: could not check the latest release ({e}); installing the latest release anyway");
            Some(String::new())
        }
    };

    let Some(target) = target else {
        match &pinned {
            Some(t) => println!("cli: {NAME} {CURRENT_VERSION} is already {t}"),
            None => println!("cli: {NAME} {CURRENT_VERSION} is up to date"),
        }
        finish(&skills, skills_opt_in, app.as_ref(), 0);
    };

    if !channel.upgradable {
        eprintln!(
            "cli: {} is installed via {}; not replacing it. Upgrade with: {}",
            channel.path,
            channel.channel,
            channel.hint.as_deref().unwrap_or("its package manager")
        );
        finish(&skills, skills_opt_in, app.as_ref(), 1);
    }

    let pin = (!target.is_empty()).then_some(target.as_str());
    match pin {
        Some(v) => println!("cli: upgrading {NAME} {CURRENT_VERSION} -> {v}"),
        None => println!("cli: reinstalling the latest {NAME} release"),
    }
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    if let Err(e) = run_installer(dir, pin) {
        eprintln!(
            "cli: upgrade failed ({e}); {} was left as it was. Install manually:\n  curl -fsSL {INSTALL_URL} | sh",
            exe.display()
        );
        exit(2);
    }
    let now = installed_version(&exe).unwrap_or_else(|| "unknown".to_string());
    if pin.is_some_and(|v| v != now) {
        eprintln!(
            "cli: install.sh finished but {} reports {now}, expected {}",
            exe.display(),
            target
        );
        finish(&skills, skills_opt_in, app.as_ref(), 1);
    }
    if now == CURRENT_VERSION {
        println!("cli: {NAME} {now} (unchanged)");
    } else {
        println!("cli: {NAME} {CURRENT_VERSION} -> {now}");
    }
    finish(&skills, skills_opt_in, app.as_ref(), 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(vars: &[&str]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|k| (k.to_string(), OsString::from("1")))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.3.1", "0.3.0"));
        assert!(is_newer("v0.4.0", "0.3.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.3.10", "0.3.9"), "numeric, not lexical");
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("0.3.0-rc.1", "0.3.0"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(
            is_newer("0.3.0", "0.3.0-rc.1"),
            "release beats its pre-release"
        );
        assert!(is_newer("0.3.1-rc.1", "0.3.0"));
        assert!(!is_newer("0.3.0-rc.2", "0.3.0-rc.1"));
        assert!(!is_newer("0.3.0+build.5", "0.3.0"));
        assert!(!is_newer("", "0.3.0"));
        assert!(!is_newer("garbage", "0.3.0"));
    }

    #[test]
    fn latest_release_parsing() {
        assert_eq!(
            parse_latest_release(br#"{"tag_name":"v0.3.1","prerelease":false}"#).unwrap(),
            "0.3.1"
        );
        assert!(parse_latest_release(br#"{"tag_name":"v9.0.0","prerelease":true}"#).is_err());
        assert!(
            parse_latest_release(br#"{"message":"API rate limit exceeded"}"#)
                .unwrap_err()
                .contains("rate limit")
        );
        assert!(parse_latest_release(b"<html>").is_err());
    }

    #[test]
    fn check_is_throttled_to_once_a_day() {
        let now = 1_900_000_000;
        let at = |checked_at| UpdateCache {
            checked_at,
            latest: "0.3.0".into(),
        };
        assert!(check_due(&UpdateCache::default(), now));
        assert!(!check_due(&at(now), now));
        assert!(!check_due(&at(now - 86_399), now));
        assert!(check_due(&at(now - 86_400), now));
        assert!(!check_due(&at(now + 60), now), "clock went backwards");
    }

    #[test]
    fn cache_file_location_and_format() {
        let dir = std::env::temp_dir().join(format!("cookie-use-ut-{}", std::process::id()));
        let path = cache_path_from(Some(dir.clone().into()), None);
        assert_eq!(path, dir.join("cookie-use/update-check.json"));
        let c = UpdateCache {
            checked_at: 7,
            latest: "9.9.9".into(),
        };
        write_cache(&path, &c);
        assert_eq!(read_cache(&path), c);
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw, serde_json::json!({"checked_at": 7, "latest": "9.9.9"}));
        let _ = std::fs::remove_dir_all(&dir);

        let home = PathBuf::from("/home/someone");
        assert_eq!(
            cache_path_from(None, Some(home.clone())),
            home.join(".cache/cookie-use/update-check.json")
        );
        assert_eq!(
            cache_path_from(Some("relative".into()), Some(home.clone())),
            home.join(".cache/cookie-use/update-check.json")
        );
    }

    #[test]
    fn opt_out_env_vars() {
        assert!(!update_check_disabled(env_of(&[])));
        for var in ["CI", "COOKIE_USE_NO_UPDATE_CHECK", "USE_NO_UPDATE_CHECK"] {
            assert!(update_check_disabled(env_of(&[var])), "{var}");
        }
        assert!(!update_check_disabled(env_of(&[
            "CHROME_USE_NO_UPDATE_CHECK"
        ])));
    }

    #[test]
    fn notice_is_the_exact_line_only_when_newer() {
        let mut err = Vec::new();
        write_notice(&mut err, "0.3.1", "0.3.0");
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "cookie-use 0.3.1 is available (you have 0.3.0). Upgrade: cookie-use upgrade\n"
        );
        let mut err = Vec::new();
        write_notice(&mut err, "0.3.0", "0.3.0");
        write_notice(&mut err, "", "0.3.0");
        assert!(err.is_empty());
    }

    #[test]
    fn json_report_shape() {
        let report = build_report(
            "0.3.0",
            Ok("0.3.1".into()),
            vec![SkillInstall {
                channel: Channel::ClaudePlugin,
                path: "/placeholder".into(),
                update: "claude plugin update cookie-use@leeguooooo-plugins".into(),
            }],
        );
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "name": "cookie-use",
                "current": "0.3.0",
                "latest": "0.3.1",
                "update_available": true,
                "skills": [{
                    "channel": "claude-plugin",
                    "path": "/placeholder",
                    "update": "claude plugin update cookie-use@leeguooooo-plugins"
                }]
            })
        );
        assert_eq!(check_line(&report), "cookie-use 0.3.0 -> 0.3.1");
        let current = build_report("0.3.1", Ok("0.3.1".into()), vec![]);
        assert_eq!(check_line(&current), "cookie-use 0.3.1 is up to date");
        let failed =
            serde_json::to_value(build_report("0.3.0", Err("offline".into()), vec![])).unwrap();
        assert_eq!(failed["latest"], serde_json::Value::Null);
        assert_eq!(failed["update_available"], false);
        assert_eq!(failed["error"], "offline");
    }

    #[test]
    fn detects_plugin_git_and_copied_skills() {
        let tmp = std::env::temp_dir().join(format!("cookie-use-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let root = tmp.canonicalize().unwrap();
        let claude = root.join(".claude");
        std::fs::create_dir_all(claude.join("plugins")).unwrap();
        std::fs::write(
            claude.join("plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"cookie-use@leeguooooo-plugins":[{"installPath":"/placeholder"}]}}"#,
        )
        .unwrap();

        // Copied folder plus a symlink to it: one install.
        let agents = root.join(".agents/skills");
        std::fs::create_dir_all(agents.join("cookie-use")).unwrap();
        std::fs::write(agents.join("cookie-use/SKILL.md"), "x").unwrap();
        std::fs::create_dir_all(claude.join("skills")).unwrap();
        std::os::unix::fs::symlink(agents.join("cookie-use"), claude.join("skills/cookie-use"))
            .unwrap();

        // A cookie-use git checkout linked in.
        let checkout = root.join("checkout");
        let codex = root.join(".codex/skills");
        std::fs::create_dir_all(&codex).unwrap();
        let git_ok = Command::new("git")
            .args(["init", "-q"])
            .arg(&checkout)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if git_ok {
            Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://example.com/owner/cookie-use.git",
                ])
                .status()
                .unwrap();
            std::fs::create_dir_all(checkout.join("skills/cookie-use")).unwrap();
            std::fs::write(checkout.join("skills/cookie-use/SKILL.md"), "x").unwrap();
            std::os::unix::fs::symlink(
                checkout.join("skills/cookie-use"),
                codex.join("cookie-use"),
            )
            .unwrap();
        }

        let skills = detect_skills(&SkillScan {
            claude_dir: claude.clone(),
            skill_dirs: vec![agents, claude.join("skills"), codex],
        });
        let of = |c: Channel| skills.iter().filter(|s| s.channel == c).count();
        assert_eq!(of(Channel::ClaudePlugin), 1, "{skills:?}");
        assert_eq!(of(Channel::Copied), 1, "{skills:?}");
        if git_ok {
            assert_eq!(of(Channel::Git), 1, "{skills:?}");
            let git = skills.iter().find(|s| s.channel == Channel::Git).unwrap();
            assert_eq!(git.path, checkout.display().to_string());
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn install_channel_detection() {
        let cargo = PathBuf::from("/Users/u/.cargo/bin");
        let c = |p: &str| install_channel(Path::new(p), Some(&cargo));
        let release = c("/Users/u/.local/bin/cookie-use");
        assert_eq!(release.channel, "release");
        assert!(release.upgradable && release.hint.is_none());
        assert_eq!(c("/usr/local/bin/cookie-use").channel, "release");
        for brew in [
            "/opt/homebrew/Cellar/cookie-use/0.4.0/bin/cookie-use",
            "/usr/local/Cellar/cookie-use/0.4.0/bin/cookie-use",
            "/opt/homebrew/bin/cookie-use",
        ] {
            let b = c(brew);
            assert_eq!(b.channel, "brew", "{brew}");
            assert!(!b.upgradable);
            assert_eq!(b.hint.as_deref(), Some("brew upgrade cookie-use"));
        }
        assert_eq!(c("/Users/u/.cargo/bin/cookie-use").channel, "cargo");
        assert!(!c("/Users/u/.cargo/bin/cookie-use").upgradable);
        assert_eq!(
            c("/src/cookie-use/target/release/cookie-use").channel,
            "source"
        );
        assert_eq!(
            c("/src/cookie-use/target/debug/deps/cookie-use-1a2b").channel,
            "source"
        );
        assert_eq!(
            c("/usr/local/lib/node_modules/cookie-use/bin/cookie-use").channel,
            "npm"
        );
        // A directory merely named "release" is not a build tree.
        assert_eq!(c("/opt/release/cookie-use").channel, "release");
        assert_eq!(
            install_channel(Path::new("/Users/u/.cargo/bin/cookie-use"), None).channel,
            "release"
        );
    }

    #[test]
    fn tag_parsing() {
        assert_eq!(parse_tag("v0.4.0").unwrap(), "0.4.0");
        assert_eq!(parse_tag("0.10.2").unwrap(), "0.10.2");
        for bad in [
            "",
            "v1.2",
            "1.2.3.4",
            "1.2.3-rc.1",
            "latest",
            "v1.2.3; rm -rf /",
            "1..3",
        ] {
            assert!(parse_tag(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pinned_check_line() {
        let mut r = build_report("0.4.0", Ok("0.5.0".into()), vec![]);
        r.target = Some("0.3.0".into());
        assert_eq!(check_line(&r), "cookie-use 0.4.0 -> 0.3.0 (pinned)");
        r.target = Some("0.4.0".into());
        assert_eq!(check_line(&r), "cookie-use 0.4.0 is already 0.4.0");
    }
}
