//! Vault master-key storage. On macOS the 32-byte AES key lives in the login
//! Keychain (service "cookie-use", account "vault-key"), base64-encoded — so the
//! encrypted vault on disk is useless without the user's Keychain.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};

#[cfg(target_os = "macos")]
const SERVICE: &str = "cookie-use";
#[cfg(target_os = "macos")]
const ACCOUNT: &str = "vault-key";

/// Return the vault key, creating and persisting one on first use.
pub fn get_or_create_key() -> Result<[u8; 32]> {
    if let Some(k) = get_key()? {
        return Ok(k);
    }
    let key = crate::crypto::generate_key();
    store_key(&key)?;
    Ok(key)
}

#[cfg(target_os = "macos")]
fn get_key() -> Result<Option<[u8; 32]>> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
        .output()
        .context("running `security` to read the vault key")?;
    if !out.status.success() {
        return Ok(None); // not found yet
    }
    let b64 = String::from_utf8_lossy(&out.stdout);
    let bytes = B64
        .decode(b64.trim())
        .context("decoding the stored vault key")?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("stored vault key has the wrong length"))?;
    Ok(Some(arr))
}

#[cfg(target_os = "macos")]
fn store_key(key: &[u8; 32]) -> Result<()> {
    let b64 = B64.encode(key);
    let status = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            SERVICE,
            "-a",
            ACCOUNT,
            "-w",
            &b64,
            "-U",
        ])
        .status()
        .context("running `security` to store the vault key")?;
    if !status.success() {
        return Err(anyhow!("failed to store the vault key in the Keychain"));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn get_key() -> Result<Option<[u8; 32]>> {
    Err(anyhow!("cookie-use currently supports macOS only"))
}

#[cfg(not(target_os = "macos"))]
fn store_key(_key: &[u8; 32]) -> Result<()> {
    Err(anyhow!("cookie-use currently supports macOS only"))
}
