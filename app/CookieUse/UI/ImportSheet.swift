import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Import a saved session from a JSON / cURL / Cookie-header file into the vault.
/// Unlike capture, `import` has no default id — the account id is required.
struct ImportSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var filePath: String = ""
    @State private var site: String = ""
    @State private var accountID: String = ""
    @State private var label: String = ""
    @State private var hint: String = ""
    @State private var busy = false

    private var trimmedSite: String { site.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var trimmedID: String { accountID.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canSubmit: Bool {
        !filePath.isEmpty && !trimmedSite.isEmpty && !trimmedID.isEmpty && !busy
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Import Session", systemImage: "square.and.arrow.down")
                .font(.title2.weight(.semibold))

            Text("Load a saved login from a JSON cookie array, a cURL command, or a Cookie-header file.")
                .font(.callout)
                .foregroundStyle(.secondary)

            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 10) {
                    Button {
                        chooseFile()
                    } label: {
                        Label("Choose File…", systemImage: "folder")
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)

                    if !filePath.isEmpty {
                        Text((filePath as NSString).lastPathComponent)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .help(filePath)
                    }
                }
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Site").font(.callout).foregroundStyle(.secondary)
                TextField("example.com", text: $site)
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Account id").font(.callout).foregroundStyle(.secondary)
                TextField("example.com/work-01", text: $accountID)
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Label (optional)").font(.callout).foregroundStyle(.secondary)
                TextField("Work account", text: $label)
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Hint (optional)").font(.callout).foregroundStyle(.secondary)
                TextField("name@example.com", text: $hint)
                    .textFieldStyle(.roundedBorder)
            }

            if let error = model.lastError {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 10) {
                Spacer()
                if busy { ProgressView().controlSize(.small) }
                Button("Cancel") { dismiss() }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .disabled(busy)
                Button("Import") { submit() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 440)
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.json, .text, .data]
        panel.prompt = "Choose"
        if panel.runModal() == .OK, let url = panel.url {
            filePath = url.path
        }
    }

    private func submit() {
        guard canSubmit else { return }
        busy = true
        Task {
            let ok = await model.importFile(
                path: filePath,
                site: trimmedSite,
                id: trimmedID,
                label: label.trimmingCharacters(in: .whitespacesAndNewlines),
                hint: hint.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            busy = false
            if ok { dismiss() }
        }
    }
}
