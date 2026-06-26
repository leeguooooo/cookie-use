import SwiftUI
import AppKit

/// Export a password-encrypted `.cusession` bundle for one account.
///
/// The bundle's id + site are stored cleartext; cookies/localStorage are
/// sealed with AES-256-GCM behind an argon2id-derived key (~1s, hence the
/// ProgressView). On success the new file is revealed in Finder.
struct ShareSheet: View {
    let account: AccountSummary

    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var password = ""
    @State private var confirmPassword = ""
    @State private var busy = false

    private var trimmedPassword: String {
        password.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var passwordsMatch: Bool { password == confirmPassword }

    private var canSubmit: Bool {
        !trimmedPassword.isEmpty && passwordsMatch && !busy
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Share Account", systemImage: "square.and.arrow.up")
                .font(.title2.weight(.semibold))

            // Which account is being exported.
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.displayName)
                        .font(.headline)
                    Text(account.primarySite)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                StatusBadge(status: account.status)
            }
            .infoCard(border: account.status.color)

            VStack(alignment: .leading, spacing: 8) {
                SecureField("Password", text: $password)
                    .textFieldStyle(.roundedBorder)
                SecureField("Confirm password", text: $confirmPassword)
                    .textFieldStyle(.roundedBorder)

                if !confirmPassword.isEmpty && !passwordsMatch {
                    Text("Passwords don’t match.")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }

                Text("The session is sealed with AES-256-GCM. Key derivation (argon2id) takes about a second.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if let error = model.lastError, !error.isEmpty {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 10) {
                if busy {
                    ProgressView()
                        .controlSize(.small)
                    Text("Sealing…")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Button("Cancel") { dismiss() }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .disabled(busy)
                Button("Share") { submit() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 440)
    }

    private func submit() {
        guard canSubmit else { return }
        busy = true
        Task {
            let url = await model.share(account, password: trimmedPassword)
            busy = false
            if let url {
                NSWorkspace.shared.activateFileViewerSelecting([url])
                dismiss()
            }
        }
    }
}
