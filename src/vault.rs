//! The encrypted account vault: data model + load/save.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
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
    /// Free-form note (e.g. "2FA on the work phone"), display-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Free-form tags (e.g. "prod", "admin") for grouping and search.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
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
    /// Ids removed (or renamed away) and when — so a sync can propagate the
    /// delete instead of resurrecting the account from another machine.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    deleted: BTreeMap<String, DateTime<Utc>>,
    /// CookieCloud-compatible sync settings (endpoint, uuid, password). Kept in
    /// the encrypted vault, never in a plaintext config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cloud: Option<CloudConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudConfig {
    /// "cookiecloud" (a CookieCloud server at `endpoint`) or "github" (a file
    /// in a private GitHub repo, through the user's `gh` login — no server).
    #[serde(default = "default_backend")]
    pub backend: String,
    /// `owner/repo` for the github backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_repo: Option<String>,
    #[serde(default)]
    pub endpoint: String,
    pub uuid: String,
    pub password: String,
    /// "aes-128-cbc-fixed" (default) or "legacy" (CryptoJS passphrase mode).
    pub crypto_type: String,
    /// Also publish the most recently used account per site in CookieCloud's
    /// own `cookie_data` shape, so the CookieCloud browser extension can
    /// download it. Off → only cookie-use's sealed vault is uploaded.
    #[serde(default = "default_true")]
    pub browser_compat: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_push: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pull: Option<DateTime<Utc>>,
}

fn default_true() -> bool {
    true
}

fn default_backend() -> String {
    "cookiecloud".into()
}

/// What a [`Vault::merge`] changed.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct MergeReport {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: usize,
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
        self.mark_deleted(id);
        Ok(())
    }

    /// Record that `id` no longer exists here (removed or renamed away).
    pub fn mark_deleted(&mut self, id: &str) {
        self.data.deleted.insert(id.to_string(), Utc::now());
    }

    pub fn deleted(&self) -> &BTreeMap<String, DateTime<Utc>> {
        &self.data.deleted
    }

    pub fn cloud(&self) -> Option<&CloudConfig> {
        self.data.cloud.as_ref()
    }

    pub fn set_cloud(&mut self, cfg: Option<CloudConfig>) {
        self.data.cloud = cfg;
    }

    /// Merge accounts from another machine. Per id the newer `updated_at` wins;
    /// a delete wins over any copy that is older than it. Never touches an
    /// account the other side doesn't mention.
    pub fn merge(
        &mut self,
        remote: Vec<Account>,
        remote_deleted: &BTreeMap<String, DateTime<Utc>>,
    ) -> MergeReport {
        let mut report = MergeReport::default();
        for (id, at) in remote_deleted {
            let newer = self.data.deleted.get(id).map(|t| at > t).unwrap_or(true);
            if newer {
                self.data.deleted.insert(id.clone(), *at);
            }
            if let Some(local) = self.find(id) {
                if local.updated_at < *at {
                    self.data.accounts.retain(|a| &a.id != id);
                    report.removed.push(id.clone());
                }
            }
        }
        for acct in remote {
            if self
                .data
                .deleted
                .get(&acct.id)
                .map(|t| acct.updated_at <= *t)
                .unwrap_or(false)
            {
                continue; // deleted here after that copy was made
            }
            match self.find_mut(&acct.id) {
                None => {
                    report.added.push(acct.id.clone());
                    self.data.deleted.remove(&acct.id);
                    self.data.accounts.push(acct);
                }
                Some(local) if acct.updated_at > local.updated_at => {
                    report.updated.push(acct.id.clone());
                    *local = acct;
                }
                Some(local) => {
                    // Same or older copy: keep ours, but remember the latest use.
                    if acct.last_used_at > local.last_used_at {
                        local.last_used_at = acct.last_used_at;
                    }
                    report.unchanged += 1;
                }
            }
        }
        report
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

/// Directory that holds the vault and its plaintext sidecars (e.g. the
/// fingerprint cache). Mirrors [`vault_path`]: the parent of `COOKIE_USE_VAULT`
/// when set, else `~/.cookie-use`.
pub fn config_dir() -> Result<PathBuf> {
    let p = vault_path()?;
    Ok(p.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".")))
}

fn vault_path() -> Result<PathBuf> {
    // `COOKIE_USE_VAULT` overrides the vault file location (headless hosts,
    // multiple isolated vaults, and isolated integration tests).
    if let Some(p) = std::env::var_os("COOKIE_USE_VAULT") {
        return Ok(PathBuf::from(p));
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("could not find home directory"))?;
    Ok(home.join(".cookie-use").join("vault.enc"))
}

/// The host to open for a comma-joined `site`: the first host, unless a later
/// one is a subdomain of it — then that, since `cloudflare.com,dash.cloudflare.com`
/// means "log in on the dashboard", not the marketing page. localStorage is
/// captured and injected on this same origin.
pub fn landing_host(site: &str) -> String {
    let hosts: Vec<&str> = site
        .split(',')
        .map(|h| h.trim().trim_start_matches('.'))
        .filter(|h| !h.is_empty())
        .collect();
    let Some(first) = hosts.first() else {
        return site.trim().to_string();
    };
    let suffix = format!(".{first}");
    hosts
        .iter()
        .find(|h| h.ends_with(&suffix))
        .unwrap_or(first)
        .to_string()
}

#[cfg(test)]
mod landing_tests {
    use super::landing_host;

    #[test]
    fn prefers_a_listed_subdomain_of_the_first_host() {
        assert_eq!(
            landing_host("cloudflare.com,dash.cloudflare.com"),
            "dash.cloudflare.com"
        );
        assert_eq!(landing_host("chatgpt.com,openai.com"), "chatgpt.com");
        assert_eq!(
            landing_host("pb-super-admin.pwtk.cc,pwtk.cc"),
            "pb-super-admin.pwtk.cc"
        );
        assert_eq!(landing_host(" .example.com "), "example.com");
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use chrono::Duration;

    fn acct(id: &str, updated: DateTime<Utc>) -> Account {
        Account {
            id: id.into(),
            site: "x.com".into(),
            label: None,
            account_hint: None,
            note: None,
            tags: vec![],
            cookies: vec![],
            local_storage: None,
            created_at: updated,
            updated_at: updated,
            last_used_at: None,
            status: Status::Live,
            proxy: None,
            fingerprint: None,
        }
    }

    fn vault(accounts: Vec<Account>) -> Vault {
        Vault {
            data: VaultData {
                accounts,
                ..Default::default()
            },
            key: [0; 32],
            path: PathBuf::from("/nonexistent"),
        }
    }

    #[test]
    fn newer_wins_and_new_ids_are_added() {
        let t = Utc::now();
        let mut v = vault(vec![acct("a", t), acct("b", t)]);
        let mut newer_a = acct("a", t + Duration::seconds(5));
        newer_a.label = Some("remote".into());
        let older_b = acct("b", t - Duration::seconds(5));
        let r = v.merge(vec![newer_a, older_b, acct("c", t)], &BTreeMap::new());
        assert_eq!(r.updated, vec!["a"]);
        assert_eq!(r.added, vec!["c"]);
        assert_eq!(r.unchanged, 1);
        assert_eq!(v.find("a").unwrap().label.as_deref(), Some("remote"));
    }

    #[test]
    fn deletes_propagate_but_never_beat_a_newer_copy() {
        let t = Utc::now();
        let mut v = vault(vec![
            acct("old", t),
            acct("fresh", t + Duration::seconds(60)),
        ]);
        let mut gone = BTreeMap::new();
        gone.insert("old".to_string(), t + Duration::seconds(1));
        gone.insert("fresh".to_string(), t + Duration::seconds(1));
        let r = v.merge(vec![], &gone);
        assert_eq!(r.removed, vec!["old"]);
        assert!(v.find("fresh").is_some(), "edited after the remote delete");

        // A copy older than our own delete doesn't come back.
        v.mark_deleted("zombie");
        let r = v.merge(
            vec![acct("zombie", t - Duration::seconds(5))],
            &BTreeMap::new(),
        );
        assert!(r.added.is_empty() && v.find("zombie").is_none());
    }
}
