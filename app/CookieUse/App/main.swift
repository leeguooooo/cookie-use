import AppKit
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let model = AppModel()
    private var statusItem: NSStatusItem!
    private var popover = NSPopover()
    private var window: NSWindow?

    func applicationDidFinishLaunching(_: Notification) {
        NSApp.setActivationPolicy(.accessory)

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "person.2.badge.key", accessibilityDescription: "cookie-use")
            button.action = #selector(togglePopover(_:))
            button.target = self
        }

        popover.behavior = .transient
        popover.contentSize = NSSize(width: 376, height: 472)
        popover.contentViewController = NSHostingController(
            rootView: MenuBarView(
                model: model,
                onOpenWindow: { [weak self] in self?.openWindow() },
                onCapture: { [weak self] in self?.openWindow() }
            )
        )
    }

    @objc private func togglePopover(_: Any?) {
        guard let button = statusItem.button else { return }
        if popover.isShown {
            popover.performClose(nil)
        } else {
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            popover.contentViewController?.view.window?.makeKey()
            Task { await model.refresh() }
        }
    }

    func openWindow() {
        popover.performClose(nil)
        if window == nil {
            let hosting = NSHostingController(rootView: ManagementView(model: model))
            let win = NSWindow(contentViewController: hosting)
            win.title = "cookie-use"
            win.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
            win.titlebarAppearsTransparent = true
            win.setContentSize(NSSize(width: 760, height: 580))
            win.center()
            window = win
        }
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.run()
}
