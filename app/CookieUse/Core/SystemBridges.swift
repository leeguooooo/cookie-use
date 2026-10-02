import AppKit
import Foundation

/// The URL of the front Chrome window's active tab, for "capture this tab".
/// Asked only when the user clicks the button, so the one-time Automation
/// prompt appears in context.
enum ChromeFrontTab {
    static func currentURL() async -> String? {
        await Task.detached(priority: .userInitiated) { () -> String? in
            let source = """
            if application "Google Chrome" is running then
                tell application "Google Chrome"
                    if (count of windows) > 0 then return URL of active tab of front window
                end tell
            end if
            return ""
            """
            var error: NSDictionary?
            let result = NSAppleScript(source: source)?.executeAndReturnError(&error)
            return result?.stringValue?.nilIfBlank
        }.value
    }
}

/// Fires when the vault file changes — an agent running the CLI in a terminal
/// shows up in the menu bar without a manual refresh.
final class VaultWatcher {
    private var source: DispatchSourceFileSystemObject?
    private var fd: Int32 = -1
    private let onChange: () -> Void
    private let url: URL

    init(onChange: @escaping () -> Void) {
        self.onChange = onChange
        let custom = ProcessInfo.processInfo.environment["COOKIE_USE_VAULT"]
        url = custom.map(URL.init(fileURLWithPath:))
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".cookie-use/vault.enc")
        start()
    }

    /// The CLI saves atomically (tmp + rename), which replaces the inode, so we
    /// watch the directory and re-arm.
    private func start() {
        let dir = url.deletingLastPathComponent()
        fd = open(dir.path, O_EVTONLY)
        guard fd >= 0 else { return }
        let src = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .rename, .delete], queue: .main)
        var pending: DispatchWorkItem?
        src.setEventHandler { [weak self] in
            pending?.cancel()
            let work = DispatchWorkItem { self?.onChange() }
            pending = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.8, execute: work)
        }
        let fd = fd
        src.setCancelHandler { close(fd) }
        src.resume()
        source = src
    }

    deinit { source?.cancel() }
}

enum Clipboard {
    static func copy(_ s: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(s, forType: .string)
    }
}
