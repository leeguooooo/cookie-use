//! The encrypted account vault: data model + load/save.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// One stored session for one site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Namespaced id, e.g. "chatgpt/work-01".
    pub id: String,
    /// The domain(s) this session covers, comma-joined as given by the user.
    pub site: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional human hint (email / username), display-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_hint: Option<String>,
    /// Full cross-domain cookie set, CDP `Network.setCookie` shape.
    pub cookies: Vec<Value>,
    /// Optional localStorage snapshot for the primary origin (key -> value).
    /// Many SPAs keep token/user info here, not in cookies; captured on demand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_storage: Option<serde_json::Map<String, Value>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: Status,
    // Reserved for v2 (anti-correlation). Kept optional so the model is stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Unknown,
    Live,
    Expired,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Status::Unknown => "unknown",
            Status::Live => "live",
            Status::Expired => "expired",
        };
        f.pad(s)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct VaultData {
    #[serde(default)]
    accounts: Vec<Account>,
}

pub struct Vault {
    data: VaultData,
    key: [u8; 32],
    path: PathBuf,
}

impl Vault {
    /// Open (or initialize) the vault at `~/.cookie-use/vault.enc`.
    pub fn open() -> Result<Self> {
        let path = vault_path()?;
        let key = crate::keychain::get_or_create_key()?;
        let data = if path.exists() {
            let raw = std::fs::read_to_string(&path).context("reading vault file")?;
            let blob =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw.trim())
                    .context("decoding vault file")?;
            let plain = crate::crypto::decrypt(&key, &blob)?;
            serde_json::from_slice(&plain).context("parsing decrypted vault")?
        } else {
            VaultData::default()
        };
        Ok(Self { data, key, path })
    }

    pub fn accounts(&self) -> &[Account] {
        &self.data.accounts
    }

    pub fn find(&self, id: &str) -> Option<&Account> {
        self.data.accounts.iter().find(|a| a.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Account> {
        self.data.accounts.iter_mut().find(|a| a.id == id)
    }

    /// Insert or replace an account by id.
    pub fn upsert(&mut self, account: Account) {
        if let Some(existing) = self.find_mut(&account.id) {
            *existing = account;
        } else {
            self.data.accounts.push(account);
        }
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.data.accounts.len();
        self.data.accounts.retain(|a| a.id != id);
        if self.data.accounts.len() == before {
            return Err(anyhow!("no account with id \"{}\"", id));
        }
        Ok(())
    }

    /// Delete the on-disk vault file entirely. Used by `wipe`.
    pub fn delete_file(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path).context("deleting vault file")?;
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).context("creating ~/.cookie-use")?;
        }
        let plain = serde_json::to_vec(&self.data)?;
        let blob = crate::crypto::encrypt(&self.key, &plain)?;
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, blob);
        // Write atomically (temp + rename) so a crash can't truncate the vault.
        let tmp = self.path.with_extension("enc.tmp");
        std::fs::write(&tmp, b64).context("writing vault")?;
        std::fs::rename(&tmp, &self.path).context("committing vault")?;
        Ok(())
    }
}

fn vault_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("could not find home directory"))?;
    Ok(home.join(".cookie-use").join("vault.enc"))
}
