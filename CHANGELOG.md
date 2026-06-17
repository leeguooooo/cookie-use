# Changelog

All notable changes to cookie-use are documented here. Versions follow semver.

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
