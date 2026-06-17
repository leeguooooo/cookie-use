//! `share` / `redeem`: hand a single session to a teammate as a
//! password-encrypted bundle. Every redeem requires installing cookie-use, so
//! each shared login is also an install invite (the product's viral loop).

use crate::vault::Vault;
use anyhow::Result;

/// Export account `id` as a password-encrypted `.cusession` bundle.
pub fn cmd_share(
    _vault: &Vault,
    _id: &str,
    _out: Option<&str>,
    _password: Option<&str>,
) -> Result<()> {
    anyhow::bail!("share: not yet implemented")
}

/// Import a `.cusession` bundle into the vault.
pub fn cmd_redeem(
    _vault: &mut Vault,
    _bundle_path: &str,
    _password: Option<&str>,
    _new_id: Option<&str>,
) -> Result<()> {
    anyhow::bail!("redeem: not yet implemented")
}
