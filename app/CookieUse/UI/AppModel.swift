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
}
