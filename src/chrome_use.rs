//! Thin wrapper around the `chrome-use` CLI. cookie-use delegates every browser
//! and cookie operation here, so it never re-implements decryption or CDP.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};
use std::io::Write;
use std::process::Command;

fn bin() -> String {
    std::env::var("CHROME_USE_BIN").unwrap_or_else(|_| "chrome-use".to_string())
}

/// Where an account's cookies get applied.
pub enum Target {
    /// An existing chrome-use session (connected via the extension or a prior
    /// launch). This is the default and the most reliable path.
    Session(String),
    /// A fresh, throwaway isolated browser (`chrome-use --launch`).
    Isolated,
    /// One specific connected Chrome profile, by account email (or relay id):
    /// `chrome-use --browser <sel>`. Used to write into a *chosen* profile
    /// (copy between profiles) rather than whichever window has focus.
    Browser(String),
}

impl Target {
    pub fn parse(s: &str) -> Result<Target> {
        if s == "isolated" {
            Ok(Target::Isolated)
        } else if let Some(name) = s.strip_prefix("session:") {
            Ok(Target::Session(name.to_string()))
        } else if let Some(sel) = s.strip_prefix("browser:") {
            Ok(Target::Browser(sel.to_string()))
        } else if s.strip_prefix("profile:").is_some() {
            Err(anyhow!(
                "profile: targets aren't in v0.1 — connect chrome-use's extension to that \
                 profile and use `--target session:<name>` instead"
            ))
        } else {
            Err(anyhow!(
                "unknown target \"{}\" (use session:<name>, browser:<email> or isolated)",
                s
            ))
        }
    }

    /// The concrete chrome-use session name this target resolves to. Mirrors the
    /// resolution in [`apply`] (an Isolated target uses the fixed throwaway
    /// session name). Used for machine-readable (`--json`) output.
    pub fn session_name(&self) -> String {
        match self {
            Target::Session(name) => name.clone(),
            Target::Isolated => "cookie-use-iso".to_string(),
            // chrome-use pins a session to a profile on its first connect and
            // keeps it, so each profile gets its own session name.
            Target::Browser(sel) => format!("cookie-use-to-{}", slug(sel)),
        }
    }

    /// Leading chrome-use args that address this target.
    fn args(&self) -> Vec<String> {
        let mut a = Vec::new();
        if let Target::Browser(sel) = self {
            a.extend(["--browser".to_string(), sel.clone()]);
        }
        a.extend(["--session".to_string(), self.session_name()]);
        a
    }
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Run chrome-use against `target` with extra args.
fn run_on(target: &Target, rest: &[&str]) -> Result<()> {
    let mut args = target.args();
    args.extend(rest.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&refs)
}

/// Options controlling how an account is applied to a target.
#[derive(Default)]
pub struct ApplyOpts<'a> {
    /// Rewrite every cookie's domain to this host before injecting (e.g.
    /// "localhost"), so a session captured on one origin can be reused on
    /// another for local/cross-origin testing.
    pub rewrite_domain: Option<&'a str>,
    /// URL to open after the cookies are set. Skipped when None.
    pub open_url: Option<&'a str>,
    /// localStorage items to inject into the opened origin (requires open_url).
    pub local_storage: Option<&'a Map<String, Value>>,
}

/// A local Chrome profile as Chrome's `Local State` describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalProfile {
    /// Directory name ("Default", "Profile 3") — what `cookies export --from` takes.
    pub dir: String,
    /// Display name shown in Chrome's profile menu.
    pub name: String,
    /// Signed-in Google account, if any — what `--browser` pins to.
    pub email: Option<String>,
}

fn chrome_user_data_dir() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("COOKIE_USE_CHROME_DIR") {
        return Some(p.into());
    }
    dirs::home_dir().map(|h| h.join("Library/Application Support/Google/Chrome"))
}

/// Every profile listed in Chrome's `Local State`.
pub fn local_profiles() -> Result<Vec<LocalProfile>> {
    let path = chrome_user_data_dir()
        .ok_or_else(|| anyhow!("could not find the Chrome data directory"))?
        .join("Local State");
    let raw = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    parse_local_state(&raw)
}

fn parse_local_state(raw: &[u8]) -> Result<Vec<LocalProfile>> {
    let v: Value = serde_json::from_slice(raw).context("parsing Chrome Local State")?;
    let cache = v
        .pointer("/profile/info_cache")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("Chrome Local State has no profile list"))?;
    let mut out: Vec<LocalProfile> = cache
        .iter()
        .map(|(dir, info)| LocalProfile {
            dir: dir.clone(),
            name: info
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(dir)
                .to_string(),
            email: info
                .get("user_name")
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
                .map(String::from),
        })
        .collect();
    out.sort_by(|a, b| (a.dir != "Default", &a.dir).cmp(&(b.dir != "Default", &b.dir)));
    Ok(out)
}

/// Resolve a user-supplied profile (directory, display name or email).
/// Display names repeat ("wind" ×3), so an ambiguous name is an error that
/// lists the directories to pick from.
pub fn resolve_profile(sel: &str) -> Result<LocalProfile> {
    let all = local_profiles()?;
    pick_profile(&all, sel)
}

fn pick_profile(all: &[LocalProfile], sel: &str) -> Result<LocalProfile> {
    let s = sel.trim();
    if let Some(p) = all.iter().find(|p| p.dir.eq_ignore_ascii_case(s)) {
        return Ok(p.clone());
    }
    let hits: Vec<&LocalProfile> = all
        .iter()
        .filter(|p| {
            p.name.eq_ignore_ascii_case(s)
                || p.email
                    .as_deref()
                    .map(|e| e.eq_ignore_ascii_case(s))
                    .unwrap_or(false)
        })
        .collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(anyhow!(
            "no Chrome profile \"{s}\" (have: {})",
            all.iter()
                .map(|p| format!("{} ({})", p.name, p.dir))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        many => Err(anyhow!(
            "\"{s}\" matches {} profiles — use the directory: {}",
            many.len(),
            many.iter()
                .map(|p| p.dir.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Export a profile's decrypted cookies for the given site(s) via chrome-use.
pub fn export_from_profile(profile: &str, site: &str) -> Result<Vec<Value>> {
    let out = Command::new(bin())
        .args([
            "cookies", "export", "--from", profile, "--domain", site, "--json",
        ])
        .output()
        .with_context(|| "running `chrome-use cookies export` (is chrome-use installed?)")?;
    if !out.status.success() {
        return Err(anyhow!(
            "chrome-use cookies export failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    parse_cookies_json(&out.stdout)
}

/// Capture localStorage for `url` using a throwaway isolated browser seeded with
/// `cookies`. One in-browser read: launch, seed cookies, open the origin, read
/// localStorage, close. Returns an empty map if the origin has no localStorage.
pub fn capture_local_storage(cookies: &[Value], url: &str) -> Result<Map<String, Value>> {
    let session = "cookie-use-capture";
    let tmp = write_temp_cookies(cookies)?;
    let path = tmp.to_string_lossy().to_string();

    let result = (|| -> Result<Map<String, Value>> {
        run(&["--session", session, "--launch", "open", "about:blank"])?;
        run(&["--session", session, "cookies", "set", "--curl", &path])?;
        run(&["--session", session, "open", url])?;
        let out = capture(&["--session", session, "storage", "local", "get", "--json"])?;
        parse_local_storage_json(&out)
    })();

    // Always tear the throwaway session down, even on error.
    let _ = run(&["--session", session, "close"]);
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Apply a cookie set to a target. Returns once chrome-use confirms.
pub fn apply(cookies: &[Value], target: &Target, opts: &ApplyOpts) -> Result<()> {
    let cookies: Vec<Value> = match opts.rewrite_domain {
        Some(host) => rewrite_cookie_domains(cookies, host),
        None => cookies.to_vec(),
    };
    let tmp = write_temp_cookies(&cookies)?;
    let path = tmp.to_string_lossy().to_string();

    // An isolated target starts a throwaway browser first.
    if let Target::Isolated = target {
        run_on(target, &["--launch", "open", "about:blank"])?;
    }

    run_on(target, &["cookies", "set", "--curl", &path])?;
    if let Some(url) = opts.open_url {
        run_on(target, &["open", url])?;
        // localStorage is origin-scoped, so it can only be injected once we're
        // on the opened page. Reload afterwards so the app reads it on boot.
        if let Some(items) = opts.local_storage.filter(|m| !m.is_empty()) {
            inject_local_storage(target, items)?;
            let _ = run_on(target, &["reload"]);
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

/// Launch a fresh isolated browser under an explicit session name and apply
/// `cookies`, opening `open_url`. Unlike `Target::Isolated` (one fixed session),
/// this lets callers run several isolated accounts side by side — each a
/// distinct, named throwaway session. Used by `run --all`.
pub fn apply_isolated_named(
    cookies: &[Value],
    session: &str,
    open_url: &str,
    local_storage: Option<&Map<String, Value>>,
) -> Result<()> {
    run(&["--session", session, "--launch", "open", "about:blank"])?;
    let opts = ApplyOpts {
        rewrite_domain: None,
        open_url: Some(open_url),
        local_storage,
    };
    apply(cookies, &Target::Session(session.to_string()), &opts)
}

/// Sign a site out of a session target before a clean switch (used by `switch`).
///
/// Never calls `chrome-use cookies clear`: that is CDP
/// `Network.clearBrowserCookies`, which wipes *every* site in the user's real
/// Chrome. Instead we expire exactly the cookies in `known` (name + domain +
/// path, as previously stored for this site) by re-setting each with a past
/// expiry, which Chrome treats as a delete. Isolated targets start empty.
pub fn clear_site(target: &Target, known: &[Value]) -> Result<()> {
    if let Target::Isolated = target {
        return Ok(());
    }
    let tombstones = tombstones(known);
    if tombstones.is_empty() {
        return Ok(());
    }
    let tmp = write_temp_cookies(&tombstones)?;
    let path = tmp.to_string_lossy().to_string();
    let result = run_on(target, &["cookies", "set", "--curl", &path]);
    let _ = std::fs::remove_file(&tmp);
    result
}

/// One expired copy per distinct (name, domain, path), keeping the original
/// flags so Secure / SameSite / partitioned cookies are actually overwritten.
fn tombstones(cookies: &[Value]) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    cookies
        .iter()
        .filter_map(|c| {
            let obj = c.as_object()?;
            let key = (
                obj.get("name")?.as_str()?.to_string(),
                obj.get("domain")?.as_str()?.to_string(),
                obj.get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .to_string(),
            );
            if !seen.insert(key) {
                return None;
            }
            let mut t = obj.clone();
            t.insert("value".into(), json!(""));
            t.insert("expires".into(), json!(1));
            t.remove("session");
            t.remove("size");
            Some(Value::Object(t))
        })
        .collect()
}

/// Return a copy of `cookies` with every `domain` rewritten to `host`. Used by
/// `--rewrite-domain` so a session captured on one origin can be replayed on
/// another (e.g. a production `.example.com` token reused on `localhost`).
pub fn rewrite_cookie_domains(cookies: &[Value], host: &str) -> Vec<Value> {
    cookies
        .iter()
        .map(|c| {
            let mut c = c.clone();
            if let Some(obj) = c.as_object_mut() {
                // Plain host, no leading dot: `localhost` has no subdomains and a
                // leading-dot domain is rejected there. `Secure` cookies still
                // apply on localhost (a secure context) over http, so leave it.
                obj.insert("domain".into(), json!(host));
            }
            c
        })
        .collect()
}

fn inject_local_storage(target: &Target, items: &Map<String, Value>) -> Result<()> {
    for (k, v) in items {
        // localStorage values are always strings; unwrap JSON strings so we
        // don't double-quote them, and stringify anything else defensively.
        let val = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        run_on(target, &["storage", "local", "set", k, &val])?;
    }
    Ok(())
}

fn run(args: &[&str]) -> Result<()> {
    capture(args).map(|_| ())
}

/// Run a chrome-use command and return its stdout.
fn capture(args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new(bin())
        .args(args)
        .output()
        .with_context(|| format!("running `chrome-use {}`", args.join(" ")))?;
    if !out.status.success() {
        return Err(anyhow!(
            "chrome-use {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn parse_cookies_json(stdout: &[u8]) -> Result<Vec<Value>> {
    let v: Value = serde_json::from_slice(stdout).context("parsing chrome-use output")?;
    // chrome-use --json wraps as {success, data:[...]}; tolerate a bare array too.
    let arr = v.get("data").unwrap_or(&v);
    arr.as_array()
        .cloned()
        .ok_or_else(|| anyhow!("unexpected chrome-use output shape"))
}

/// Parse `storage local get --json` into a flat key->value map. Tolerates both
/// an object payload ({"k":"v"}) and an array of {key,value} pairs, with or
/// without the {success, data} envelope.
fn parse_local_storage_json(stdout: &[u8]) -> Result<Map<String, Value>> {
    let trimmed = stdout.iter().all(u8::is_ascii_whitespace);
    if trimmed {
        return Ok(Map::new());
    }
    let v: Value = serde_json::from_slice(stdout).context("parsing localStorage output")?;
    let payload = v.get("data").unwrap_or(&v);
    match payload {
        Value::Object(map) => Ok(map.clone()),
        Value::Array(items) => {
            let mut out = Map::new();
            for item in items {
                if let (Some(k), Some(val)) = (item.get("key"), item.get("value")) {
                    if let Some(k) = k.as_str() {
                        out.insert(k.to_string(), val.clone());
                    }
                }
            }
            Ok(out)
        }
        Value::Null => Ok(Map::new()),
        _ => Err(anyhow!("unexpected localStorage output shape")),
    }
}

/// Write cookies to a private temp file for `cookies set --curl`.
fn write_temp_cookies(cookies: &[Value]) -> Result<std::path::PathBuf> {
    let mut path = std::env::temp_dir();
    let nonce: u64 = rand::random();
    path.push(format!("cookie-use-{nonce:x}.json"));
    let json = serde_json::to_vec(cookies)?;
    let mut f = std::fs::File::create(&path).context("creating temp cookie file")?;
    f.write_all(&json).context("writing temp cookie file")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_every_cookie_domain() {
        let cookies = vec![
            json!({"name": "token", "value": "a", "domain": ".pwtk.cc", "path": "/"}),
            json!({"name": "sid", "value": "b", "domain": "sg-git.pwtk.cc", "path": "/"}),
        ];
        let out = rewrite_cookie_domains(&cookies, "localhost");
        assert_eq!(out.len(), 2);
        for c in &out {
            assert_eq!(c["domain"], json!("localhost"));
        }
        // Non-domain fields are preserved.
        assert_eq!(out[0]["value"], json!("a"));
        assert_eq!(out[1]["name"], json!("sid"));
    }

    #[test]
    fn local_state_profiles_and_ambiguous_names() {
        let raw = br#"{"profile":{"info_cache":{
            "Profile 7":{"name":"wind","user_name":"a@x.com"},
            "Default":{"name":"Leo","user_name":"leo@x.com"},
            "Profile 9":{"name":"wind","user_name":""}}}}"#;
        let all = parse_local_state(raw).unwrap();
        assert_eq!(all[0].dir, "Default");
        assert_eq!(all[2].email, None);
        assert_eq!(pick_profile(&all, "leo").unwrap().dir, "Default");
        assert_eq!(pick_profile(&all, "A@X.com").unwrap().dir, "Profile 7");
        assert_eq!(pick_profile(&all, "profile 9").unwrap().name, "wind");
        let err = pick_profile(&all, "wind").unwrap_err().to_string();
        assert!(
            err.contains("Profile 7") && err.contains("Profile 9"),
            "{err}"
        );
    }

    #[test]
    fn tombstones_expire_each_distinct_cookie_once() {
        let cookies = vec![
            json!({"name": "sid", "value": "a", "domain": ".x.com", "path": "/", "secure": true, "session": false, "size": 4}),
            json!({"name": "sid", "value": "b", "domain": ".x.com", "path": "/"}),
            json!({"name": "sid", "value": "c", "domain": "app.x.com", "path": "/"}),
            json!({"value": "no-name"}),
        ];
        let t = tombstones(&cookies);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0]["value"], json!(""));
        assert_eq!(t[0]["expires"], json!(1));
        assert_eq!(t[0]["secure"], json!(true));
        assert!(t[0].get("session").is_none() && t[0].get("size").is_none());
        assert_eq!(t[1]["domain"], json!("app.x.com"));
    }

    #[test]
    fn parses_local_storage_object_payload() {
        let raw = br#"{"success":true,"data":{"token":"xyz","kolUser":"{\"id\":1}"}}"#;
        let map = parse_local_storage_json(raw).unwrap();
        assert_eq!(map.get("token"), Some(&json!("xyz")));
        assert_eq!(map.get("kolUser"), Some(&json!("{\"id\":1}")));
    }

    #[test]
    fn parses_local_storage_array_payload() {
        let raw = br#"{"data":[{"key":"a","value":"1"},{"key":"b","value":"2"}]}"#;
        let map = parse_local_storage_json(raw).unwrap();
        assert_eq!(map.get("a"), Some(&json!("1")));
        assert_eq!(map.get("b"), Some(&json!("2")));
    }

    #[test]
    fn empty_local_storage_is_ok() {
        assert!(parse_local_storage_json(b"").unwrap().is_empty());
        assert!(parse_local_storage_json(b"   ").unwrap().is_empty());
        assert!(parse_local_storage_json(br#"{"data":{}}"#)
            .unwrap()
            .is_empty());
    }
}
