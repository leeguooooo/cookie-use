import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Import a shared `.cusession` bundle into the local vault. The bundle's id/site
/// are cleartext, but cookies/localStorage are AES-GCM-sealed behind the password.
struct RedeemSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var bundlePath = ""
    @State private var password = ""
    @State private var newID = ""
    @State private var busy = false

    private var canSubmit: Bool {
        !bundlePath.trimmingCharacters(in: .whitespaces).isEmpty
            && !password.isEmpty
            && !busy
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Redeem session", systemImage: "gift")
                .font(.title2.weight(.semibold))

            Text("Open a shared .cusession bundle and unlock it with the sender's password.")
                .font(.callout)
                .foregroundStyle(.secondary)

            VStack(alignment: .leading, spacing: 6) {
                Text("Bundle").font(.callout).foregroundStyle(.secondary)
                HStack(spacing: 8) {
                    Button {
                        chooseBundle()
                    } label: {
                        Label("Choose .cusession…", systemImage: "doc.badge.plus")
                    }
                    .buttonStyle(.bordered).controlSize(.large)

                    if !bundlePath.isEmpty {
                        Text(displayName(bundlePath))
                            .font(.callout)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("Password").font(.callout).foregroundStyle(.secondary)
                SecureField("Password", text: $password)
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("New id (optional)").font(.callout).foregroundStyle(.secondary)
                TextField("Rename on import to avoid collisions", text: $newID)
                    .textFieldStyle(.roundedBorder)
            }

            if let error = model.lastError {
                Text(error).font(.callout).foregroundStyle(.orange)
            }

            HStack(spacing: 10) {
                if busy { ProgressView().controlSize(.small) }
                Spacer()
                Button("Cancel") { dismiss() }
                    .buttonStyle(.bordered).controlSize(.large)
                Button("Redeem") { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 440)
    }

    private func chooseBundle() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        if let type = UTType(filenameExtension: "cusession") {
            panel.allowedContentTypes = [type]
        }
        panel.prompt = "Choose"
        if panel.runModal() == .OK, let url = panel.url {
            bundlePath = url.path
        }
    }

    private func displayName(_ path: String) -> String {
        URL(fileURLWithPath: path).lastPathComponent
    }

    private func submit() {
        let bundle = bundlePath.trimmingCharacters(in: .whitespaces)
        let trimmedID = newID.trimmingCharacters(in: .whitespaces)
        let id = trimmedID.isEmpty ? nil : trimmedID
        busy = true
        Task {
            let ok = await model.redeem(bundle: bundle, password: password, newID: id)
            busy = false
            if ok { dismiss() }
        }
    }
}
