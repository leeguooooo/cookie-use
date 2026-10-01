//! `cloud` — sync the vault through a CookieCloud server
//! (<https://github.com/easychen/CookieCloud>), wire-compatible with it.
//!
//! The server only stores `{uuid, encrypted, crypto_type}`; everything is
//! encrypted client-side with CookieCloud's scheme (key = first 16 hex chars of
//! `MD5(uuid + "-" + password)`, either `aes-128-cbc-fixed` or CryptoJS
//! `legacy`). Inside, the payload is CookieCloud's own shape —
//! `{cookie_data, local_storage_data, update_time}` — plus a `cookie_use` field
//! holding the full multi-account vault as a v2 bundle sealed again with
//! argon2id + AES-GCM, because CookieCloud's MD5-derived key is weak and its
//! format keeps only one login per domain.
//!
//! So: any CookieCloud server works (self-hosted Docker or a public one), the
//! CookieCloud browser extension can download what cookie-use pushes
//! (`browser_compat`: the most recently used account per site), and accounts
//! can be imported from what the extension uploaded (`cloud import`).

use crate::share::{seal_accounts, unseal_any, Payload};
use crate::vault::{Account, CloudConfig, Vault};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::Utc;
use md5::{Digest, Md5};
use rand::{Rng, RngCore};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

pub const FIXED: &str = "aes-128-cbc-fixed";
pub const LEGACY: &str = "legacy";

// ---------------------------------------------------------------------------
// CookieCloud crypto
// ---------------------------------------------------------------------------

fn the_key(uuid: &str, password: &str) -> String {
    let hex = format!("{:x}", Md5::digest(format!("{uuid}-{password}").as_bytes()));
    hex[..16].to_string()
}

/// OpenSSL `EVP_BytesToKey` (MD5, 1 round) as CryptoJS uses for passphrases.
fn evp_bytes_to_key(pass: &[u8], salt: &[u8]) -> ([u8; 32], [u8; 16]) {
    let mut out = Vec::with_capacity(48);
    let mut prev: Vec<u8> = Vec::new();
    while out.len() < 48 {
        let mut h = Md5::new();
        h.update(&prev);
        h.update(pass);
        h.update(salt);
        prev = h.finalize().to_vec();
        out.extend_from_slice(&prev);
    }
    let mut key = [0u8; 32];
    let mut iv = [0u8; 16];
    key.copy_from_slice(&out[..32]);
    iv.copy_from_slice(&out[32..48]);
    (key, iv)
}

pub fn encrypt(uuid: &str, password: &str, plaintext: &[u8], crypto_type: &str) -> Result<String> {
    let k = the_key(uuid, password);
    match crypto_type {
        FIXED => {
            let ct = cbc::Encryptor::<aes::Aes128>::new(k.as_bytes().into(), &[0u8; 16].into())
                .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
            Ok(B64.encode(ct))
        }
        LEGACY => {
            let mut salt = [0u8; 8];
            rand::thread_rng().fill_bytes(&mut salt);
            let (key, iv) = evp_bytes_to_key(k.as_bytes(), &salt);
            let ct = cbc::Encryptor::<aes::Aes256>::new(&key.into(), &iv.into())
                .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
            let mut blob = b"Salted__".to_vec();
            blob.extend_from_slice(&salt);
            blob.extend_from_slice(&ct);
            Ok(B64.encode(blob))
        }
        other => bail!("unknown crypto_type \"{other}\" (use {FIXED} or {LEGACY})"),
    }
}

pub fn decrypt(uuid: &str, password: &str, encrypted: &str, crypto_type: &str) -> Result<Vec<u8>> {
    let k = the_key(uuid, password);
    let raw = B64
        .decode(encrypted.trim())
        .context("decoding CookieCloud data")?;
    let wrong = || anyhow!("wrong uuid/password, or not CookieCloud data");
    match crypto_type {
        FIXED => cbc::Decryptor::<aes::Aes128>::new(k.as_bytes().into(), &[0u8; 16].into())
            .decrypt_padded_vec_mut::<Pkcs7>(&raw)
            .map_err(|_| wrong()),
        LEGACY => {
            if raw.len() < 16 || &raw[..8] != b"Salted__" {
                return Err(wrong());
            }
            let (key, iv) = evp_bytes_to_key(k.as_bytes(), &raw[8..16]);
            cbc::Decryptor::<aes::Aes256>::new(&key.into(), &iv.into())
                .decrypt_padded_vec_mut::<Pkcs7>(&raw[16..])
                .map_err(|_| wrong())
        }
        other => bail!("unknown crypto_type \"{other}\""),
    }
}

// ---------------------------------------------------------------------------
// Cookie shape conversion (CDP ↔ chrome.cookies, which CookieCloud stores)
// ---------------------------------------------------------------------------

fn to_extension_cookie(c: &Value) -> Option<Value> {
    let o = c.as_object()?;
    let domain = o.get("domain")?.as_str()?;
    let expires = o.get("expires").and_then(Value::as_f64).unwrap_or(-1.0);
    let same_site = match o
        .get("sameSite")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("none") => "no_restriction",
        Some("lax") => "lax",
        Some("strict") => "strict",
        _ => "unspecified",
    };
    let mut m = json!({
        "name": o.get("name")?,
        "value": o.get("value").cloned().unwrap_or(json!("")),
        "domain": domain,
        "path": o.get("path").cloned().unwrap_or(json!("/")),
        "secure": o.get("secure").cloned().unwrap_or(json!(false)),
        "httpOnly": o.get("httpOnly").cloned().unwrap_or(json!(false)),
        "sameSite": same_site,
        "hostOnly": !domain.starts_with('.'),
        "session": expires <= 0.0,
        "storeId": "0",
    });
    if expires > 0.0 {
        m["expirationDate"] = json!(expires);
    }
    Some(m)
}

fn from_extension_cookie(c: &Value) -> Option<Value> {
    let o = c.as_object()?;
    let same_site = match o.get("sameSite").and_then(Value::as_str) {
        Some("no_restriction") => Some("None"),
        Some("lax") => Some("Lax"),
        Some("strict") => Some("Strict"),
        _ => None,
    };
    let mut m = json!({
        "name": o.get("name")?,
        "value": o.get("value").cloned().unwrap_or(json!("")),
        "domain": o.get("domain")?,
        "path": o.get("path").cloned().unwrap_or(json!("/")),
        "secure": o.get("secure").cloned().unwrap_or(json!(false)),
        "httpOnly": o.get("httpOnly").cloned().unwrap_or(json!(false)),
        // Keep the real expiry (the CookieCloud extension itself drops it on
        // download, turning every cookie into a session cookie).
        "expires": o.get("expirationDate").and_then(Value::as_f64).unwrap_or(-1.0),
    });
    if let Some(s) = same_site {
        m["sameSite"] = json!(s);
    }
    Some(m)
}

/// CookieCloud's `cookie_data` / `local_storage_data` for the most recently
/// used account of each site — what the CookieCloud extension can download.
fn browser_view(accounts: &[Account]) -> (Map<String, Value>, Map<String, Value>) {
    let mut latest: BTreeMap<String, &Account> = BTreeMap::new();
    for a in accounts
        .iter()
        .filter(|a| !a.tags.iter().any(|t| t == "backup"))
    {
        let site = crate::vault::landing_host(&a.site);
        let stamp = |x: &Account| x.last_used_at.unwrap_or(x.updated_at);
        match latest.get(&site) {
            Some(cur) if stamp(cur) >= stamp(a) => {}
            _ => {
                latest.insert(site, a);
            }
        }
    }
    let mut cookies: Map<String, Value> = Map::new();
    let mut storage: Map<String, Value> = Map::new();
    for (host, a) in latest {
        for c in a.cookies.iter().filter_map(to_extension_cookie) {
            let d = c["domain"].as_str().unwrap_or_default().to_string();
            let slot = cookies.entry(d).or_insert_with(|| json!([]));
            if let Some(arr) = slot.as_array_mut() {
                arr.push(c);
            }
        }
        if let Some(ls) = a.local_storage.as_ref().filter(|m| !m.is_empty()) {
            storage.insert(host, Value::Object(ls.clone()));
        }
    }
    (cookies, storage)
}

// ---------------------------------------------------------------------------
// HTTP (curl, like `upgrade`: no TLS stack in the binary)
// ---------------------------------------------------------------------------

fn http(method: &str, url: &str, body: Option<&[u8]>) -> Result<(u16, Vec<u8>)> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--max-time",
        "60",
        "-X",
        method,
        "-w",
        "\n%{http_code}",
    ]);
    if body.is_some() {
        cmd.args([
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ]);
    }
    cmd.arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("running curl")?;
    if let Some(b) = body {
        child.stdin.take().expect("piped").write_all(b)?;
    } else {
        drop(child.stdin.take());
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "can't reach {url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = out.stdout;
    let split = text.iter().rposition(|b| *b == b'\n').unwrap_or(0);
    let code = String::from_utf8_lossy(&text[split..])
        .trim()
        .parse()
        .unwrap_or(0);
    Ok((code, text[..split].to_vec()))
}

fn endpoint(cfg: &CloudConfig) -> String {
    cfg.endpoint.trim().trim_end_matches('/').to_string()
}

/// The decrypted remote payload, or `None` when nothing was uploaded yet.
fn fetch(cfg: &CloudConfig) -> Result<Option<Value>> {
    let (code, body) = http("GET", &format!("{}/get/{}", endpoint(cfg), cfg.uuid), None)?;
    if code == 404 {
        return Ok(None);
    }
    if code != 200 {
        bail!(
            "server answered {code} for /get: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }
    let v: Value = serde_json::from_slice(&body)
        .context("server reply is not JSON — is this a CookieCloud endpoint?")?;
    let Some(enc) = v.get("encrypted").and_then(Value::as_str) else {
        return Ok(None);
    };
    let ct = v
        .get("crypto_type")
        .and_then(Value::as_str)
        .unwrap_or(LEGACY);
    let plain = decrypt(&cfg.uuid, &cfg.password, enc, ct)?;
    Ok(Some(
        serde_json::from_slice(&plain).context("decrypted data is not JSON")?,
    ))
}

fn upload(cfg: &CloudConfig, payload: &Value) -> Result<()> {
    let encrypted = encrypt(
        &cfg.uuid,
        &cfg.password,
        payload.to_string().as_bytes(),
        &cfg.crypto_type,
    )?;
    let body = json!({ "uuid": cfg.uuid, "encrypted": encrypted, "crypto_type": cfg.crypto_type });
    let (code, reply) = http(
        "POST",
        &format!("{}/update", endpoint(cfg)),
        Some(body.to_string().as_bytes()),
    )?;
    let ok = code == 200
        && serde_json::from_slice::<Value>(&reply)
            .ok()
            .and_then(|v| v.get("action").cloned())
            == Some(json!("done"));
    if !ok {
        bail!(
            "upload failed ({code}): {}",
            String::from_utf8_lossy(&reply).trim()
        );
    }
    Ok(())
}

/// The cookie-use vault inside a remote payload, if this payload has one.
fn remote_vault(remote: &Value, password: &str) -> Result<Option<Payload>> {
    match remote.pointer("/cookie_use/bundle") {
        Some(b) => Ok(Some(unseal_any(b.to_string().as_bytes(), password)?)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn need(vault: &Vault) -> Result<CloudConfig> {
    vault.cloud().cloned().ok_or_else(|| {
        anyhow!("cloud sync isn't set up — run `cookie-use cloud setup --endpoint <url>`")
    })
}

/// CookieCloud-style 22-char base62 id.
fn random_token() -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut r = rand::thread_rng();
    (0..22)
        .map(|_| A[r.gen_range(0..A.len())] as char)
        .collect()
}

fn print(json_mode: bool, v: Value, human: impl FnOnce()) -> Result<()> {
    if json_mode {
        println!("{}", serde_json::to_string(&v)?);
    } else {
        human();
    }
    Ok(())
}

pub fn cmd_setup(
    endpoint_url: &str,
    uuid: Option<String>,
    password: Option<String>,
    crypto_type: &str,
    browser_compat: bool,
    json_mode: bool,
) -> Result<()> {
    if !matches!(crypto_type, FIXED | LEGACY) {
        bail!("--crypto must be {FIXED} or {LEGACY}");
    }
    let url = endpoint_url.trim().trim_end_matches('/');
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("--endpoint must be an http(s) URL, e.g. https://cookiecloud.example.com");
    }
    let cfg = CloudConfig {
        endpoint: url.to_string(),
        uuid: uuid.unwrap_or_else(random_token),
        password: password.unwrap_or_else(random_token),
        crypto_type: crypto_type.to_string(),
        browser_compat,
        last_push: None,
        last_pull: None,
    };
    let mut vault = Vault::open()?;
    vault.set_cloud(Some(cfg.clone()));
    vault.save()?;
    print(
        json_mode,
        json!({ "endpoint": cfg.endpoint, "uuid": cfg.uuid, "password": cfg.password, "crypto_type": cfg.crypto_type }),
        || {
            println!("cloud sync set up for {}", cfg.endpoint);
            println!("  uuid:     {}", cfg.uuid);
            println!("  password: {}", cfg.password);
            println!("Use the same three values on your other computers (`cookie-use cloud setup --endpoint … --uuid … --password …`)");
            println!("or in the CookieCloud browser extension. Then run `cookie-use cloud sync`.");
        },
    )
}

pub fn cmd_status(json_mode: bool) -> Result<()> {
    let vault = Vault::open()?;
    let Some(cfg) = vault.cloud() else {
        return print(json_mode, json!({ "configured": false }), || {
            println!("cloud sync is not set up")
        });
    };
    print(
        json_mode,
        json!({
            "configured": true, "endpoint": cfg.endpoint, "uuid": cfg.uuid,
            "crypto_type": cfg.crypto_type, "browser_compat": cfg.browser_compat,
            "last_push": cfg.last_push, "last_pull": cfg.last_pull,
        }),
        || {
            println!("endpoint:  {}", cfg.endpoint);
            println!("uuid:      {}", cfg.uuid);
            println!("crypto:    {}", cfg.crypto_type);
            println!(
                "last push: {}",
                cfg.last_push
                    .map(|t| t.to_rfc3339())
                    .unwrap_or("never".into())
            );
            println!(
                "last pull: {}",
                cfg.last_pull
                    .map(|t| t.to_rfc3339())
                    .unwrap_or("never".into())
            );
        },
    )
}

pub fn cmd_show_secret(json_mode: bool) -> Result<()> {
    let cfg = need(&Vault::open()?)?;
    print(
        json_mode,
        json!({ "uuid": cfg.uuid, "password": cfg.password }),
        || {
            println!("uuid:     {}", cfg.uuid);
            println!("password: {}", cfg.password);
        },
    )
}

pub fn cmd_disconnect(json_mode: bool) -> Result<()> {
    let mut vault = Vault::open()?;
    vault.set_cloud(None);
    vault.save()?;
    print(json_mode, json!({ "configured": false }), || {
        println!("cloud sync turned off (the server copy is left as is)")
    })
}

/// Pull, merge, and (unless `pull_only`) push the merged vault back.
pub fn cmd_sync(pull_only: bool, force: bool, json_mode: bool) -> Result<()> {
    let mut vault = Vault::open()?;
    let mut cfg = need(&vault)?;
    let remote = fetch(&cfg)?;

    let mut report = crate::vault::MergeReport::default();
    let mut browser_only = false;
    if let Some(r) = &remote {
        match remote_vault(r, &cfg.password)? {
            Some(p) => report = vault.merge(p.accounts, &p.deleted),
            None => browser_only = true,
        }
    }
    cfg.last_pull = Some(Utc::now());

    let mut pushed = false;
    if !pull_only {
        if browser_only && !force {
            bail!(
                "this uuid holds data uploaded by the CookieCloud browser extension — pushing would \
                 replace it. Import from it with `cookie-use cloud import <domain> --id <id>`, set up a \
                 separate uuid for cookie-use, or pass --force"
            );
        }
        let bundle: Value = serde_json::from_slice(&seal_accounts(
            &Payload {
                accounts: vault.accounts().to_vec(),
                deleted: vault.deleted().clone(),
            },
            &cfg.password,
        )?)?;
        let (cookie_data, local_storage_data) = if cfg.browser_compat {
            browser_view(vault.accounts())
        } else {
            (Map::new(), Map::new())
        };
        let payload = json!({
            "cookie_data": cookie_data,
            "local_storage_data": local_storage_data,
            "update_time": Utc::now().to_rfc3339(),
            "cookie_use": { "version": 1, "bundle": bundle },
        });
        upload(&cfg, &payload)?;
        cfg.last_push = Some(Utc::now());
        pushed = true;
    }
    vault.set_cloud(Some(cfg));
    vault.save()?;

    let accounts = vault.accounts().len();
    print(
        json_mode,
        json!({
            "added": report.added, "updated": report.updated, "removed": report.removed,
            "unchanged": report.unchanged, "pushed": pushed, "accounts": accounts,
            "remote_browser_only": browser_only,
        }),
        || {
            println!(
                "pulled: {} new, {} updated, {} removed, {} unchanged",
                report.added.len(),
                report.updated.len(),
                report.removed.len(),
                report.unchanged
            );
            if browser_only {
                println!("note: the server copy came from the CookieCloud extension — see `cookie-use cloud domains`");
            }
            if pushed {
                println!("pushed {accounts} account(s)");
            }
        },
    )
}

fn remote_cookie_data(cfg: &CloudConfig) -> Result<Map<String, Value>> {
    let remote = fetch(cfg)?.ok_or_else(|| anyhow!("nothing uploaded under this uuid yet"))?;
    Ok(remote
        .get("cookie_data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default())
}

/// Domains in the remote `cookie_data` (what the CookieCloud extension uploaded).
pub fn cmd_domains(json_mode: bool) -> Result<()> {
    let cfg = need(&Vault::open()?)?;
    let data = remote_cookie_data(&cfg)?;
    let rows: Vec<(String, usize)> = data
        .iter()
        .map(|(d, v)| (d.clone(), v.as_array().map(Vec::len).unwrap_or(0)))
        .collect();
    print(
        json_mode,
        json!({ "domains": rows.iter().map(|(d, n)| json!({"domain": d, "cookies": n})).collect::<Vec<_>>() }),
        || {
            for (d, n) in &rows {
                println!("{d:<40} {n:>4}");
            }
        },
    )
}

/// Turn the remote `cookie_data` for a site into a vault account.
pub fn cmd_import(site: &str, id: &str, label: Option<String>, json_mode: bool) -> Result<()> {
    let mut vault = Vault::open()?;
    let cfg = need(&vault)?;
    let data = remote_cookie_data(&cfg)?;
    let hosts: Vec<String> = site
        .split(',')
        .map(|h| h.trim().trim_start_matches('.').to_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    let cookies: Vec<Value> = data
        .iter()
        .flat_map(|(_, list)| list.as_array().cloned().unwrap_or_default())
        .filter(|c| {
            let d = c
                .get("domain")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_start_matches('.')
                .to_lowercase();
            hosts
                .iter()
                .any(|h| d == *h || d.ends_with(&format!(".{h}")) || h.ends_with(&format!(".{d}")))
        })
        .filter_map(|c| from_extension_cookie(&c))
        .collect();
    if cookies.is_empty() {
        bail!("no cookies for {site} on the server (see `cookie-use cloud domains`)");
    }
    let n = cookies.len();
    crate::store(&mut vault, id.to_string(), site, cookies, None, label, None)?;
    vault.save()?;
    print(
        json_mode,
        json!({ "id": id, "site": site, "cookies": n }),
        || println!("imported \"{id}\" ({site}) — {n} cookie(s) from CookieCloud"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Generated with crypto-js 4 exactly as the CookieCloud extension does.
    const UUID: &str = "jNp1T2qZ6shwVW9VmjLvp1";
    const PW: &str = "iZ4PCqzfJcHyiwAQcCuupD";
    const DATA: &str = r#"{"cookie_data":{".x.com":[{"name":"sid","value":"v1","domain":".x.com","path":"/"}]},"local_storage_data":{},"update_time":"2026-10-02T00:00:00.000Z"}"#;
    const FIXED_CT: &str = "4Z2Iab4HqvND8iT3iloKC9cM3HWrJoRvp0xXhoCWq66WC8Gk9ohKwL2PaIh+1AmvlxpI7pFG1WgndzPJC/R9mNmHdVV3T59/zYwHSHrKLV007cAqEf9kUEKj+54MkMTxdCfXSj1+RK5/xn6/GkvddKgiQUGhnFmBLw6UYG6DcK+sn/w0nPPzJa2/gIdx/9pwkmCekAsclRuG4kDzvKHFSw==";
    const LEGACY_CT: &str = "U2FsdGVkX19gPqpgHzxjRcL/ExCdDTVFHMAuRvmTEwfsPturKgf/bl11hjoc6r+nkGoM3ueF03gOROL0dBT1zLUjB8oDJc+fIvKEOrqCToqyNufryqXLycHe5S0hiaOfMwmXp45coGoCHartbn49wX3Rrng220EdUKfX2JT6FQtDJZE3oReNXvDu9wWOAB9UdqM2iNrH8OJPrcnXnUf4jlNXHCiaZ/IhROdH2rC/Ta8=";

    #[test]
    fn fixed_iv_matches_crypto_js_byte_for_byte() {
        assert_eq!(encrypt(UUID, PW, DATA.as_bytes(), FIXED).unwrap(), FIXED_CT);
        assert_eq!(decrypt(UUID, PW, FIXED_CT, FIXED).unwrap(), DATA.as_bytes());
    }

    #[test]
    fn legacy_decrypts_crypto_js_and_round_trips() {
        assert_eq!(
            decrypt(UUID, PW, LEGACY_CT, LEGACY).unwrap(),
            DATA.as_bytes()
        );
        let ct = encrypt(UUID, PW, DATA.as_bytes(), LEGACY).unwrap();
        assert!(ct.starts_with("U2FsdGVkX1")); // "Salted__"
        assert_eq!(decrypt(UUID, PW, &ct, LEGACY).unwrap(), DATA.as_bytes());
    }

    #[test]
    fn wrong_password_is_an_error_not_garbage() {
        assert!(decrypt(UUID, "nope", FIXED_CT, FIXED).is_err());
        assert!(decrypt(UUID, "nope", LEGACY_CT, LEGACY).is_err());
    }

    #[test]
    fn cookie_shapes_round_trip_and_keep_expiry() {
        let cdp = json!({"name":"sid","value":"v","domain":"app.x.com","path":"/","secure":true,
                         "httpOnly":true,"sameSite":"None","expires":1900000000.5});
        let ext = to_extension_cookie(&cdp).unwrap();
        assert_eq!(ext["sameSite"], "no_restriction");
        assert_eq!(ext["hostOnly"], true);
        assert_eq!(ext["expirationDate"], 1900000000.5);
        let back = from_extension_cookie(&ext).unwrap();
        assert_eq!(back["sameSite"], "None");
        assert_eq!(back["expires"], 1900000000.5);
        let session =
            to_extension_cookie(&json!({"name":"a","domain":".x.com","expires":-1})).unwrap();
        assert_eq!(session["session"], true);
        assert!(session.get("expirationDate").is_none());
    }
}
