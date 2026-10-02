# Changelog

All notable changes to cookie-use are documented here. Versions follow semver.

## [Unreleased]

### Added
- **`verify`**: tests whether a saved session still signs in, instead of only
  trusting cookie expiry. It replays the session in one throwaway, off-screen
  browser and compares against an anonymous visit — landing on a sign-in page
  means the login is dead. Site-agnostic. The result (`valid`/`invalid`) is
  written back and surfaced in `list`/`show --json`.
- App: a "Verify logins" toolbar button and per-account "Check", an honest
  status — a hollow grey dot now means "cookies present, login not checked",
  green means a login confirmed by replay, red means confirmed dead — and
  "Needs attention" includes confirmed-dead logins.

### Changed
- **Sync never lets a dead login overwrite a working one.** If one computer
  re-captured a logged-out profile (a newer but dead session), a sync keeps the
  other computer's session that still works, instead of taking the newer copy
  (reported as `kept_working`).

## [0.10.0] - 2026-10-02

### Changed
- Sync follows renames: an edit to the old id made on another computer lands
  on the renamed account instead of resurrecting the old id as a duplicate.
  Renames are recorded in the vault and travel with sync and `export`.
- Edits are stamped strictly after the version they change, so a computer
  whose clock lags still wins with edits made after seeing another's.

## [0.9.0] - 2026-10-02

### Changed
- **Sync conflicts between computers are merged, not overwritten.** Each
  account's login (cookies, localStorage) and metadata (label, hint, note,
  tags) carry their own timestamps and merge separately, so editing tags on
  one Mac no longer discards a login refreshed on another (reported as
  `merged`). `last_used_at` keeps the latest.
- The vault is locked (`vault.lock`) while read or written; `cloud sync`
  never holds it across the network and merges into the current vault, so a
  CLI/agent write during a sync is kept.
- CookieCloud pushes are read back and retried if another upload landed on top.

### Added
- Snapshots before a sync changes the vault (last 10) with `cloud backups` and
  `cloud restore <name>`; a restore wins on the next sync. App: Restore… menu
  in Sync between Macs.

## [0.8.0] - 2026-10-02

### Added
- **Sync through a private GitHub repo — no server**:
  `cloud setup --github <owner/repo> [--create]`. The vault is one sealed
  `.cusession` file (argon2id + AES-GCM; `redeem`-able by hand) committed via
  the user's `gh` login. Public repos are refused; concurrent pushes are
  caught by GitHub's sha check and retried after a re-merge.
- App: "Sync between Macs" offers GitHub (repo prefilled from the `gh` login,
  created private on request) or a CookieCloud server.

## [0.7.0] - 2026-10-02

### Added
- **`copy --site <d> --from <profile> --to <profile>`**: overwrite one Chrome
  profile's login for a site with another's. Only that site's cookies change
  (stale ones are expired one by one; other sites untouched), the destination's
  previous login is saved to the vault first (tag `backup`) for undo, and
  `--dry-run` shows the counts. Profiles resolve by directory, name or email
  from Chrome's Local State; a new `browser:<email>` target pins writes to one
  connected profile.
- **`export`**: many accounts (all, `--site`, or ids) in one encrypted v2
  bundle; `redeem` merges it (newer copy of each account wins).
- **`cloud`: CookieCloud-compatible sync** (`setup`, `sync`, `pull`, `status`,
  `secret`, `domains`, `import`, `disconnect`). Works with any CookieCloud
  server; both CookieCloud encryptions (verified byte-for-byte against
  crypto-js); the multi-account vault is sealed again with argon2id + AES-GCM;
  the latest login per site is also published in CookieCloud's `cookie_data`
  for the browser extension, and extension uploads can be imported with their
  real expiry. Deletes and renames propagate.
- App: Copy between profiles (preview, Undo), Export logins, multi-account
  import preview, Cloud sync settings with automatic sync.

### Changed
- `install-app.sh` installs without sudo, so the app is owned by you and
  re-running it upgrades with no password. sudo is used once, only to remove a
  root-owned copy left by the old installer. `COOKIE_USE_APP_DIR` picks another
  folder (e.g. `~/Applications`). It launches the app when done.

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
