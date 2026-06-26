# cookie-use → ChooseBrowser-style SwiftUI GUI: Authoritative CLI Contract

Source of truth: `/Users/leo/github.com/cookie-use/src/{main.rs, vault.rs, crypto.rs, keychain.rs, confirm.rs, share.rs, chrome_use.rs, runner.rs, act_as.rs}` cross-checked against `README.md`. 18 subcommands (Revoke is a pure alias of Rm → both route to `cmd_rm`). All success messages go to **stdout**; notes/warnings/prompts/errors go to **stderr**. `main()` maps any `anyhow` error to `error: {e:#}` on stderr + `exit(1)` — exit codes are coarse (no per-failure-class codes; `as` collapses a child's real exit code into 1).

---

## 1. Complete Command Table

| Command | Synopsis | Flags | stdout format | JSON? | Touch-ID gated | chrome-use calls |
|---|---|---|---|---|---|---|
| **add** | `add --from-profile <p> --site <d[,d]> [--id] [--label] [--hint] [--with-localstorage]` | `--from-profile`(req)→chrome-use `--from`; `--site`(req, comma list, ONE account)→`--domain`; `--id`(opt, default `<site-base>/NN` via `next_id()`); `--label`; `--hint`→`account_hint`; `--with-localstorage`(bool) | Human only. Success: `added "<id>" (<site>) from profile "<profile>"`. If localStorage captured, FIRST line: `captured <n> localStorage item(s) from <url>`. localStorage notes→stderr (`note: no localStorage found…` / `warning: localStorage capture failed…` non-fatal). | No | No | `cookies export --from <p> --domain <site> --json`; if `--with-localstorage`: capture session `cookie-use-capture` (launch about:blank → `cookies set --curl` → `open https://<primary>` → `storage local get --json` → `close`) |
| **import** | `import --file <f> --site <d> --id <id> [--label] [--hint]` | `--file`(req, JSON array OR `name=value; …` header string); `--site`(req); `--id`(req, **no default**); `--label`; `--hint`→`account_hint` | Human only. `imported "<id>" (<site>) from <file>` | No | No | **None** — pure local parse + vault write. localStorage always None. |
| **list** | `list [<site>] [--json]` | `<site>` positional (URL/domain, forgiving matcher via `normalize_site_filter`/`account_matches`; matches id/label/hint too); `--site` DEPRECATED flag (`conflicts_with="site"`); `--json`(bool) | **--json:** single-line `{"accounts":[{id,site,label,account_hint,status,cookies:<count>,last_used_at}]}` (empty→`{"accounts":[]}`; never cookie values). **Default:** grouped by site (BTreeMap-sorted), per account `  {id:<24} {status:<8} {cookies:>4}  {hint}`. Empty: `no saved accounts matching site "<s>"` / `no accounts yet — …`. | **YES** (only command) | No | None |
| **show** | `show <id>` | `<id>`(req) | Human only, key-aligned. Lines: `id:`, `site:`, opt `label:`/`hint:`, `status:`, `cookies:<count>`, opt `localStorage: <n> (<sorted KEYS only>)`, `created:`, `updated:`, opt `last used:`, `domains:<sorted unique>`, `expires:<rfc3339 (soonest)>` or `session cookies only…`, `storage: local-only, AES-256-GCM (~/.cookie-use/vault.enc)`. NEVER prints values. | No | No | None |
| **use** | `use <id> [--target session:<n>\|isolated] [--no-open] [--rewrite-domain <h>] [--open-url <url>] [--no-localstorage] [--no-confirm]` | `--target`(default `session:default`; `profile:*` rejected); `--no-open`; `--rewrite-domain`; `--open-url`(overrides --no-open & rewrite-skip); `--no-localstorage`; `--no-confirm` | Human only. `applied "<id>" (use)`. stderr possible: `note: --rewrite-domain set without --open-url; skipping auto-open…` | No | **YES** | `confirm::require` gate → `apply`: temp json → resolve session (launch `cookie-use-iso` if Isolated) → `cookies set --curl <tmp>` → if open_url `open <url>` → per-item `storage local set <k> <v>` + `reload`. `clear_first=false`. |
| **switch** | `switch <id> [same flags as use]` | Identical to `use` but `clear_first=true` | Human only. `applied "<id>" (switched)`. Same stderr note. | No | **YES** | Same gate; BEFORE apply: `cookies clear` (Session targets only) → then identical `apply` sequence. |
| **replay** | `replay <id> --to <host[:port]\|URL> [--target] [--no-confirm]` | `<id>`; `--to`(req, dev origin); `--target`; `--no-confirm` | Human only. `applied "<id>" (use)` (sugar over use). rewrite-skip note never fires (always passes both rewrite+open). | No | **YES** | Delegates to apply with `rewrite_domain=Some(host)`, `open_url=Some(url)`, `inject_localstorage=true`, `clear_first=false` ≡ `use --rewrite-domain <h> --open-url http://<h:port>`. |
| **share** | `share <id> [--out <f>] [--password <pw>]` | `<id>`; `--out`(default `<slug>.cusession`, `id_to_slug`); `--password`(TTY prompt if omitted; non-TTY without it errors) | Human only, 2 lines: line1 = bundle path (only scrapable datum); line2 = `redeem with: cookie-use redeem <path>` | No | No (password-gated, read-only) | None — calls pure `seal()` |
| **redeem** | `redeem <bundle> [--password <pw>] [--id\|--new-id <new>]` | `<bundle>`(req); `--password`; `--id`/`--new-id`(rename, avoid collision) | Human only, 2 lines: `redeemed "<final_id>" (<site>)`; install hint line. | No | No | None — pure `unseal()` + upsert. Sniff-checks JSON before password prompt. |
| **run** | `run [<id>] [--site <d>] [--all]` | `<id>`(single); `--site`(plain `String::contains`, NOT forgiving matcher); `--all`. Precedence: all > id > site. | Human only. Per account `opened "<id>" → <session>` (session `cookie-use-iso-<slug>`). Failures→stderr `warning: could not open "<id>": <err>`. Final: `opened <n> account(s) in isolated windows`. | No | **No (asymmetry!)** | Per account: `apply_isolated_named` → `--session cookie-use-iso-<slug> --launch open about:blank` → apply sequence. Each gets own named throwaway. |
| **as** | `as <id> [--target] [--no-confirm] -- <command…>` | `<id>`; `--target`; `--no-confirm`; `-- <command…>`(trailing varargs, must be non-empty) | Human only. `acting as "<id>" — running: <cmd>`. Then child inherits stdio. | No | **YES** | `confirm::require` → apply (open `https://<primary>`, inject localStorage) → spawn child with 4 env vars (`COOKIE_USE_ACCOUNT/SITE/TARGET`, `CHROME_USE_SESSION`). |
| **check** | `check <id>` | `<id>` | Human only, 1 line: `<id>: <status>` (Live/Expired). | No | No | None — heuristic only, no network probe. Writes status back, saves. |
| **rm** | `rm <id>` | `<id>` | `removed "<id>"` | No | No | None. No confirmation prompt. |
| **revoke** | `revoke <id>` | `<id>` | `removed "<id>"` (identical) | No | No | None — **pure alias** of `rm` (`Cmd::Revoke => cmd_rm`). |
| **rename** | `rename <id> <new_id>` | `<id>`, `<new_id>` | `renamed "<id>" -> "<new_id>"` | No | No | None. Errors if new_id exists / id missing. Refreshes `updated_at`. |
| **wipe** | `wipe [--yes]` | `--yes`(skip prompt) | `wiped vault (<n> account(s) removed)`. Without `--yes`: TTY `[y/N]` prompt→stderr. | No | No (plain `confirm_tty`, NOT biometric) | None. Irreversible. Non-interactive without `--yes` → refuses. |

**Security model summary the GUI must respect:**
- Vault `~/.cookie-use/vault.enc` (override `COOKIE_USE_VAULT`) = single AES-256-GCM blob (`nonce(12)||ct`, base64-std text, atomic tmp+rename write). Master key in macOS Keychain (`security` shell-out, service `cookie-use`/account `vault-key`); headless via `COOKIE_USE_VAULT_KEY` (base64 of exactly 32B) bypasses Keychain.
- **Injection gate** (`confirm.rs::decide(skip, yes_env, is_tty)`, precedence skip>yes_env>tty>deny): `--no-confirm`→Allow; `COOKIE_USE_YES` non-empty→Allow; stdin TTY→Biometric (Touch ID via `swift -` piped LocalAuthentication, OK/FAIL/UNAVAILABLE→Approved/Denied/TTY-fallback); else→Deny (`refusing to inject … without confirmation in a non-interactive shell; pass --no-confirm or set COOKIE_USE_YES=1`).
- **For a GUI**: it runs non-interactively → it MUST pass `--no-confirm` or set `COOKIE_USE_YES=1` on every gated call (use/switch/replay/as), and surface its own Touch-ID/confirm UX if desired. `run`/`wipe`/`share`/`redeem` are NOT biometric-gated.
- chrome-use binary overridable via `CHROME_USE_BIN`. cookie-use cannot detect whether a chrome-use session is connected (no `sessions list`/status call exists) — only signal is non-zero exit + stderr text.

---

## 2. Account Data Model (GUI mirror)

From `src/vault.rs struct Account` (derives Serialize/Deserialize). The GUI's Swift model must decode the `list --json` projection now, and the full Account once a `show --json` exists.

```swift
enum AccountStatus: String, Codable {   // #[serde(rename_all="lowercase")], default = unknown
    case unknown, live, expired
}

struct Account: Codable, Identifiable {
    let id: String                       // namespaced primary key, e.g. "chatgpt/work-01"  (required)
    var site: String                     // comma-joined domains (required)
    var label: String?                   // skip_serializing_if none
    var accountHint: String?             // JSON key "account_hint" — email/username, display-only
    var cookies: [JSONValue]             // full CDP Network.setCookie shape (required) — NEVER shown in list/show
    var localStorage: [String: JSONValue]?  // JSON key "local_storage" — primary-origin snapshot, optional
    var createdAt: Date                  // "created_at", RFC3339 (required)
    var updatedAt: Date                  // "updated_at", RFC3339 (required)
    var lastUsedAt: Date?                // "last_used_at", optional
    var status: AccountStatus            // default .unknown
    var proxy: String?                   // reserved v2, optional
    var fingerprint: JSONValue?          // reserved v2, optional

    enum CodingKeys: String, CodingKey {
        case id, site, label, cookies, status, proxy, fingerprint
        case accountHint = "account_hint"
        case localStorage = "local_storage"
        case createdAt = "created_at", updatedAt = "updated_at", lastUsedAt = "last_used_at"
    }
}
```

**`list --json` row projection** (what the GUI can decode TODAY — note `cookies` is a COUNT here, not the array):
```swift
struct AccountSummary: Codable, Identifiable {
    let id: String
    let site: String
    let label: String?
    let accountHint: String?   // "account_hint"
    let status: AccountStatus
    let cookies: Int           // count, NOT values
    let lastUsedAt: Date?      // "last_used_at"
}
struct ListResponse: Codable { let accounts: [AccountSummary] }
```

Container on disk: `VaultData { accounts: [Account] }` (the encrypted plaintext). v2-reserved fields are Optional → forward-stable.

---

## 3. JSON Output Gap Analysis — FIRST IMPLEMENTATION TASK

**Only `list --json` emits machine-readable output.** Everything else is human text. Before the GUI can parse reliably, add a `--json` flag (or a global `--json`) to the Rust CLI for the following. Listed concretely in priority order:

1. **`show --json`** (HIGH) — the GUI's detail pane needs structured per-account data: `domains[]`, soonest-expiry timestamp + `session_only` bool, `local_storage` KEY list, `created_at`/`updated_at`/`last_used_at`, `cookies` count. Today this is only pretty-printed `key: value` text.
2. **`check --json`** (HIGH) — liveness result is only `<id>: <status>` text, AND **exit is 0 regardless of Live/Expired**, so there is no signal at all to script against. Emit `{"id":…,"status":"live|expired"}`.
3. **apply family `--json`** (HIGH) — `use`/`switch`/`replay`/`as`/`run` only print `applied "<id>" (use|switched)` / `opened … → <session>`. No structured report of: which session was used, what URL opened, how many cookies/localStorage items injected, and **for `run --all` no per-account success/failure array** (failures only go to stderr text). The GUI needs `{ "results":[{"id":…,"session":…,"opened_url":…,"cookies":N,"localstorage":N,"ok":bool,"error":…}] }`.
4. **`add --json` / `import --json`** (MEDIUM) — only confirm with a text line; critically, **`add`'s auto-generated default id (`<site-base>/NN`) is not returned in any parseable field**, so the GUI can't know the id it just created without re-listing. Return `{"id":…,"site":…,"localstorage_captured":N}`.
5. **`share --json`** (MEDIUM) — bundle path is a bare first line. Return `{"path":…,"redeem_cmd":…}`. (Better: GUI binds directly to pure `seal()`/`unseal()` — see CLIBridge §4 note.)
6. **`redeem --json`** (LOW) — return `{"id":…,"site":…,"overwrote_existing":bool}` (CLI currently silently upsert-overwrites a same-id account with no collision signal).
7. **Cross-cutting**: Status enum currently surfaces only as Display strings (`live`/`expired`/`unknown`) with no stable numeric code — `--json` keeps the string, which is fine. **Exit codes stay coarse** (any error → `exit(1)`); the GUI must not rely on exit code to distinguish failure classes — parse `--json` error fields instead. Recommend a uniform error envelope on `--json` failures: `{"error":"<msg>"}` to stderr + exit 1.
8. **Missing entirely — chrome-use session status** (BLOCKER for "is it connected?" UX): there is NO `chrome-use sessions list`/status command anywhere. The GUI cannot tell whether a session name exists / the extension is attached before calling `cookies set --curl` (only failure signal is non-zero exit + stderr). Either add `chrome-use sessions list --json` or have the GUI treat apply failures as "session not connected" and prompt the user.

---

## 4. Recommended `CLIBridge` Swift Surface

A typed actor wrapping process spawns. **All injection methods must inject `COOKIE_USE_YES=1` into the child env (or pass `--no-confirm`)** because the GUI is non-interactive; the GUI owns its own confirmation/Touch-ID UX. Honor `CHROME_USE_BIN`/`COOKIE_USE_VAULT*` passthrough. Methods marked ⚠️ need the new `--json` flags from §3 before they return structured data — until then they return `Void`/raw text.

```swift
actor CLIBridge {
    // --- Reads (parse JSON) ---
    func listAccounts(filter: String? = nil) async throws -> [AccountSummary]
        // cookie-use list [<filter>] --json   → ListResponse.accounts        ✅ available today

    func show(id: String) async throws -> Account            // ⚠️ needs `show --json`
        // cookie-use show <id> --json

    func check(id: String) async throws -> AccountStatus     // ⚠️ needs `check --json`
        // cookie-use check <id> --json   (today: exit 0 always, must parse)

    // --- Capture / import (writes vault) ---
    func add(fromProfile: String, site: String, id: String? = nil,
             label: String? = nil, hint: String? = nil,
             withLocalStorage: Bool = false) async throws -> Account   // ⚠️ needs `add --json` for generated id
        // cookie-use add --from-profile <p> --site <d> [--id][--label][--hint][--with-localstorage] --json

    func importFile(_ path: String, site: String, id: String,
                    label: String? = nil, hint: String? = nil) async throws -> Account  // ⚠️ `import --json`
        // cookie-use import --file <f> --site <d> --id <id> [--label][--hint] --json

    // --- Injection (GATED — set COOKIE_USE_YES=1; GUI provides its own confirm) ---
    enum Target { case session(String), isolated }   // profile:* unsupported in v0.1

    func use(id: String, target: Target = .session("default"), open: Bool = true,
             rewriteDomain: String? = nil, openURL: String? = nil,
             injectLocalStorage: Bool = true) async throws -> ApplyResult   // ⚠️ `use --json`
        // cookie-use use <id> --target … [--no-open][--rewrite-domain][--open-url][--no-localstorage] --no-confirm

    func `switch`(id: String, target: Target = .session("default"), open: Bool = true,
                  rewriteDomain: String? = nil, openURL: String? = nil,
                  injectLocalStorage: Bool = true) async throws -> ApplyResult   // ⚠️ `switch --json`
        // cookie-use switch <id> …  --no-confirm   (clears target first)

    func replay(id: String, to devOrigin: String,
                target: Target = .session("default")) async throws -> ApplyResult   // ⚠️ `replay --json`
        // cookie-use replay <id> --to <host[:port]|URL> --target … --no-confirm

    func run(selector: RunSelector) async throws -> [RunResult]   // ⚠️ `run --json` for per-account array
        // cookie-use run [<id> | --site <d> | --all [--site <d>]] --json
        // NOTE: `run` is NOT biometric-gated even though it injects.

    func runAs(id: String, command: [String], target: Target = .session("default"),
               env extra: [String:String] = [:]) async throws -> Int32   // child exit code
        // cookie-use as <id> --target … --no-confirm -- <command…>
        // child gets COOKIE_USE_ACCOUNT/SITE/TARGET + CHROME_USE_SESSION

    // --- Lifecycle ---
    func rename(id: String, to newId: String) async throws        // cookie-use rename <id> <new>
    func remove(id: String) async throws                          // cookie-use rm <id>   (no prompt)
    func wipe(confirmed: Bool) async throws -> Int                 // cookie-use wipe --yes   (returns removed count)

    // --- Share / Redeem ---
    func share(id: String, out: String? = nil, password: String) async throws -> URL   // ⚠️ `share --json`
        // cookie-use share <id> [--out <f>] --password <pw>   → bundle path
    func redeem(bundle: String, password: String, newId: String? = nil) async throws -> Account  // ⚠️ `redeem --json`
        // cookie-use redeem <bundle> --password <pw> [--id <new>]
}

struct ApplyResult: Codable { let id, session: String; let openedURL: String?; let cookies, localStorage: Int }
enum RunSelector { case id(String), site(String), all(siteFilter: String?) }
struct RunResult: Codable { let id, session: String; let openedURL: String?; let ok: Bool; let error: String? }
```

**Architecture note for share/redeem:** `seal(account, password) -> Vec<u8>` (`share.rs:78`) and `unseal(bytes, password) -> Account` (`share.rs:103`) are pure, no-IO, `pub`, unit-tested. The bundle's `id` + `site` are stored CLEARTEXT (only cookies/localStorage are AES-GCM encrypted), so the GUI can preview an incoming `.cusession` before the password is entered. Argon2id derivation is intentionally ~1s — show a spinner. The redeem path silently upsert-overwrites a same-id account, so a "confirm on overwrite" GUI must check the cleartext id against the vault itself. **Strongly recommend** a Tauri/FFI binding to `seal()`/`unseal()` rather than shelling out, since the CLI wrappers add only file IO + stdout printing and have no `--json`. Also add a save-file dialog (CLI defaults to `<slug>.cusession` in cwd) and a password-confirm field (CLI rejects empty but never asks to confirm).

---

## DESIGN LANGUAGE

CHOOSEBROWSER DESIGN LANGUAGE — cheat-sheet for a new sibling macOS app (follow verbatim).

== PLATFORM / DEPLOYMENT ==
- Target macOS 26 (Tahoe). This is REQUIRED — the design relies on the macOS 26 Liquid Glass APIs (`GlassEffectContainer`, `.glassEffect(_:in:)`, `.regular.tint(_).interactive()`). These do NOT exist on earlier OS versions, so set the deployment target to macOS 26 and do not add availability fallbacks unless you intend to ship below 26.
- Pure SwiftUI views; drop to AppKit only for app icon (`NSApp.applicationIconImage`) and local key-event monitoring (`NSEvent.addLocalMonitorForEvents`). No third-party UI libs.

== LIQUID GLASS (the signature look) ==
1. Wrap a glass surface in `GlassEffectContainer(spacing: 10)` so sibling glass shapes blend/merge correctly.
2. The main floating panel uses a single large glass card:
   `.frame(width: 420)` then `.glassEffect(.regular, in: .rect(cornerRadius: 22))`. Outer `.padding(8)` around the container gives the panel breathing room from the window edge.
3. Selection / active interactive rows use ACCENT-TINTED INTERACTIVE glass, applied ONLY to the selected element (unselected stays transparent so the card shows through):
   `.glassEffect(.regular.tint(.accentColor).interactive(), in: .rect(cornerRadius: 8, style: .continuous))`
   Encapsulate this as a `@ViewBuilder` `selectionGlass(_ isSelected: Bool)` extension that returns `self` unchanged when not selected.
4. On a tinted/selected glass row, text flips to white: primary text `.white`, secondary/subtitle `.white.opacity(0.7)`, trailing badge text `.white.opacity(0.8)` on a `Color.white.opacity(0.2)` fill. Non-selected rows use `.primary` / `.secondary`.

== MATERIAL CARDS (non-glass surfaces) ==
- Settings/status surfaces and idle panels use system materials, NOT glass:
  - Window/panel background: `.background(.regularMaterial)`.
  - Grouped info cards: `.padding(16)` then `.background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))`, plus a hairline status-tinted border:
    `.overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(statusColor.opacity(0.25), lineWidth: 1))`.
  - Drag preview chips: `.padding(8).background(.regularMaterial, in: .rect(cornerRadius: 8))`.
- Lightweight inset fields (search box, key caps) use a translucent primary tint instead of material:
  `Color.primary.opacity(0.05)` fill, `.cornerRadius(8)`.

== CORNER RADIUS SCALE ==
- 22 = main floating glass panel.
- 14 = material info/status cards (always `style: .continuous`).
- 8  = rows, search field, inset controls, drag chips.
- 4  = small number/⌘ badges.
- 3  = footer keycap chips.
- Use `style: .continuous` on the larger rounded rects (panel, cards, selection rows).

== TYPOGRAPHY ==
- Use semantic system fonts; avoid hardcoded sizes except for tiny mono labels.
- Big screen title: `.font(.title.weight(.semibold))` (control panel) / `.font(.title2).fontWeight(.semibold)` (dashboard).
- Section / card title: `.font(.headline)`.
- Primary row / body text: `.font(.body)`.
- Descriptive subtitle: `.font(.callout)` with `.foregroundStyle(.secondary)`.
- Captions / metadata / "last action": `.font(.caption)` (status badge: `.caption.weight(.semibold)`; numeric badges: `.caption.monospacedDigit()`).
- Monospaced design is used deliberately for version strings, keycaps and shortcut hints: `.font(.system(size: 10/11, weight: .medium, design: .monospaced))`. Version pill = size 10; footer keycaps = size 11.

== COLOR / ACCENT ==
- Lean on system semantic colors: `.primary`, `.secondary`, `.accentColor` / `Color.accentColor`. Do NOT introduce a custom brand palette — tint everything from the system accent so it respects the user's macOS accent setting.
- Status colors are the only fixed hues: configured = `.green`, partial = `.orange`, notConfigured = `.red`. Status badges = `statusColor` text on `statusColor.opacity(0.15)` fill; card borders = `statusColor.opacity(0.25)`.
- Translucent overlays follow a fixed opacity ladder on `Color.primary`: 0.05 (inset field / small badge fill), 0.08 (version pill), 0.10 (keycap chip). On white (selected glass): 0.2 fill, 0.7/0.8 text.

== SF SYMBOLS ==
- Use SF Symbols everywhere for iconography, paired with text via `Label(title, systemImage:)` on buttons.
- Vocabulary in use: `magnifyingglass` (search), `gearshape` (settings), `play.fill` (start), `star.circle` / `checkmark.circle.fill` (set-default toggle), and status seals `checkmark.seal.fill` / `exclamationmark.triangle.fill` / `xmark.seal.fill`. Status glyphs sized `.font(.title3)`, tinted `.foregroundStyle(statusColor)`, pinned to a fixed `.frame(width: 22)` for alignment.

== BUTTONS ==
- Primary action: `.buttonStyle(.borderedProminent).controlSize(.large)`. The single most important affirmative action also gets `.keyboardShortcut(.defaultAction)`.
- Secondary action: `.buttonStyle(.bordered).controlSize(.large)`.
- List rows / tap targets: `.buttonStyle(.plain)` with custom glass/material styling and `.contentShape(.rect(cornerRadius: 8))` for a precise hit area.
- Buttons that represent a satisfied state are `.disabled(...)` with the label swapping to a "done" wording + filled checkmark symbol (e.g. "Default Configured" + `checkmark.circle.fill`).
- Group related buttons in an `HStack(spacing: 10)` ending with `Spacer(minLength: 0)`.

== BADGES / CAPSULES / KEYCAPS ==
- Version pill: monospaced size-10 text, `.padding(.horizontal,5).padding(.vertical,1)`, `Capsule().fill(Color.primary.opacity(0.08))`.
- Status badge: `.caption.weight(.semibold)`, `.padding(.horizontal,10).padding(.vertical,4)`, `.background(statusColor.opacity(0.15), in: Capsule())`.
- ⌘-number row badge: `.padding(.horizontal,4).padding(.vertical,2)`, `RoundedRectangle(cornerRadius:4)` fill (white@0.2 selected / primary@0.05 idle).
- Footer keycap hint: mono size-11, `.padding(.horizontal,4).padding(.vertical,1)`, `RoundedRectangle(cornerRadius:3).fill(Color.primary.opacity(0.1))`, paired with a `.caption .secondary` label in an `HStack(spacing:4)`.
- Drag-insertion indicator: `Capsule().fill(Color.accentColor).frame(height: 2)` overlaid at row top.

== SPACING / PADDING CONVENTIONS ==
- Header rows: `HStack(spacing: 12)` (icon ↔ text), app icon 40×40 in chooser / 52×52 in control panel; row icons via `AppIconView(size: 24)` (24 list / 18 drag chip).
- Vertical section stacks: `VStack(spacing: 14...20)` for top-level layout, `spacing: 2/3` for tight title+subtitle pairs.
- Panel padding: chooser header `.horizontal 16 / .top 16 / .bottom 12`; settings/control panels `.padding(24)` (compact) or `.padding(16)` (dense dashboard); cards `.padding(16)`.
- List content padding 12; row padding `.horizontal 10 / .vertical 8`; footer `.horizontal 16 / .vertical 10`.
- Use `Divider().padding(.horizontal, 16)` to separate header/search from the list, and a full-width `Divider()` before the footer.
- Window sizing: chooser fixed `width: 420` with a fixed list `height: 320`; control panel `minWidth: 480, minHeight: 240`; advanced dashboard `minWidth: 700, minHeight: 760`. End scroll/long stacks with `Spacer(minLength: 0)`.

== MOTION ==
- Keep animations subtle and short: selection scroll uses `withAnimation(.easeInOut(duration: 0.1))`. No flashy transitions.

== INTERACTION PATTERNS WORTH MATCHING ==
- Full keyboard-first operation: arrow keys move selection, ⌥+arrows reorder, ⌘1–9 quick-select, ↵ confirm, ⎋ cancel — surfaced in a footer of keycap hints.
- Trackpad reorder via `.draggable` + list-level `.dropDestination` with an accent capsule drop indicator.
- Search field auto-focused on appear; plain text-field style inside a tinted inset.

== OVERALL AESTHETIC ==
Native macOS 26, accent-driven, restrained. One accent color (system) + system grays + three fixed status hues. Glass for the floating action surface, regularMaterial for settings/status cards, semantic fonts, SF Symbols + Label pairing, monospaced for anything version/shortcut related. No custom brand colors, no gradients, no shadows beyond what the glass/material provide.
