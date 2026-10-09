import AppKit
import Combine
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let model = AppModel()
    private var statusItem: NSStatusItem!
    private let popover = NSPopover()
    private var window: NSWindow?
    private var hotKey: HotKey?
    private var watcher: VaultWatcher?
    private var bag: Set<AnyCancellable> = []

    func applicationDidFinishLaunching(_: Notification) {
        NSApp.setActivationPolicy(.accessory)

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "person.2.badge.key", accessibilityDescription: "CookieUse")
            button.action = #selector(togglePopover(_:))
            button.target = self
        }

        popover.behavior = .transient
        popover.animates = true
        popover.contentViewController = NSHostingController(
            rootView: MenuBarView(
                model: model,
                onOpenWindow: { [weak self] id in self?.openWindow(select: id) },
                onCapture: { [weak self] in self?.openWindow(sheet: .capture(prefill: nil)) },
                onSettings: { [weak self] in self?.openWindow(sheet: .settings) },
                onSheet: { [weak self] sheet in self?.openWindow(sheet: sheet) }
            )
        )
        (popover.contentViewController as? NSHostingController<MenuBarView>)?.sizingOptions = .preferredContentSize

        model.prefs.$hotKeyEnabled
            .sink { [weak self] on in
                self?.hotKey = on ? HotKey { [weak self] in self?.togglePopover(nil) } : nil
            }
            .store(in: &bag)

        // Agents editing the vault from a terminal show up live.
        watcher = VaultWatcher { [weak self] in Task { await self?.model.refresh() } }

        // Drop the Touch ID unlock window whenever the Mac sleeps or locks.
        let ws = NSWorkspace.shared.notificationCenter
        for name in [NSWorkspace.screensDidSleepNotification, NSWorkspace.willSleepNotification, NSWorkspace.sessionDidResignActiveNotification] {
            ws.addObserver(forName: name, object: nil, queue: .main) { _ in MainActor.assumeIsolated { BiometricGate.lock() } }
        }
        DistributedNotificationCenter.default().addObserver(forName: .init("com.apple.screenIsLocked"), object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { BiometricGate.lock() }
        }

        Task {
            await model.refresh()
            await model.loadCloud()
        }

        // The menu bar app runs for weeks: check at launch, hourly and after wake (once a day at most).
        model.updates.startPeriodicChecks()
    }

    /// Double-clicked / dropped-on-Dock `.cusession` bundles and cookie files.
    func application(_: NSApplication, open urls: [URL]) {
        guard let url = urls.first else { return }
        openWindow()
        model.handleFile(url)
    }

    @objc private func togglePopover(_: Any?) {
        guard let button = statusItem.button else { return }
        if popover.isShown {
            popover.performClose(nil)
        } else {
            NSApp.activate(ignoringOtherApps: true)
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            popover.contentViewController?.view.window?.makeKey()
            Task { await model.refresh() }
        }
    }

    func openWindow(select id: String? = nil, sheet: AppModel.Sheet? = nil) {
        popover.performClose(nil)
        if window == nil {
            let hosting = NSHostingController(rootView: RootWindow(model: model))
            let win = NSWindow(contentViewController: hosting)
            win.title = "CookieUse"
            win.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
            win.toolbarStyle = .unified
            win.setContentSize(NSSize(width: 980, height: 620))
            win.setFrameAutosaveName("CookieUseManager")
            win.isReleasedWhenClosed = false
            win.delegate = self
            win.center()
            window = win
        }
        if let id { model.selectedID = id }
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
        if let sheet {
            // Let the window come up before presenting the sheet on it.
            DispatchQueue.main.async { self.model.sheet = sheet }
        }
    }

    /// Back to a pure menu-bar app (no Dock icon) once the manager closes.
    func windowWillClose(_: Notification) {
        NSApp.setActivationPolicy(.accessory)
    }

    func applicationShouldHandleReopen(_: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows { openWindow() }
        return true
    }
}

/// The manager plus every sheet, so a sheet can be raised from the menu bar too.
struct RootWindow: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ManagementView(model: model, selection: $model.selectedID)
            .sheet(item: $model.sheet) { sheet in
                switch sheet {
                case let .capture(prefill): CaptureSheet(model: model, prefill: prefill)
                case let .importFile(path): ImportSheet(model: model, filePath: path ?? "")
                case let .redeem(path): RedeemSheet(model: model, bundlePath: path ?? "")
                case let .share(account): ShareSheet(account: account, model: model)
                case .settings: SettingsView(prefs: model.prefs, updates: model.updates)
                case .copy: CopySheet(model: model)
                case let .export(site): ExportSheet(model: model, site: site)
                case .cloud: CloudSyncSheet(model: model, prefs: model.prefs)
                }
            }
    }
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.run()
}
