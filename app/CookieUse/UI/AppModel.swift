import AppKit
import Combine
import SwiftUI

/// Observable state for the whole app: the account list, Chrome connection,
/// and every action the quick switcher and the window drive.
@MainActor
final class AppModel: ObservableObject {
    @Published var accounts: [AccountSummary] = []
    @Published var connectedBrowsers: [ConnectedBrowser] = []
    @Published var missingBinaries: [String] = []
    @Published var isLoading = false
    @Published var hasLoaded = false
    /// Account id currently being applied (drives row spinners).
    @Published var busyID: String?
    @Published var banner: Banner?
    @Published var sheet: Sheet?
    /// The manager window's selected account.
    @Published var selectedID: String?
    /// Cloud sync settings, nil until loaded.
    @Published var cloud: CloudStatus?
    @Published var syncing = false
    /// A button shown next to the banner (e.g. "Undo" after a copy).
    @Published var bannerAction: BannerAction?

    let prefs = Preferences()
    private let bridge = CLIBridge.shared
    private var bannerTask: Task<Void, Never>?
    private var prefsSink: AnyCancellable?

    private var syncTimer: Timer?
    private var intervalSink: AnyCancellable?

    init() {
        // Views observe the model; re-publish preference changes (pins, toggles) through it.
        prefsSink = prefs.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }
        intervalSink = prefs.$syncInterval.sink { [weak self] minutes in self?.scheduleSync(minutes) }
    }

    // MARK: Cloud sync

    func loadCloud() async { cloud = await bridge.cloudStatus() }

    private func scheduleSync(_ minutes: Int) {
        syncTimer?.invalidate()
        syncTimer = nil
        guard minutes > 0 else { return }
        syncTimer = Timer.scheduledTimer(withTimeInterval: TimeInterval(minutes * 60), repeats: true) { [weak self] _ in
            Task { @MainActor in _ = await self?.syncNow(quiet: true) }
        }
    }

    /// Pull, merge and push. Returns an error message, or nil on success.
    @discardableResult
    func syncNow(quiet: Bool = false) async -> String? {
        guard !syncing else { return nil }
        syncing = true
        defer { syncing = false }
        do {
            let r = try await bridge.cloudSync()
            await loadCloud()
            await refresh()
            // Quiet (timer) syncs only speak up when something arrived.
            if !quiet || r.summary != "already up to date" { show(.success("Synced: \(r.summary)")) }
            return nil
        } catch {
            show(.error("Sync failed: \(error.localizedDescription)"))
            return error.localizedDescription
        }
    }

    // MARK: Copy between profiles

    func copyLogin(site: String, from: ChromeProfile, to: ChromeProfile, keepExtra: Bool) async -> Bool {
        guard await BiometricGate.confirm(reason: "Replace \(to.name)’s \(site) login with \(from.name)’s", policy: prefs.unlockPolicy)
        else { return false }
        do {
            let r = try await bridge.copy(site: site, from: from.directory, to: to.directory, keepExtra: keepExtra, dryRun: false)
            await refresh()
            let host = site.split(separator: ",").first.map(String.init) ?? site
            show(.success("\(to.name) is now signed in to \(host) as \(from.name)’s account"))
            if let backup = r.backupId, let email = to.email {
                bannerAction = BannerAction(title: "Undo") { [weak self] in
                    Task { await self?.restore(backup: backup, into: email, name: to.name) }
                }
            }
            return true
        } catch {
            show(.error(error.localizedDescription))
            return false
        }
    }

    /// Put a backup taken by `copy` back into the profile it came from.
    func restore(backup: String, into email: String, name: String) async {
        do {
            _ = try await bridge.open(id: backup, target: .browser(email), clean: true, openSite: false)
            show(.success("Restored \(name)’s previous login"))
        } catch { show(.error(error.localizedDescription)) }
    }

    var chromeConnected: Bool { !connectedBrowsers.isEmpty }

    /// "Chrome · leo@gmail.com" — injections land in *this* profile's cookie jar.
    var chromeLabel: String {
        guard let b = connectedBrowsers.first(where: { $0.default == true }) ?? connectedBrowsers.first else {
            return "Chrome not connected"
        }
        return b.email.map { "Chrome · \($0)" } ?? "Chrome connected"
    }

    // MARK: Derived collections

    var sites: [(site: String, count: Int)] {
        Dictionary(grouping: accounts, by: { Favicons.key($0.primarySite) })
            .map { (site: $0.key, count: $0.value.count) }
            .sorted { $0.site < $1.site }
    }

    var allTags: [String] { Array(Set(accounts.flatMap(\.tagList))).sorted() }

    var needsAttention: [AccountSummary] {
        accounts.filter(\.needsAttention)
    }

    /// Whether a Verify pass is running (drives the toolbar spinner).
    var verifying: Bool { verifyProgress != nil }

    var pinnedAccounts: [AccountSummary] {
        prefs.pinned.compactMap { id in accounts.first { $0.id == id } }
    }

    var recentAccounts: [AccountSummary] {
        accounts.filter { $0.lastUsedDate != nil && !prefs.isPinned($0.id) }
            .sorted { ($0.lastUsedDate ?? .distantPast) > ($1.lastUsedDate ?? .distantPast) }
    }

    func accounts(in scope: Scope, matching query: String) -> [AccountSummary] {
        let base: [AccountSummary]
        switch scope {
        case .all: base = accounts
        case .pinned: base = pinnedAccounts
        case .attention: base = needsAttention
        case let .site(s): base = accounts.filter { Favicons.key($0.primarySite) == s }
        case let .tag(t): base = accounts.filter { $0.tagList.contains(t) }
        }
        return base.filter { $0.matches(query) }
            .sorted { ($0.primarySite, $0.displayName.lowercased()) < ($1.primarySite, $1.displayName.lowercased()) }
    }

    func account(_ id: String?) -> AccountSummary? {
        guard let id else { return nil }
        return accounts.first { $0.id == id }
    }

    // MARK: Loading

    private var refreshing = false
    private var refreshAgain = false

    func refresh() async {
        // Never run two refreshes at once: the vault watcher can fire a burst
        // (an agent writing in a loop), and overlapping refreshes republishing
        // the same @Published arrays kept SwiftUI re-rendering forever. Coalesce
        // into one in-flight run plus at most one queued follow-up.
        if refreshing { refreshAgain = true; return }
        refreshing = true
        defer { refreshing = false }
        repeat {
            refreshAgain = false
            await refreshOnce()
        } while refreshAgain
    }

    private func refreshOnce() async {
        if !hasLoaded { isLoading = true }
        defer { isLoading = false; hasLoaded = true }
        let missing = await bridge.missingBinaries()
        setIfChanged(\.missingBinaries, missing)
        guard !missing.contains("cookie-use") else { setIfChanged(\.accounts, []); return }
        async let list = bridge.listAccounts()
        async let browsers = bridge.connectedBrowsers()
        do {
            let (a, b) = try await (list, browsers)
            // Only publish when the value actually changed, so an identical
            // refresh doesn't invalidate the whole view tree.
            setIfChanged(\.accounts, a)
            setIfChanged(\.connectedBrowsers, b)
            prefs.reconcile(validIDs: Set(a.map(\.id)))
        } catch {
            setIfChanged(\.connectedBrowsers, await browsers)
            show(.error(error.localizedDescription))
        }
    }

    /// Assign only when different, to avoid a no-op `@Published` republish.
    private func setIfChanged<T: Equatable>(_ key: ReferenceWritableKeyPath<AppModel, T>, _ value: T) {
        if self[keyPath: key] != value { self[keyPath: key] = value }
    }

    // MARK: Opening sessions

    enum Destination { case chrome, isolated }

    /// The quick switcher's primary action. Touch-ID gated per preferences.
    func open(_ account: AccountSummary, in destination: Destination? = nil) async {
        let dest = destination ?? (prefs.openIsolatedByDefault ? .isolated : .chrome)
        if dest == .chrome, !chromeConnected {
            show(.error("Chrome isn’t connected. Open Chrome with the chrome-use extension, or open in an isolated window (⌘↩)."))
            return
        }
        let reason = dest == .chrome
            ? "Sign in to \(account.primarySite) as “\(account.displayName)”"
            : "Open \(account.primarySite) as “\(account.displayName)” in a new window"
        guard await BiometricGate.confirm(reason: reason, policy: prefs.unlockPolicy) else { return }
        busyID = account.id
        defer { busyID = nil }
        do {
            switch dest {
            case .chrome:
                _ = try await bridge.open(id: account.id, target: .session("default"), clean: prefs.cleanSwitch)
                show(.success("Signed in to \(account.primarySite) as \(account.displayName)"))
            case .isolated:
                let results = try await bridge.run(ids: [account.id])
                if let failed = results.first(where: { !$0.ok }) {
                    throw CLIError(message: failed.error ?? "could not open \(account.id)")
                }
                show(.success("Opened \(account.displayName) in its own window"))
            }
            await refresh()
        } catch {
            show(.error(error.localizedDescription))
        }
    }

    /// Several accounts of one site, side by side (multi-account QA).
    func openSideBySide(_ list: [AccountSummary]) async {
        guard !list.isEmpty,
              await BiometricGate.confirm(reason: "Open \(list.count) accounts in separate windows", policy: prefs.unlockPolicy)
        else { return }
        do {
            let results = try await bridge.run(ids: list.map(\.id))
            let failed = results.filter { !$0.ok }
            show(failed.isEmpty ? .success("Opened \(results.count) windows")
                : .error("\(failed.count) of \(results.count) failed: \(failed.first?.error ?? "")"))
        } catch { show(.error(error.localizedDescription)) }
    }

    func replay(_ account: AccountSummary, to devOrigin: String) async {
        guard await BiometricGate.confirm(reason: "Replay “\(account.displayName)” on \(devOrigin)", policy: prefs.unlockPolicy) else { return }
        busyID = account.id
        defer { busyID = nil }
        do {
            _ = try await bridge.replay(id: account.id, to: devOrigin, target: .session("default"))
            show(.success("Replayed \(account.displayName) on \(devOrigin)"))
        } catch { show(.error(error.localizedDescription)) }
    }

    // MARK: Vault mutations (each refreshes the list on success)

    func capture(fromProfile: String, site: String, id: String?, label: String?, hint: String?, withLocalStorage: Bool) async -> String? {
        do {
            let newID = try await bridge.add(fromProfile: fromProfile, site: site, id: id?.nilIfBlank,
                                             label: label?.nilIfBlank, hint: hint?.nilIfBlank, withLocalStorage: withLocalStorage)
            show(.success("Saved \(newID)"))
            await refresh()
            return newID
        } catch { show(.error(error.localizedDescription)); return nil }
    }

    func importFile(path: String, site: String, id: String, label: String?, hint: String?) async -> Bool {
        do {
            let newID = try await bridge.importFile(path, site: site, id: id, label: label?.nilIfBlank, hint: hint?.nilIfBlank)
            show(.success("Imported \(newID)"))
            await refresh()
            return true
        } catch { show(.error(error.localizedDescription)); return false }
    }

    func edit(_ account: AccountSummary, label: String? = nil, hint: String? = nil, note: String? = nil, tags: [String]? = nil) async {
        do {
            try await bridge.edit(id: account.id, label: label, hint: hint, note: note, tags: tags)
            await refresh()
        } catch { show(.error(error.localizedDescription)) }
    }

    func rename(_ account: AccountSummary, to newID: String) async -> Bool {
        let newID = newID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !newID.isEmpty, newID != account.id else { return false }
        do {
            try await bridge.rename(id: account.id, to: newID)
            prefs.renamePin(from: account.id, to: newID)
            await refresh()
            return true
        } catch { show(.error(error.localizedDescription)); return false }
    }

    func remove(_ account: AccountSummary) async {
        do {
            try await bridge.remove(id: account.id)
            show(.success("Removed \(account.id)"))
            await refresh()
        } catch { show(.error(error.localizedDescription)) }
    }

    func share(_ account: AccountSummary, to url: URL, password: String) async -> URL? {
        do { return try await bridge.share(id: account.id, out: url.path, password: password) }
        catch { show(.error(error.localizedDescription)); return nil }
    }

    func redeemMany(bundle: String, password: String) async -> Result<String, Error> {
        do {
            let r = try await bridge.redeemMany(bundle: bundle, password: password)
            show(.success("Imported: \(r.summary)"))
            await refresh()
            return .success(r.summary)
        } catch { return .failure(error) }
    }

    func redeem(bundle: String, password: String, newID: String?) async -> Result<String, Error> {
        do {
            let r = try await bridge.redeem(bundle: bundle, password: password, newID: newID?.nilIfBlank)
            show(.success(r.overwrote ? "Redeemed \(r.id) (replaced the existing one)" : "Redeemed \(r.id)"))
            await refresh()
            return .success(r.id)
        } catch { return .failure(error) }
    }

    /// "checked 3 / 12" while a Verify pass runs, else nil.
    @Published var verifyProgress: String?

    /// Actually test whether logins still sign in (replays them in a throwaway
    /// browser). `ids` empty = every account. This opens a real browser
    /// off-screen and can take a few seconds each, so it is explicit, not automatic.
    func verify(ids: [String] = []) async {
        guard verifyProgress == nil else { return }
        verifyProgress = "Checking…"
        defer { verifyProgress = nil }
        do {
            let r = try await bridge.verify(ids: ids)
            await refresh()
            let msg: String
            if r.invalid == 0 {
                msg = r.checked == 1 ? "That login works" : "All \(r.valid) checked logins work"
            } else {
                msg = "\(r.invalid) of \(r.checked) need a fresh login"
            }
            show(.success(msg))
        } catch {
            show(.error("Couldn’t check logins: \(error.localizedDescription)"))
        }
    }

    // MARK: Files dropped / opened from Finder

    func handleFile(_ url: URL) {
        if url.pathExtension.lowercased() == "cusession" {
            sheet = .redeem(url.path)
        } else {
            sheet = .importFile(url.path)
        }
    }

    // MARK: Banner

    enum Banner: Equatable {
        case success(String)
        case error(String)
        var text: String { switch self { case let .success(s), let .error(s): return s } }
        var isError: Bool { if case .error = self { return true } else { return false } }
    }

    struct BannerAction: Identifiable {
        let id = UUID()
        let title: String
        let run: () -> Void
    }

    func show(_ banner: Banner) {
        self.banner = banner
        bannerAction = nil
        bannerTask?.cancel()
        bannerTask = Task {
            try? await Task.sleep(for: .seconds(banner.isError ? 8 : (self.bannerAction == nil ? 3.5 : 10)))
            if !Task.isCancelled { self.banner = nil; self.bannerAction = nil }
        }
    }

    // MARK: Routing

    enum Scope: Hashable {
        case all, pinned, attention
        case site(String)
        case tag(String)
    }

    enum Sheet: Identifiable, Equatable {
        case capture(prefill: CapturePrefill?)
        case importFile(String?)
        case redeem(String?)
        case share(AccountSummary)
        case settings
        case copy
        case export(site: String?)
        case cloud

        var id: String {
            switch self {
            case .capture: return "capture"
            case .importFile: return "import"
            case .redeem: return "redeem"
            case let .share(a): return "share-\(a.id)"
            case .settings: return "settings"
            case .copy: return "copy"
            case .export: return "export"
            case .cloud: return "cloud"
            }
        }
    }

    /// Re-capturing an existing account keeps its id/site fixed.
    struct CapturePrefill: Equatable {
        var site: String
        var id: String?
        var label: String?
    }
}
