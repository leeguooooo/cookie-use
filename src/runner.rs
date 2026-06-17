//! `run`: open one or more accounts in isolated browser windows at once, so the
//! same site can be driven under several identities side by side.

use crate::vault::Vault;
use anyhow::Result;

/// Open account(s) in isolated browsers. With a single `id`, opens that one.
/// With `all` (optionally narrowed by `site`), opens every matching account,
/// each in its own isolated context.
pub fn cmd_run(
    _vault: &mut Vault,
    _id: Option<&str>,
    _site: Option<&str>,
    _all: bool,
) -> Result<()> {
    anyhow::bail!("run: not yet implemented")
}
