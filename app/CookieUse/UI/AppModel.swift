import SwiftUI

/// Observable state for the whole app: the account list, chrome-use connection
/// status, and the high-level actions the menu bar and window drive.
@MainActor
final class AppModel: ObservableObject {
    @Published var accounts: [AccountSummary] = []
    @Published var searchQuery: String = ""
    @Published var chromeConnected: Bool = false
    @Published var targetSession: String = "default"
    @Published var currentAccountID: String?        // last account we switched into (per target)
    @Published var isLoading = false
    @Published var lastError: String?

    private let bridge = CLIBridge.shared

    /// Accounts after the search filter, grouped by primary site, sites sorted.
    var groupedAccounts: [(site: String, accounts: [AccountSummary])] {
        let q = searchQuery.lowercased()
        let filtered = q.isEmpty ? accounts : accounts.filter {
            $0.displayName.lowercased().contains(q)
                || $0.id.lowercased().contains(q)
                || $0.site.lowercased().contains(q)
                || ($0.accountHint?.lowercased().contains(q) ?? false)
        }
        let groups = Dictionary(grouping: filtered, by: \.primarySite)
        return groups.keys.sorted().map { (site: $0, accounts: groups[$0]!.sorted { $0.displayName < $1.displayName }) }
    }

    func refresh() async {
        isLoading = true
        defer { isLoading = false }
        do {
            async let list = bridge.listAccounts()
            async let sessions = try? bridge.chromeSessions()
            async let current = bridge.currentChromeSession()
            let (accounts, sessionInfo, currentName) = await (try list, sessions, current)
            self.accounts = accounts
            self.chromeConnected = (sessionInfo?.data?.relay ?? false) || !(sessionInfo?.data?.sessions.isEmpty ?? true)
            if let currentName { self.targetSession = currentName }
            self.lastError = nil
        } catch {
            self.lastError = error.localizedDescription
        }
    }

    /// Touch-ID-gated clean switch into the live Chrome session.
    func switchInto(_ account: AccountSummary) async {
        guard await BiometricGate.confirm(reason: "Switch to “\(account.displayName)” on \(account.primarySite)") else { return }
        do {
            _ = try await bridge.switchTo(id: account.id, target: .session(targetSession))
            currentAccountID = account.id
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    /// Open every account of a site in side-by-side isolated windows.
    func runAll(site: String) async {
        do { _ = try await bridge.run(site: site, all: true) }
        catch { lastError = error.localizedDescription }
    }

    // MARK: Sheet routing

    enum Sheet: Identifiable, Equatable {
        case capture
        case importFile
        case redeem
        case share(AccountSummary)
        var id: String {
            switch self {
            case .capture: return "capture"
            case .importFile: return "import"
            case .redeem: return "redeem"
            case let .share(a): return "share-\(a.id)"
            }
        }
    }

    @Published var sheet: Sheet?
    @Published var lastMessage: String?

    private func report(_ message: String) { lastMessage = message; lastError = nil }
    private func fail(_ error: Error) { lastError = error.localizedDescription }

    // MARK: Mutating actions (each refreshes the list on success)

    func capture(fromProfile: String, site: String, id: String?, label: String?, hint: String?, withLocalStorage: Bool) async -> Bool {
        do {
            let newID = try await bridge.add(fromProfile: fromProfile, site: site, id: id?.nilIfBlank,
                                             label: label?.nilIfBlank, hint: hint?.nilIfBlank, withLocalStorage: withLocalStorage)
            report("Captured “\(newID)”."); await refresh(); return true
        } catch { fail(error); return false }
    }

    func importFile(path: String, site: String, id: String, label: String?, hint: String?) async -> Bool {
        do {
            let newID = try await bridge.importFile(path, site: site, id: id, label: label?.nilIfBlank, hint: hint?.nilIfBlank)
            report("Imported “\(newID)”."); await refresh(); return true
        } catch { fail(error); return false }
    }

    func share(_ account: AccountSummary, password: String) async -> URL? {
        do { let url = try await bridge.share(id: account.id, password: password); report("Shared to \(url.lastPathComponent)."); return url }
        catch { fail(error); return nil }
    }

    func redeem(bundle: String, password: String, newID: String?) async -> Bool {
        do {
            let result = try await bridge.redeem(bundle: bundle, password: password, newID: newID?.nilIfBlank)
            report(result.overwrote ? "Redeemed “\(result.id)” (replaced existing)." : "Redeemed “\(result.id)”.")
            await refresh(); return true
        } catch { fail(error); return false }
    }

    func rename(_ account: AccountSummary, to newID: String) async -> Bool {
        do { try await bridge.rename(id: account.id, to: newID); report("Renamed to “\(newID)”."); await refresh(); return true }
        catch { fail(error); return false }
    }

    func remove(_ account: AccountSummary) async -> Bool {
        do { try await bridge.remove(id: account.id); report("Removed “\(account.id)”."); await refresh(); return true }
        catch { fail(error); return false }
    }

    func replay(_ account: AccountSummary, to devOrigin: String) async {
        guard await BiometricGate.confirm(reason: "Replay “\(account.displayName)” on \(devOrigin)") else { return }
        do { _ = try await bridge.replay(id: account.id, to: devOrigin, target: .session(targetSession)); report("Replayed on \(devOrigin).") }
        catch { fail(error) }
    }
}

private extension String {
    var nilIfBlank: String? { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self }
}
