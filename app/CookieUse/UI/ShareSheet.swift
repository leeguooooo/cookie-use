import AppKit
import SwiftUI

/// Export a password-encrypted `.cusession` bundle for one account.
///
/// The bundle's id + site are stored cleartext; cookies/localStorage are
/// sealed with AES-256-GCM behind an argon2id-derived key (~1s, hence the
/// spinner). On success the file is revealed in Finder.
struct ShareSheet: View {
    let account: AccountSummary
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var password = ""
    @State private var confirmPassword = ""
    @State private var busy = false

    private var mismatch: Bool { !confirmPassword.isEmpty && password != confirmPassword }
    private var tooShort: Bool { !password.isEmpty && password.count < 8 }
    private var canSubmit: Bool { password.count >= 8 && password == confirmPassword && !busy }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Share login", systemImage: "square.and.arrow.up").font(.title2.weight(.semibold))

            HStack(spacing: 12) {
                SiteIcon(host: account.primarySite, size: 36)
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.displayName).font(.headline)
                    Text(account.primarySite).font(.callout).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                StatusBadge(account: account)
            }
            .infoCard(border: account.effectiveStatus.color)

            VStack(alignment: .leading, spacing: 8) {
                SecureField("Password (8+ characters)", text: $password).textFieldStyle(.roundedBorder)
                SecureField("Confirm password", text: $confirmPassword).textFieldStyle(.roundedBorder)
                if tooShort { Text("Use at least 8 characters.").font(.caption).foregroundStyle(.orange) }
                if mismatch { Text("Passwords don’t match.").font(.caption).foregroundStyle(.orange) }
            }

            Label("Anyone with the file and the password is signed in as this account. Send the password over a different channel.",
                  systemImage: "lock.shield")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)

            HStack(spacing: 10) {
                if busy { ProgressView().controlSize(.small); Text("Sealing…").font(.callout).foregroundStyle(.secondary) }
                Spacer(minLength: 0)
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(busy)
                Button("Save bundle…") { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction).disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    private func submit() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = account.id.replacingOccurrences(of: "/", with: "-") + ".cusession"
        panel.directoryURL = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
        guard panel.runModal() == .OK, let url = panel.url else { return }
        busy = true
        Task {
            let out = await model.share(account, to: url, password: password)
            busy = false
            if let out {
                NSWorkspace.shared.activateFileViewerSelecting([out])
                model.show(.success("Saved \(out.lastPathComponent)"))
                dismiss()
            }
        }
    }
}
