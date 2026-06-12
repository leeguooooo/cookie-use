//! Thin wrapper around the `chrome-use` CLI. cookie-use delegates every browser
//! and cookie operation here, so it never re-implements decryption or CDP.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
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
}

impl Target {
    pub fn parse(s: &str) -> Result<Target> {
        if s == "isolated" {
            Ok(Target::Isolated)
        } else if let Some(name) = s.strip_prefix("session:") {
            Ok(Target::Session(name.to_string()))
        } else if s.strip_prefix("profile:").is_some() {
            Err(anyhow!(
                "profile: targets aren't in v0.1 — connect chrome-use's extension to that \
                 profile and use `--target session:<name>` instead"
            ))
        } else {
            Err(anyhow!(
                "unknown target \"{}\" (use session:<name> or isolated)",
                s
            ))
        }
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

/// Apply a cookie set to a target. Returns once chrome-use confirms.
pub fn apply(cookies: &[Value], target: &Target, open_url: Option<&str>) -> Result<()> {
    let tmp = write_temp_cookies(cookies)?;
    let path = tmp.to_string_lossy().to_string();

    match target {
        Target::Session(session) => {
            run(&["--session", session, "cookies", "set", "--curl", &path])?;
            if let Some(url) = open_url {
                run(&["--session", session, "open", url])?;
            }
        }
        Target::Isolated => {
            let session = "cookie-use-iso";
            // Launch a throwaway browser, seed the cookies, then load the site.
            run(&["--session", session, "--launch", "open", "about:blank"])?;
            run(&["--session", session, "cookies", "set", "--curl", &path])?;
            if let Some(url) = open_url {
                run(&["--session", session, "open", url])?;
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

/// Clear a site's cookies in a session target (used by `switch`).
pub fn clear(target: &Target) -> Result<()> {
    if let Target::Session(session) = target {
        run(&["--session", session, "cookies", "clear"])?;
    }
    Ok(())
}

fn run(args: &[&str]) -> Result<()> {
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
    Ok(())
}

fn parse_cookies_json(stdout: &[u8]) -> Result<Vec<Value>> {
    let v: Value = serde_json::from_slice(stdout).context("parsing chrome-use output")?;
    // chrome-use --json wraps as {success, data:[...]}; tolerate a bare array too.
    let arr = v.get("data").unwrap_or(&v);
    arr.as_array()
        .cloned()
        .ok_or_else(|| anyhow!("unexpected chrome-use output shape"))
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
