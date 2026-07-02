//! Hash-only session fingerprints.
//!
//! A fingerprint is a per-cookie SHA-256 of the cookie *value* (never the value
//! itself), so a separate tool — `chrome-use` — can verify "is the live browser
//! session logged in as this account?" without ever seeing a secret.
//!
//! Fingerprints are cached in a **plaintext** sidecar next to the vault
//! (`~/.cookie-use/fingerprints.json`). They hold only hashes + cookie
//! names/scope, so they're safe at rest, and reading them needs neither the
//! Keychain nor a vault decrypt — the encrypted vault stays the sole home of
//! cookie values.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::vault::Account;

/// Cookie values shorter than this are excluded from the fingerprint:
/// low-entropy values ("en", "1") are brute-forceable and useless as identity.
const MIN_VALUE_LEN: usize = 8;

/// On-disk schema version for the plaintext cache.
const CACHE_VERSION: u32 = 1;

/// One cookie's identity contribution: its name/scope plus a hash of its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieFingerprint {
    pub name: String,
    pub domain: String,
    pub path: String,
    /// Lowercase hex SHA-256 of the raw cookie value string.
    pub sha256: String,
    #[serde(rename = "httpOnly")]
    pub http_only: bool,
    pub secure: bool,
}

/// A whole account's hash-only fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountFingerprint {
    pub id: String,
    pub site: String,
    pub domains: Vec<String>,
    pub cookies: Vec<CookieFingerprint>,
    pub computed_at: DateTime<Utc>,
}

/// Compute a hash-only fingerprint from an account's stored cookies. Cookies
/// whose value is shorter than [`MIN_VALUE_LEN`] chars are excluded.
pub fn compute(account: &Account) -> AccountFingerprint {
    let mut cookies: Vec<CookieFingerprint> = account
        .cookies
        .iter()
        .filter_map(cookie_fingerprint)
        .collect();
    // Deterministic order so the same session hashes to the same fingerprint
    // regardless of capture order.
    cookies.sort_by(|a, b| (&a.domain, &a.name, &a.path).cmp(&(&b.domain, &b.name, &b.path)));

    let mut domains: Vec<String> = cookies.iter().map(|c| c.domain.clone()).collect();
    domains.sort();
    domains.dedup();

    AccountFingerprint {
        id: account.id.clone(),
        site: account.site.clone(),
        domains,
        cookies,
        computed_at: Utc::now(),
    }
}

/// Hash one cookie, or `None` if it lacks name/value or its value is low-entropy.
fn cookie_fingerprint(c: &Value) -> Option<CookieFingerprint> {
    let name = c.get("name").and_then(Value::as_str)?;
    let value = c.get("value").and_then(Value::as_str)?;
    if value.chars().count() < MIN_VALUE_LEN {
        return None; // low-entropy — excluded
    }
    Some(CookieFingerprint {
        name: name.to_string(),
        domain: c
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        path: c
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("/")
            .to_string(),
        sha256: sha256_hex(value.as_bytes()),
        http_only: c.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
        secure: c.get("secure").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Lowercase hex SHA-256.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ---------------------------------------------------------------------------
// Plaintext cache
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: BTreeMap<String, AccountFingerprint>,
}

/// The plaintext fingerprint cache (`~/.cookie-use/fingerprints.json`).
pub struct Cache {
    data: CacheFile,
    path: PathBuf,
}

impl Cache {
    /// Load the cache from its default location (creating an empty one in
    /// memory if the file doesn't exist yet). No Keychain, no vault decrypt.
    pub fn open() -> Result<Self> {
        Self::open_path(cache_path()?)
    }

    fn open_path(path: PathBuf) -> Result<Self> {
        let data = if path.exists() {
            let raw = std::fs::read_to_string(&path).context("reading fingerprint cache")?;
            serde_json::from_str(&raw).context("parsing fingerprint cache")?
        } else {
            CacheFile::default()
        };
        Ok(Self { data, path })
    }

    pub fn get(&self, id: &str) -> Option<&AccountFingerprint> {
        self.data.accounts.get(id)
    }

    /// Ids of every cached account, sorted (the `BTreeMap` keeps them ordered).
    pub fn ids(&self) -> Vec<String> {
        self.data.accounts.keys().cloned().collect()
    }

    pub fn insert(&mut self, fp: AccountFingerprint) {
        self.data.accounts.insert(fp.id.clone(), fp);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.data.accounts.remove(id).is_some()
    }

    pub fn save(&mut self) -> Result<()> {
        self.data.version = CACHE_VERSION;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).context("creating ~/.cookie-use")?;
        }
        let json = serde_json::to_string_pretty(&self.data)?;
        // Write atomically so a crash can't truncate the cache.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).context("writing fingerprint cache")?;
        std::fs::rename(&tmp, &self.path).context("committing fingerprint cache")?;
        Ok(())
    }

    /// Delete the on-disk cache file entirely (used by `wipe`).
    pub fn delete_file() -> Result<()> {
        let path = cache_path()?;
        if path.exists() {
            std::fs::remove_file(&path).context("deleting fingerprint cache")?;
        }
        Ok(())
    }
}

/// Recompute and cache an account's fingerprint. Called after add/import/use so
/// the plaintext cache stays warm and later `fingerprint` reads need no decrypt.
pub fn refresh_cache(account: &Account) -> Result<()> {
    let mut cache = Cache::open()?;
    cache.insert(compute(account));
    cache.save()
}

/// Drop an account's cached fingerprint (best effort) so stale hashes don't
/// linger after `rm` / `rename`.
pub fn forget(id: &str) {
    if let Ok(mut cache) = Cache::open() {
        if cache.remove(id) {
            let _ = cache.save();
        }
    }
}

fn cache_path() -> Result<PathBuf> {
    Ok(crate::vault::config_dir()?.join("fingerprints.json"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(cookies: Vec<Value>) -> Account {
        let now = Utc::now();
        Account {
            id: "cloudflare/davian".to_string(),
            site: "cloudflare.com,dash.cloudflare.com".to_string(),
            label: None,
            account_hint: None,
            cookies,
            local_storage: None,
            created_at: now,
            updated_at: now,
            last_used_at: None,
            status: crate::vault::Status::Live,
            proxy: None,
            fingerprint: None,
        }
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // Standard NIST test vector for SHA-256("abc").
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn fingerprint_shape_hashes_values_and_never_leaks_them() {
        let a = account(vec![json!({
            "name": "CF_Authorization",
            "value": "a-long-secret-session-token",
            "domain": ".cloudflare.com",
            "path": "/",
            "httpOnly": true,
            "secure": true,
        })]);
        let fp = compute(&a);
        assert_eq!(fp.id, "cloudflare/davian");
        assert_eq!(fp.cookies.len(), 1);
        let c = &fp.cookies[0];
        assert_eq!(c.name, "CF_Authorization");
        assert_eq!(c.domain, ".cloudflare.com");
        assert!(c.http_only && c.secure);
        assert_eq!(c.sha256, sha256_hex(b"a-long-secret-session-token"));
        assert_eq!(fp.domains, vec![".cloudflare.com".to_string()]);

        // The serialized form uses the agreed camelCase key and never the value.
        let s = serde_json::to_string(&fp).unwrap();
        assert!(s.contains("\"httpOnly\":true"), "missing httpOnly key: {s}");
        assert!(!s.contains("http_only"), "leaked snake_case key: {s}");
        assert!(
            !s.contains("a-long-secret-session-token"),
            "fingerprint leaked a cookie value: {s}"
        );
    }

    #[test]
    fn low_entropy_values_are_excluded() {
        let a = account(vec![
            json!({"name": "lang", "value": "en", "domain": ".x.com", "path": "/"}),
            json!({"name": "n", "value": "1", "domain": ".x.com", "path": "/"}),
            json!({"name": "sid", "value": "exactly8", "domain": ".x.com", "path": "/"}),
            json!({"name": "big", "value": "this-is-plenty-of-entropy", "domain": ".x.com", "path": "/"}),
        ]);
        let fp = compute(&a);
        let names: Vec<&str> = fp.cookies.iter().map(|c| c.name.as_str()).collect();
        // "en" (2) and "1" (1) dropped; "exactly8" (8) and the long one kept.
        assert_eq!(names, vec!["big", "sid"]); // sorted by (domain, name, path)
    }

    #[test]
    fn cache_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("cu-fp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fingerprints.json");

        let fp = compute(&account(vec![json!({
            "name": "s", "value": "a-real-session-value", "domain": ".x.com", "path": "/"
        })]));

        let mut cache = Cache::open_path(path.clone()).unwrap();
        cache.insert(fp.clone());
        cache.save().unwrap();

        let reopened = Cache::open_path(path.clone()).unwrap();
        assert_eq!(reopened.get("cloudflare/davian"), Some(&fp));

        std::fs::remove_dir_all(&dir).ok();
    }
}
