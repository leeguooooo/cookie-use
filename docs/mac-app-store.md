# Mac App Store: evaluation

Status: **not shipping to the Mac App Store.** CookieUse ships as a Developer ID
signed, Apple-notarized dmg (v0.6.1+). This records why, and what a store build
would have to give up, so the question doesn't have to be re-researched.

## The constraint

Every Mac App Store app must run in the App Sandbox (guideline 2.4.5), and may
not download or execute code it doesn't ship (2.5.2). Review also rejects apps
that read other apps' private data (5.1.1). ChooseBrowser passes because
everything it does — LaunchServices, `NSWorkspace.open`, being the default
browser — is allowed in the sandbox. cookie-use's core is not.

## Feature by feature

| Capability | Today | In the sandbox |
|---|---|---|
| Drive `cookie-use` / `chrome-use` in `~/.local/bin` | `Process` | Blocked. Both would have to be embedded in the bundle, signed with `app-sandbox` + `inherit`, and run sandboxed too |
| One vault shared with the CLI and agents (`~/.cookie-use`) | the product's main differentiator | Vault moves into the container. Sharing needs an App Group container the CLI reads, which macOS 15+ guards with a per-app data-access prompt |
| Vault key in the login Keychain via `security` | shared with the CLI | Separate access group; the unsandboxed CLI can't read it |
| Capture: decrypt a Chrome profile's cookie store | reads `Cookies` + the "Chrome Safe Storage" key | Needs user-granted folder access *and* another app's Keychain secret. Decrypting another app's credentials is a near-certain 5.1.1 rejection |
| Separate window (Chrome for Testing) | chrome-use downloads and launches a browser | 2.5.2: downloading and executing code |
| Apply into the user's Chrome | chrome-use extension relay over localhost | `network.client` covers the socket; the helper still has to be embedded |
| "Current tab" (AppleScript to Chrome) | `automation.apple-events` | Temporary-exception entitlement, justified per bundle id; reviewers push back |
| Favicons from Chrome's local cache | reads `Favicons` | Possible via a user-selected folder bookmark (ChooseBrowser's `ProfileAccessStore` pattern) |
| ⌥⌘K hotkey, launch at login, Touch ID | Carbon, `SMAppService`, LocalAuthentication | Fine |

## What a store build could be

A separate "CookieUse Lite": its own vault inside the container, accounts
added only by importing cookie exports (Cookie-Editor JSON, cURL) and redeeming
`.cusession` bundles, no Chrome decryption, no CLI sharing, apply limited to
what an embedded, sandboxed helper can reach. That is a different, weaker
product that also loses the agent story — and still carries review risk for a
tool whose purpose is moving login sessions between browsers.

## Decision

- Ship the full app as a notarized dmg: `install-app.sh`, or the dmg on the
  GitHub Release. It opens on a double-click with no Gatekeeper warning.
- Revisit only if there is demand that the notarized build can't meet
  (e.g. enterprise App Store–only fleets), and then as the Lite product above.

## Release plumbing already in place

- Repo secrets `APPLE_TEAM_ID`, `ASC_KEY_ID`, `ASC_ISSUER_ID`,
  `ASC_KEY_P8_BASE64` (App Store Connect API key) and `MACOS_CERTIFICATE_*`
  (Developer ID Application) — the same set ChooseBrowser uses.
- A store build would additionally need cloud-managed "Apple Distribution"
  signing through that same API key (see ChooseBrowser's
  `appstore-release.yml` / `build-appstore.sh`), a sandboxed `AppStore`
  configuration, and an App Store Connect app record.
