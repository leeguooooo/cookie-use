import Foundation
import ServiceManagement

/// Per-user GUI preferences. Account data lives in the vault; these are only
/// how *this* app presents and acts on it.
@MainActor
final class Preferences: ObservableObject {
    private let store = UserDefaults.standard

    /// Accounts pinned to the top of the quick switcher.
    @Published var pinned: [String] { didSet { store.set(pinned, forKey: "pinned") } }

    /// Where Return in the quick switcher opens an account.
    @Published var openIsolatedByDefault: Bool { didSet { store.set(openIsolatedByDefault, forKey: "openIsolated") } }

    /// Sign the site's previous account out before applying (scoped to that site).
    @Published var cleanSwitch: Bool { didSet { store.set(cleanSwitch, forKey: "cleanSwitch") } }

    @Published var unlockPolicy: UnlockPolicy { didSet { store.set(unlockPolicy.rawValue, forKey: "unlockPolicy") } }

    @Published var hotKeyEnabled: Bool { didSet { store.set(hotKeyEnabled, forKey: "hotKey") } }

    @Published var launchAtLogin: Bool {
        didSet {
            guard launchAtLogin != (SMAppService.mainApp.status == .enabled) else { return }
            do {
                if launchAtLogin { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            } catch {
                launchAtLogin = SMAppService.mainApp.status == .enabled
            }
        }
    }

    init() {
        pinned = store.stringArray(forKey: "pinned") ?? []
        openIsolatedByDefault = store.bool(forKey: "openIsolated")
        cleanSwitch = store.object(forKey: "cleanSwitch") as? Bool ?? true
        unlockPolicy = UnlockPolicy(rawValue: store.string(forKey: "unlockPolicy") ?? "") ?? .tenMinutes
        hotKeyEnabled = store.object(forKey: "hotKey") as? Bool ?? true
        launchAtLogin = SMAppService.mainApp.status == .enabled
    }

    func isPinned(_ id: String) -> Bool { pinned.contains(id) }

    func togglePin(_ id: String) {
        if let i = pinned.firstIndex(of: id) { pinned.remove(at: i) } else { pinned.append(id) }
    }

    /// Keep pins in sync with renames / removals.
    func reconcile(validIDs: Set<String>) {
        let kept = pinned.filter(validIDs.contains)
        if kept != pinned { pinned = kept }
    }

    func renamePin(from old: String, to new: String) {
        if let i = pinned.firstIndex(of: old) { pinned[i] = new }
    }
}
