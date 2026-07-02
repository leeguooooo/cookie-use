# Changelog

All notable changes to cookie-use are documented here. Versions follow semver.

## [Unreleased]

### Added
- **`fingerprint <id>` / `fingerprint --all`** — export a **hash-only** fingerprint
  of an account's session cookies (SHA-256 of each cookie value, never the value)
  so a separate tool such as `chrome-use` can verify "is the live browser session
  logged in as this account?" without ever seeing a secret. `--json` emits the
  agreed contract (single id → one account object like `show`; `--all` →
  `{"accounts":[…]}` like `list`); human mode prints counts only (id, site, cookie
  count, httpOnly/secure) — never values, never hashes. Cookie values shorter than
  8 chars are excluded (low-entropy, useless as identity). Fingerprints are cached
  in a **plaintext** sidecar (`~/.cookie-use/fingerprints.json`, hashes + names
  only) written automatically on `add` / `import` / `use` / `switch`, so
  `fingerprint` reads need no Keychain access or vault decrypt; `--all` uses the
  cache and skips (listing on stderr) any account without a cached fingerprint.

### Changed
- **`list` now takes the website as a positional argument and accepts a full URL.**
  `cookie-use list https://dash.cloudflare.com/` (or `cookie-use list dash.cloudflare.com`)
  now works — previously `list` only had a `--site <domain>` flag and rejected a
  positional URL with "unexpected argument". URLs are normalized to their host
  (scheme/path/query/port stripped). The old `--site` flag still works as a
  deprecated alias.
- **`list` matching is forgiving.** The term matches base-domain ↔ subdomain in
  either direction (`cloudflare.com` finds `dash.cloudflare.com` accounts and
  vice-versa), partial terms (`cloudflare`), and also searches the account id /
  label / hint (so `cookie-use list leo` or `… wind` works).
- **`list` output is grouped by website**, with accounts listed (sorted by id)
  under each site header — easier to scan a vault of many accounts per site.

## [0.2.1] - 2026-06-17

### Security
- **Fixed an inverted confirmation gate in `as`.** Due to a flipped boolean in
  dispatch, `cookie-use as <id> -- <cmd>` *skipped* the Touch ID / TTY injection
  gate by default and *demanded* it when `--no-confirm` was passed — the exact
  opposite of intended. The default path now correctly gates (and refuses to
  inject in a non-interactive shell without `--no-confirm` / `COOKIE_USE_YES`).
  Anyone on 0.2.0 should upgrade.

### Added
- **Headless key/path overrides.** `COOKIE_USE_VAULT_KEY` (base64 of 32 bytes)
  supplies the vault key directly, bypassing the macOS Keychain; `COOKIE_USE_VAULT`
  overrides the vault file location. Enables CI / headless / agent hosts and
  isolated integration tests.
- Binary-level integration test suite (`tests/cli.rs`), runnable anywhere.

### Changed
- `as` now uses clap `last = true` for its trailing command, so cookie-use's own
  flags (`--no-confirm`, `--target`) parse correctly before `--`.

### Hardening
- Touch ID Swift source is piped to `swift -` over stdin instead of a temp file,
  removing a predictable-path symlink/TOCTOU vector.
- Dropped the unmaintained `atty` crate (RUSTSEC-2021-0145) for std `IsTerminal`.
- `cargo audit` clean across all dependencies.

## [0.2.0] - 2026-06-17

### Added
- `share` / `redeem` — password-encrypted (`argon2id` + AES-256-GCM) `.cusession`
  session bundles to hand a login to a teammate; redeeming requires installing
  cookie-use. The plaintext-at-rest invariant is preserved (ciphertext only).
- `run` — open one or many accounts in side-by-side isolated browser windows.
- `as <id> -- <cmd>` — run a command in a session-scoped environment, so an agent
  can act as a specific stored account for a single task.
- `replay <id> --to <host:port>` — cross-origin QA sugar over `--rewrite-domain`
  + `--open-url`.
- `revoke` / `wipe` — remove one account / the entire vault.
- Touch ID injection gate on `use` / `switch` / `replay` / `as` (LocalAuthentication
  via Swift, TTY fallback, `COOKIE_USE_YES` bypass for agents).
- `show` now reports the soonest cookie expiry and a local-only storage banner.

## [0.1.0] - 2026-06-12

- Initial release: encrypted multi-account session vault with `add` / `import` /
  `list` / `show` / `check` / `use` / `switch` / `rename` / `rm`, on top of
  chrome-use.
