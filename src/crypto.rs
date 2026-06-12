//! AES-256-GCM encryption for the vault blob. The on-disk format is
//! `nonce(12) || ciphertext`, base64-encoded by the caller.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Result};
use rand::RngCore;

pub fn generate_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    k
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| anyhow!("encrypt: {e}"))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 12 {
        return Err(anyhow!("vault blob too short"));
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| anyhow!("could not decrypt the vault (wrong key or corrupt file)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = generate_key();
        let pt = br#"[{"name":"x","value":"secret"}]"#;
        let blob = encrypt(&key, pt).unwrap();
        assert_ne!(&blob[12..], &pt[..]); // actually encrypted
        assert_eq!(decrypt(&key, &blob).unwrap(), pt);
    }

    #[test]
    fn wrong_key_fails() {
        let blob = encrypt(&generate_key(), b"hello").unwrap();
        assert!(decrypt(&generate_key(), &blob).is_err());
    }
}
