//! `share` / `redeem` — export one stored account as a password-encrypted
//! `.cusession` bundle that a teammate redeems.
//!
//! # Bundle format (JSON, UTF-8)
//!
//! ```json
//! {
//!   "cookie_use_bundle": 1,
//!   "id":   "<original account id>",
//!   "site": "<site string>",
//!   "kdf":  "argon2id",
//!   "salt": "<base64-std 16 bytes>",
//!   "ciphertext": "<base64-std; nonce(12)||ciphertext as produced by crate::crypto::encrypt>"
//! }
//! ```
//!
//! The plaintext (before encryption) is the full [`Account`] serialised as
//! JSON. Cookie values and localStorage data never appear outside the
//! ciphertext — the plaintext-at-rest invariant is maintained throughout.
//!
//! # Key derivation
//!
//! Key = Argon2id(password, salt, m=65536, t=3, p=1) → 32 bytes.
//! A fresh random 16-byte salt is generated per `share` invocation.
//! The same password + salt combination is used for `redeem`.

use crate::vault::{Account, Vault};
use anyhow::{anyhow, bail, Context, Result};
use argon2::{Argon2, Params};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Bundle wire type
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Bundle {
    /// Format version sentinel — bump if the shape changes incompatibly.
    cookie_use_bundle: u32,
    id: String,
    site: String,
    kdf: String,
    /// Base64-encoded 16-byte Argon2id salt.
    salt: String,
    /// Base64-encoded `nonce(12) || ciphertext` blob (crate::crypto::encrypt output).
    ciphertext: String,
}

// ---------------------------------------------------------------------------
// KDF
// ---------------------------------------------------------------------------

/// Derive a 32-byte AES key from `password` + `salt` using Argon2id.
///
/// Parameters: m = 64 MiB, t = 3, p = 1 — generous enough for an offline
/// brute-force attacker while staying under ~1 s on a laptop.
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    // m_cost in kibibytes, t_cost = iterations, p_cost = parallelism
    let params = Params::new(65536, 3, 1, Some(32)).map_err(|e| anyhow!("argon2 params: {e}"))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("argon2id key derivation failed: {e}"))?;
    Ok(key)
}

// ---------------------------------------------------------------------------
// Pure crypto helpers (used by both the commands and the unit tests)
// ---------------------------------------------------------------------------

/// Encrypt `account` with `password`; return bundle JSON bytes.
///
/// Pure function — no I/O; suitable for unit testing.
pub fn seal(account: &Account, password: &str) -> Result<Vec<u8>> {
    // Fresh random salt per call.
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);

    let key = derive_key(password, &salt)?;
    let plaintext = serde_json::to_vec(account).context("serialising account")?;
    let encrypted = crate::crypto::encrypt(&key, &plaintext)?;

    let bundle = Bundle {
        cookie_use_bundle: 1,
        id: account.id.clone(),
        site: account.site.clone(),
        kdf: "argon2id".to_string(),
        salt: B64.encode(salt),
        ciphertext: B64.encode(&encrypted),
    };
    serde_json::to_vec_pretty(&bundle).context("serialising bundle")
}

/// Decrypt bundle JSON bytes with `password`; return the reconstructed
/// [`Account`].
///
/// Returns a descriptive error on wrong password, corrupt data, or an
/// unrecognised bundle version.
pub fn unseal(bundle_bytes: &[u8], password: &str) -> Result<Account> {
    let bundle: Bundle = serde_json::from_slice(bundle_bytes)
        .context("parsing bundle JSON — is this a .cusession file?")?;

    if bundle.cookie_use_bundle != 1 {
        bail!(
            "unsupported bundle version {} (this build only supports version 1)",
            bundle.cookie_use_bundle
        );
    }
    if bundle.kdf != "argon2id" {
        bail!("unsupported KDF \"{}\" in bundle", bundle.kdf);
    }

    let salt = B64.decode(&bundle.salt).context("decoding bundle salt")?;
    let encrypted = B64
        .decode(&bundle.ciphertext)
        .context("decoding bundle ciphertext")?;

    let key = derive_key(password, &salt)?;

    let plaintext = crate::crypto::decrypt(&key, &encrypted)
        .map_err(|_| anyhow!("wrong password or corrupt bundle"))?;

    let account: Account =
        serde_json::from_slice(&plaintext).context("deserialising account from bundle")?;

    Ok(account)
}

// ---------------------------------------------------------------------------
// ID slug helper
// ---------------------------------------------------------------------------

/// Produce a filesystem-safe filename slug from an account id.
///
/// Replaces `/` and any non-alphanumeric character with `-`.
fn id_to_slug(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Password prompting
// ---------------------------------------------------------------------------

/// Read a password. If `password` is already `Some`, return it. Otherwise,
/// if stdin is a TTY, prompt interactively (echo suppressed). If stdin is
/// not a TTY and no password was supplied, return an instructive error.
fn require_password(password: Option<&str>, prompt: &str) -> Result<String> {
    if let Some(p) = password {
        return Ok(p.to_string());
    }
    // Check if stdin is a terminal.
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let pw = rpassword::prompt_password(prompt).context("reading password from terminal")?;
        if pw.is_empty() {
            bail!("password must not be empty");
        }
        Ok(pw)
    } else {
        bail!("no password supplied and stdin is not a terminal — pass --password <pw>");
    }
}

// ---------------------------------------------------------------------------
// Public command entry points
// ---------------------------------------------------------------------------

/// Share: export one stored account as a password-encrypted `.cusession`
/// bundle.
///
/// Writes `<slug>.cusession` (or the path given via `out`) and prints the
/// path plus a one-line redeem hint so the recipient knows exactly what to
/// run.
pub fn cmd_share(vault: &Vault, id: &str, out: Option<&str>, password: Option<&str>) -> Result<()> {
    let account = vault
        .find(id)
        .ok_or_else(|| anyhow!("no account \"{id}\""))?;

    let password = require_password(password, "enter bundle password: ")?;

    let bundle_bytes = seal(account, &password)?;

    let path: String = match out {
        Some(p) => p.to_string(),
        None => format!("{}.cusession", id_to_slug(id)),
    };

    std::fs::write(&path, &bundle_bytes).with_context(|| format!("writing bundle to {path}"))?;

    println!("{path}");
    println!("redeem with: cookie-use redeem {path}");
    Ok(())
}

/// Redeem: import a `.cusession` bundle into the local vault.
///
/// Decrypts the bundle, optionally renames the account (`new_id`), refreshes
/// `updated_at`, upserts into the vault, and saves.
pub fn cmd_redeem(
    vault: &mut Vault,
    bundle_path: &str,
    password: Option<&str>,
    new_id: Option<&str>,
) -> Result<()> {
    let bundle_bytes =
        std::fs::read(bundle_path).with_context(|| format!("reading bundle {bundle_path}"))?;

    // Sniff-check before asking for the password.
    let _pre: serde_json::Value = serde_json::from_slice(&bundle_bytes)
        .context("bundle is not valid JSON — not a .cusession file")?;

    let password = require_password(password, "enter bundle password: ")?;

    let mut account = unseal(&bundle_bytes, &password)?;

    // Optionally rename the account; always refresh timestamps.
    if let Some(nid) = new_id {
        account.id = nid.to_string();
    }
    account.updated_at = Utc::now();

    let final_id = account.id.clone();
    let site = account.site.clone();

    vault.upsert(account);
    vault.save()?;

    // Nudge the recipient to install cookie-use if they redeemed from a
    // raw file share (the install hint is the viral loop).
    println!("redeemed \"{final_id}\" ({site})");
    println!("hint: if cookie-use is not installed → https://github.com/leeguooooo/cookie-use");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{Account, Status};
    use chrono::Utc;
    use serde_json::{json, Value};

    /// Build a minimal test account — no real cookies, but realistic shape.
    fn test_account() -> Account {
        let now = Utc::now();
        Account {
            id: "test/01".to_string(),
            site: "example.com".to_string(),
            label: Some("Alice".to_string()),
            account_hint: Some("alice@example.com".to_string()),
            cookies: vec![json!({
                "name": "session_id",
                "value": "SUPER_SECRET_TOKEN_abc123",
                "domain": ".example.com",
                "path": "/"
            })],
            local_storage: None,
            created_at: now,
            updated_at: now,
            last_used_at: None,
            status: Status::Live,
            proxy: None,
            fingerprint: None,
        }
    }

    // 1. Round-trip: seal → unseal returns an equivalent account.
    #[test]
    fn roundtrip_seal_unseal() {
        let account = test_account();
        let bundle = seal(&account, "correct horse battery staple").unwrap();
        let recovered = unseal(&bundle, "correct horse battery staple").unwrap();

        assert_eq!(recovered.id, account.id);
        assert_eq!(recovered.site, account.site);
        assert_eq!(recovered.cookies.len(), account.cookies.len());
        // Cookie values are faithfully restored.
        assert_eq!(
            recovered.cookies[0].get("value").and_then(Value::as_str),
            Some("SUPER_SECRET_TOKEN_abc123")
        );
    }

    // 2. Wrong password → unseal returns an error.
    #[test]
    fn wrong_password_errors() {
        let account = test_account();
        let bundle = seal(&account, "correct-password").unwrap();
        let result = unseal(&bundle, "wrong-password");
        assert!(result.is_err(), "expected error with wrong password");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("wrong password") || msg.contains("corrupt"),
            "error message should mention wrong password or corrupt bundle, got: {msg}"
        );
    }

    // 3. Tampered / garbage bundle → clear error.
    #[test]
    fn garbage_bundle_errors() {
        let result = unseal(b"this is not json at all", "whatever");
        assert!(result.is_err());

        // Tampered ciphertext (flip a byte).
        let account = test_account();
        let mut bundle_bytes = seal(&account, "pw").unwrap();
        // Find the ciphertext value and flip the last base64 character.
        let last = bundle_bytes.len() - 3; // before the closing `"}`
        bundle_bytes[last] ^= 0xff;
        let result = unseal(&bundle_bytes, "pw");
        assert!(result.is_err(), "tampered bundle should fail");
    }

    // 4. The bundle JSON must NOT contain the known cookie value in cleartext.
    #[test]
    fn bundle_does_not_leak_cookie_values() {
        let secret = "SUPER_SECRET_TOKEN_abc123";
        let account = test_account();
        let bundle = seal(&account, "some-password").unwrap();

        // The raw bundle bytes (JSON) must not contain the secret anywhere.
        let bundle_str = String::from_utf8_lossy(&bundle);
        assert!(
            !bundle_str.contains(secret),
            "bundle must not contain the cookie value in plaintext, but found it in: {bundle_str}"
        );
    }

    // 5. Two seal calls with same password produce different ciphertexts (random salt/nonce).
    #[test]
    fn seal_is_non_deterministic() {
        let account = test_account();
        let b1 = seal(&account, "pw").unwrap();
        let b2 = seal(&account, "pw").unwrap();
        // Different salt means different ciphertext.
        assert_ne!(b1, b2, "two seal calls should produce different bundles");
    }

    // 6. id_to_slug replaces slashes and spaces.
    #[test]
    fn slug_replaces_special_chars() {
        assert_eq!(id_to_slug("chatgpt/work-01"), "chatgpt-work-01");
        assert_eq!(id_to_slug("my account / test"), "my-account---test");
        assert_eq!(id_to_slug("simple"), "simple");
    }
}
