import AppKit
import SwiftUI

/// Export many accounts into one password-encrypted bundle, to move them to
/// another computer (open the file there, or `cookie-use redeem <file>`).
struct ExportSheet: View {
    @ObservedObject var model: AppModel
    /// Preselect "this site" when opened from a site or account.
    var site: String?
    @Environment(\.dismiss) private var dismiss

    enum Scope: Hashable { case all, site(String), tag(String) }

    @State private var scope: Scope = .all
    @State private var password = ""
    @State private var confirm = ""
    @State private var busy = false
    @State private var error: String?

    private var selected: [AccountSummary] {
        switch scope {
        case .all: return model.accounts.filter { !$0.tagList.contains("backup") }
        case let .site(s): return model.accounts.filter { Favicons.key($0.primarySite) == s }
        case let .tag(t): return model.accounts.filter { $0.tagList.contains(t) }
        }
    }

    private var canSubmit: Bool { !busy && !selected.isEmpty && password.count >= 8 && password == confirm }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Export logins", systemImage: "square.and.arrow.up.on.square").font(.title2.weight(.semibold))
            Text("One encrypted file for another computer. Open it there with CookieUse (or `cookie-use redeem <file>`); accounts already there keep whichever copy is newer.")
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)

            Picker("Accounts", selection: $scope) {
                Text("All accounts (\(model.accounts.filter { !$0.tagList.contains("backup") }.count))").tag(Scope.all)
                if !model.sites.isEmpty {
                    Divider()
                    ForEach(model.sites, id: \.site) { s in Text("\(s.site) (\(s.count))").tag(Scope.site(s.site)) }
                }
                if !model.allTags.isEmpty {
                    Divider()
                    ForEach(model.allTags, id: \.self) { t in Text("Tag: \(t)").tag(Scope.tag(t)) }
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                SecureField("Password (8+ characters)", text: $password).textFieldStyle(.roundedBorder)
                SecureField("Confirm password", text: $confirm).textFieldStyle(.roundedBorder)
                if !confirm.isEmpty, password != confirm { Text("Passwords don’t match.").font(.caption).foregroundStyle(.orange) }
            }

            Label("Anyone with the file and the password gets these logins. Send the password a different way.", systemImage: "lock.shield")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)

            if let error { Label(error, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.orange) }

            HStack(spacing: 10) {
                if busy { ProgressView().controlSize(.small); Text("Sealing…").font(.callout).foregroundStyle(.secondary) }
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(busy)
                Button("Export \(selected.count)…") { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction).disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 460)
        .onAppear { if let site { scope = .site(Favicons.key(site)) } }
    }

    private func submit() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "cookie-use-\(Date().formatted(.iso8601.year().month().day())).cusession"
        panel.directoryURL = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
        guard panel.runModal() == .OK, let url = panel.url else { return }
        busy = true
        error = nil
        let ids = selected.map(\.id)
        Task {
            do {
                let r = try await CLIBridge.shared.export(ids: ids, site: nil, out: url.path, password: password)
                busy = false
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: r.path)])
                model.show(.success("Exported \(r.count) account(s) to \(url.lastPathComponent)"))
                dismiss()
            } catch {
                busy = false
                self.error = error.localizedDescription
            }
        }
    }
}
