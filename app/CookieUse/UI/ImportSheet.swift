import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Import a saved session from a JSON cookie array, a cURL command, or a
/// Cookie-header file (e.g. a Cookie-Editor export) into the vault.
struct ImportSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State var filePath: String
    @State private var site = ""
    @State private var accountID = ""
    @State private var label = ""
    @State private var busy = false
    @State private var error: String?

    private var host: String { site.normalizedHost }
    private var suggestedID: String {
        let base = Favicons.key(host).split(separator: ".").first.map(String.init) ?? "site"
        return "\(base)/\(label.nilIfBlank?.lowercased().replacingOccurrences(of: " ", with: "-") ?? "imported")"
    }

    private var canSubmit: Bool { !filePath.isEmpty && !host.isEmpty && !busy }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Import cookies", systemImage: "square.and.arrow.down").font(.title2.weight(.semibold))
            Text("A JSON cookie array (Cookie-Editor, EditThisCookie), a cURL command, or a raw Cookie header.")
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)

            HStack(spacing: 10) {
                Button { chooseFile() } label: { Label("Choose file…", systemImage: "folder") }
                if !filePath.isEmpty {
                    Label((filePath as NSString).lastPathComponent, systemImage: "doc")
                        .font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).help(filePath)
                }
            }

            Form {
                TextField("Website", text: $site, prompt: Text("example.com"))
                TextField("Name", text: $label, prompt: Text("Work account"))
                TextField("Account id", text: $accountID, prompt: Text(host.isEmpty ? "site/name" : suggestedID))
            }
            .formStyle(.columns)

            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 10) {
                Spacer()
                if busy { ProgressView().controlSize(.small) }
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(busy)
                Button("Import") { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction).disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.json, .text, .data]
        if panel.runModal() == .OK, let url = panel.url { filePath = url.path }
    }

    private func submit() {
        guard canSubmit else { return }
        busy = true
        error = nil
        Task {
            let ok = await model.importFile(path: filePath, site: host, id: accountID.nilIfBlank ?? suggestedID, label: label, hint: nil)
            busy = false
            if ok { dismiss() } else { error = model.banner?.text }
        }
    }
}
