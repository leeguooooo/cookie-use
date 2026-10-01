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
    /// When the session (cookies / localStorage) last changed, and when the
    /// user's metadata (label / hint / note / tags) last changed. A sync merges
    /// the two independently, so editing tags on one Mac never throws away a
    /// fresher login captured on another. Missing (older vaults) = `updated_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: Status,
    // Reserved for v2 (anti-correlation). Kept optional so the model is stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Value>,
}

impl Account {
    pub fn session_ts(&self) -> DateTime<Utc> {
        self.session_updated_at.unwrap_or(self.updated_at)
    }

    pub fn meta_ts(&self) -> DateTime<Utc> {
        self.meta_updated_at.unwrap_or(self.updated_at)
    }

    /// Mark the session as changed now (keeps `updated_at` = latest change).
    pub fn touch_session(&mut self) {
        let ts = after(self.session_ts());
        self.session_updated_at = Some(ts);
        self.updated_at = self.updated_at.max(ts);
    }

    /// Mark the metadata as changed now.
    pub fn touch_meta(&mut self) {
        let ts = after(self.meta_ts());
        self.meta_updated_at = Some(ts);
        self.updated_at = self.updated_at.max(ts);
    }
}

/// "Now", but never at or before `prev`: an edit made after seeing a change
/// always counts as newer than it, even when this Mac's clock runs behind the
/// one that made `prev`.
pub fn after(prev: DateTime<Utc>) -> DateTime<Utc> {
    Utc::now().max(prev + chrono::Duration::milliseconds(1))
}

/// An id renamed away: where it went and when.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rename {
    pub to: String,
    pub at: DateTime<Utc>,
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
    /// Old id → new id for renames, so a sync applies another computer's
    /// edits of the old id to the renamed account instead of resurrecting it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    renamed: BTreeMap<String, Rename>,
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
    /// Accounts changed on both sides (e.g. tags here, login there) whose
    /// changes were combined rather than one side discarded.
    #[serde(default)]
    pub merged: Vec<String>,
    pub unchanged: usize,
}

pub struct Vault {
    data: VaultData,
    key: [u8; 32],
    path: PathBuf,
    /// Exclusive lock on `vault.lock`, held while this handle lives, so two
    /// processes (the app, an agent's CLI, a sync) can't lose each other's writes.
    _lock: Option<std::fs::File>,
}

/// Take the vault lock, waiting up to 30 s for another cookie-use process.
fn lock_vault(path: &std::path::Path) -> Result<std::fs::File> {
    let lock_path = path.with_extension("lock");
    if let Some(dir) = lock_path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).context("creating the vault directory")?;
    }
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(anyhow!(
                    "the vault is busy (another cookie-use is writing) — try again"
                ))
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e).context("locking the vault"),
        }
    }
}

impl Vault {
    /// Open (or initialize) the vault at `~/.cookie-use/vault.enc`.
    pub fn open() -> Result<Self> {
        let path = vault_path()?;
        let lock = lock_vault(&path)?;
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
        Ok(Self {
            data,
            key,
            path,
            _lock: Some(lock),
        })
    }

    /// Make every account the newest copy (after a restore) and forget local
    /// deletes of ids that are back.
    pub fn mark_all_current(&mut self) {
        for a in &mut self.data.accounts {
            a.touch_session();
            a.touch_meta();
        }
        let ids: Vec<String> = self.data.accounts.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            self.data.deleted.remove(&id);
        }
    }

    /// An unlocked, unsaveable copy for a dry-run merge.
    pub fn scratch_copy(other: &Vault) -> Vault {
        Vault {
            data: VaultData {
                accounts: other.data.accounts.clone(),
                deleted: other.data.deleted.clone(),
                renamed: other.data.renamed.clone(),
                cloud: None,
            },
            key: other.key,
            path: PathBuf::from("/nonexistent"),
            _lock: None,
        }
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

    pub fn renamed(&self) -> &BTreeMap<String, Rename> {
        &self.data.renamed
    }

    /// Record a rename (the old id also counts as deleted).
    pub fn mark_renamed(&mut self, old: &str, new: &str) {
        let at = Utc::now();
        self.data.deleted.insert(old.to_string(), at);
        self.data.renamed.insert(
            old.to_string(),
            Rename {
                to: new.to_string(),
                at,
            },
        );
    }

    pub fn cloud(&self) -> Option<&CloudConfig> {
        self.data.cloud.as_ref()
    }

    pub fn set_cloud(&mut self, cfg: Option<CloudConfig>) {
        self.data.cloud = cfg;
    }

    /// Merge accounts from another machine. Per account, the session (cookies,
    /// localStorage) and the metadata (label, hint, note, tags) are merged
    /// separately — each side's newer half wins — and `last_used_at` keeps the
    /// latest. A delete wins over any copy older than it. Accounts the other
    /// side doesn't mention are never touched.
    #[cfg(test)]
    pub fn merge(
        &mut self,
        remote: Vec<Account>,
        remote_deleted: &BTreeMap<String, DateTime<Utc>>,
    ) -> MergeReport {
        self.merge_full(remote, remote_deleted, &BTreeMap::new())
    }

    /// [`Vault::merge`] plus renames: an edit to an id that was renamed after
    /// the edit's base lands on the new id (on either side) instead of
    /// bringing the old id back as a duplicate.
    pub fn merge_full(
        &mut self,
        remote: Vec<Account>,
        remote_deleted: &BTreeMap<String, DateTime<Utc>>,
        remote_renamed: &BTreeMap<String, Rename>,
    ) -> MergeReport {
        for (old, r) in remote_renamed {
            let newer = self
                .data
                .renamed
                .get(old)
                .map(|l| r.at > l.at)
                .unwrap_or(true);
            if newer {
                self.data.renamed.insert(old.clone(), r.clone());
            }
        }
        // Local copies of renamed ids that were edited after the rename move
        // to the new id (merged there below) rather than surviving as a twin.
        let mut moved: Vec<Account> = Vec::new();
        let stale: Vec<String> = self
            .data
            .accounts
            .iter()
            .filter(|a| {
                self.data
                    .renamed
                    .get(&a.id)
                    .is_some_and(|r| a.updated_at > r.at)
            })
            .map(|a| a.id.clone())
            .collect();
        for id in stale {
            if let Some(pos) = self.data.accounts.iter().position(|a| a.id == id) {
                moved.push(self.data.accounts.remove(pos));
            }
        }
        let remote: Vec<Account> = remote
            .into_iter()
            .chain(moved)
            .map(|mut a| {
                if let Some(r) = self.data.renamed.get(&a.id) {
                    if a.updated_at > r.at {
                        // Follow the chain (a → b → c), each hop made before this edit.
                        let mut to = r.to.clone();
                        for _ in 0..8 {
                            match self.data.renamed.get(&to) {
                                Some(n) if a.updated_at > n.at => to = n.to.clone(),
                                _ => break,
                            }
                        }
                        a.id = to;
                    }
                }
                a
            })
            .collect();
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
            let Some(local) = self.find_mut(&acct.id) else {
                report.added.push(acct.id.clone());
                self.data.deleted.remove(&acct.id);
                self.data.accounts.push(acct);
                continue;
            };
            let take_session = acct.session_ts() > local.session_ts();
            let take_meta = acct.meta_ts() > local.meta_ts();
            if take_session {
                local.site = acct.site.clone();
                local.cookies = acct.cookies.clone();
                local.local_storage = acct.local_storage.clone();
                local.status = acct.status;
                local.session_updated_at = Some(acct.session_ts());
            }
            if take_meta {
                local.label = acct.label.clone();
                local.account_hint = acct.account_hint.clone();
                local.note = acct.note.clone();
                local.tags = acct.tags.clone();
                local.meta_updated_at = Some(acct.meta_ts());
            }
            if acct.last_used_at > local.last_used_at {
                local.last_used_at = acct.last_used_at;
            }
            local.created_at = local.created_at.min(acct.created_at);
            local.updated_at = local.session_ts().max(local.meta_ts());
            match (take_session, take_meta) {
                (false, false) => report.unchanged += 1,
                (true, true) => report.updated.push(acct.id.clone()),
                // One half from each side: both edits survive.
                (true, false) if local.meta_ts() > acct.meta_ts() => {
                    report.merged.push(acct.id.clone())
                }
                (false, true) if local.session_ts() > acct.session_ts() => {
                    report.merged.push(acct.id.clone())
                }
                _ => report.updated.push(acct.id.clone()),
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

/// The vault file's path (for snapshots).
pub fn vault_file() -> Result<PathBuf> {
    vault_path()
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
            session_updated_at: None,
            meta_updated_at: None,
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
            _lock: None,
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
    fn an_edit_after_a_future_stamp_still_counts_as_newer() {
        // Another Mac with a fast clock stamped this account 10 minutes ahead.
        let ahead = Utc::now() + Duration::minutes(10);
        let mut a = acct("a", ahead);
        a.meta_updated_at = Some(ahead);
        a.touch_meta();
        assert!(a.meta_ts() > ahead);
        assert!(a.updated_at > ahead);
    }

    #[test]
    fn session_and_metadata_merge_independently() {
        let t = Utc::now();
        let mut local = acct("a", t);
        local.tags = vec!["old".into()];
        local.cookies = vec![serde_json::json!({"name": "sid", "value": "fresh"})];
        local.session_updated_at = Some(t + Duration::seconds(10)); // re-captured here
        local.meta_updated_at = Some(t);
        local.updated_at = t + Duration::seconds(10);
        let mut v = vault(vec![local]);

        let mut remote = acct("a", t);
        remote.tags = vec!["prod".into()];
        remote.cookies = vec![serde_json::json!({"name": "sid", "value": "stale"})];
        remote.session_updated_at = Some(t);
        remote.meta_updated_at = Some(t + Duration::seconds(5)); // tagged there
        remote.updated_at = t + Duration::seconds(5);
        remote.last_used_at = Some(t + Duration::seconds(20));

        let r = v.merge(vec![remote], &BTreeMap::new());
        assert_eq!(r.merged, vec!["a"]);
        let a = v.find("a").unwrap();
        assert_eq!(a.tags, vec!["prod"], "remote's newer tags");
        assert_eq!(a.cookies[0]["value"], "fresh", "our newer login");
        assert_eq!(a.last_used_at, Some(t + Duration::seconds(20)));
        assert_eq!(a.updated_at, t + Duration::seconds(10));
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
