//! `as`: run a command in an environment scoped to one account's session. The
//! agent-facing moat — lets an agent assume a saved identity for a single task.

use crate::vault::Vault;
use anyhow::Result;

/// Apply account `id` into `target`, then run `command` with environment
/// variables describing the active session (so the child — often an agent
/// driving chrome-use — acts as that account).
pub fn cmd_as(
    _vault: &mut Vault,
    _id: &str,
    _target: &str,
    _command: &[String],
    _no_confirm: bool,
) -> Result<()> {
    anyhow::bail!("as: not yet implemented")
}
