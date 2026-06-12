# cookie-use

**Agent-friendly multi-account session manager.** Capture, store, and apply
logged-in sessions for *any* website — across browsers, profiles, and isolated
contexts. Built for the case where you have many accounts on one site (e.g. 100
ChatGPT accounts) and want an agent to freely list, pick, and switch between
them.

cookie-use is **site-agnostic**: you name a domain, it manages that domain's
session. Nothing about ChatGPT, Claude, or any specific site is hardcoded.

It sits on top of [`chrome-use`](https://github.com/leeguooooo/chrome-use): all
browser and cookie I/O (decrypting a profile's cookies, injecting into a live
browser, launching isolated contexts) is delegated to `chrome-use`. cookie-use
owns the **account model**, an **encrypted vault**, and the **orchestration**.

> macOS first (uses the Keychain for the vault key and `chrome-use`'s macOS
> cookie decryption). Other platforms are a follow-up.

## Mental model

```
        capture                       apply
profiles ─┐                      ┌─► real Chrome profile  (via chrome-use)
files ────┼─► [ encrypted vault ]┼─► isolated context     (fresh browser)
browser ──┘     N accounts/site  └─► connected session    (chrome-use --session)
```

An **account** is one stored session for one site: its full cross-domain cookie
set plus metadata (id, site, label, account hint, timestamps, status). The vault
holds many accounts across many sites, encrypted at rest.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/leeguooooo/cookie-use/main/install.sh | sh
```

Requires `chrome-use` on PATH (`curl -fsSL https://raw.githubusercontent.com/leeguooooo/chrome-use/main/install.sh | sh`).

### As an agent skill (skills.sh)

Install the cookie-use skill into your agent so it knows how to drive the CLI
(it self-heals the binary on first use):

```sh
npx skills add leeguooooo/cookie-use
```

See <https://www.skills.sh/docs>. The skill lives at `skills/cookie-use/SKILL.md`.

## Commands (v0.1 MVP)

| Command | Does |
|---|---|
| `cookie-use add --from-profile <profile> --site <domain[,domain]> [--id <id>]` | Import a logged-in session from a Chrome profile (any site) |
| `cookie-use import --file <f> --site <domain> --id <id>` | Import from a JSON / cURL / Cookie-header export |
| `cookie-use list [--site <domain>]` | List stored accounts (id, site, hint, status, last used) |
| `cookie-use show <id>` | Account metadata (never prints cookie values) |
| `cookie-use use <id> [--target session:<s>\|isolated\|profile:<p>]` | Apply an account into a browser target |
| `cookie-use switch <id> --target <…>` | Clear the site's cookies in the target, then apply (clean switch) |
| `cookie-use check <id>` | Liveness from cookie expiry (generic; site probes are pluggable later) |
| `cookie-use rm <id>` / `rename <id> <new>` | Manage entries |

`--site` accepts a comma-separated domain list so multi-host auth (e.g.
`chatgpt.com,openai.com`) is captured as one account. Suffix matching also
catches subdomains.

## Vault & security

- Location: `~/.cookie-use/vault.json` (AES-256-GCM encrypted blob).
- Master key: generated on first run, stored in the macOS Keychain
  (`cookie-use vault key`). Cookie values never touch disk in plaintext.
- `show` / `list` never print secret values; errors never echo them.

## Roadmap (post-MVP)

1. Interactive capture: `capture` (log in once → snapshot) and `grab` (pull the
   current account out of a running browser via the chrome-use extension).
2. `run --site <d> --all -- <cmd>`: concurrent orchestration over isolated
   contexts (operate dozens of accounts in parallel).
3. Generalized headless / direct-API backend.
4. MCP server wrapping the same core for agents.
5. Anti-correlation: per-account proxy + fingerprint binding (the vault already
   reserves `proxy` / `fingerprint` fields).

## Relationship to chrome-use

cookie-use shells out to `chrome-use`. As it stabilizes, the shared cookie/crypto
engine may be extracted into a common crate used by both.
