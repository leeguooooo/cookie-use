# Changelog

All notable changes to cookie-use are documented here. Versions follow semver.

## [Unreleased]

## [0.6.1] - 2026-10-01

### Changed
- **CookieUse.dmg is Developer ID signed and notarized by Apple** (app and dmg
  stapled), so it opens on a double-click with no "unidentified developer"
  warning. The release workflow signs in a throwaway keychain and notarizes
  with an App Store Connect API key; `app/build-dmg.sh` does the same locally
  when `SIGNING_IDENTITY` is set and stays ad-hoc otherwise.
- `install-app.sh` verifies the dmg's new `.sha256` and refuses an app
  Gatekeeper rejects, instead of stripping quarantine attributes.

## [0.6.0] - 2026-10-01

### Fixed
- **`switch` no longer signs you out of every website.** It called chrome-use
  `cookies clear` (CDP `Network.clearBrowserCookies`), which wipes the whole
  browser. It now expires only the cookies stored for that site (from any
  account that shares its domains), so other sites stay signed in.
- Opening an account whose site lists a subdomain of its first host
  (`cloudflare.com,dash.cloudflare.com`) lands on the subdomain (the
  dashboard), not the marketing page. localStorage is captured and injected
  on the same origin.

### Added
- `edit <id> [--label] [--hint] [--note] [--tags]` and per-account notes and
  tags (searchable through `list`).
- `list --json` rows carry `note`, `tags`, `live_until` (when the expiry
  heuristic flips to expired) and `updated_at`.
- Re-capturing an existing id keeps its label, hint, note, tags and
  last-used time.
- **CookieUse.app 0.2.0, rebuilt**: ⌥⌘K keyboard quick switcher with
  pinned and recent accounts; capture that probes every Chrome profile for
  the site (paste a URL or take Chrome's front tab); manager with sites,
  tags, accounts that need attention, local-only favicons, editable
  metadata, refresh login, side-by-side windows; `.cusession` document type
  plus drag-and-drop import; Touch ID unlock window; launch at login; live
  refresh when the vault changes.
- App fixes: chrome-use's current `session list` JSON was not parsed, so the
  app always showed "not connected" and disabled Switch; when launched from
  Finder, cookie-use couldn't find chrome-use on PATH.

## [0.5.0] - 2026-09-29

### Changed
- **`upgrade` updates only the CLI by default; `--skills` opts in to refreshing
  the skill** (plugin / git checkout). Without it the skill copies are listed
  with their refresh command and left alone.
- **`install.sh` requires the `.sha256` sidecar** (a missing or malformed one
  fails the install), verifies it before unpacking, and runs the new binary
  before swapping it in.

### Added
- `upgrade --tag vX.Y.Z` and `COOKIE_USE_VERSION=vX.Y.Z` for install.sh: install
  one exact release; the new binary must report that version or nothing is
  replaced.
- `upgrade` detects how the CLI was installed and refuses to replace a
  Homebrew, cargo, npm or source-build binary (exit 1, prints the right
  command). `--json` gains `install_channel`, `target` and `app` (the
  separately installed CookieUse.app version, never touched).

## [0.4.0] - 2026-09-28

### Added
- **`upgrade` / `upgrade --check` / `upgrade --json`** (the *-use family
  convention). `upgrade` reinstalls the latest GitHub release through
  `install.sh` into the running binary's directory, then refreshes the skill:
  Claude Code plugin (`claude plugin update`), git checkout (`git pull
  --ff-only`), or prints `npx skills update cookie-use` for a copied folder.
  `--check` / `--json` change nothing. Exit 2 when the check or download fails.
- **Daily "new version" notice** on stderr, at most once a day (cache in
  `${XDG_CACHE_HOME:-~/.cache}/cookie-use/update-check.json`, 2 s timeout in a
  detached child). Off with `CI`, `COOKIE_USE_NO_UPDATE_CHECK` or
  `USE_NO_UPDATE_CHECK`.
- `install.sh` honours `COOKIE_USE_BIN_DIR`.

## [0.3.0] - 2026-07-02

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
