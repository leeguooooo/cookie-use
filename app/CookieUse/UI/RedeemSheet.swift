import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Bring a teammate's `.cusession` bundle into the vault. The bundle's id and
/// site are cleartext, so we preview who it is (and warn about an id collision)
/// before the password is needed.
struct RedeemSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State var bundlePath: String
    @State private var password = ""
    @State private var newID = ""
    @State private var busy = false
    @State private var error: String?

    private struct Preview: Decodable { let id: String; let site: String }

    private var preview: Preview? {
        guard !bundlePath.isEmpty, let data = FileManager.default.contents(atPath: bundlePath) else { return nil }
        return try? JSONDecoder().decode(Preview.self, from: data)
    }

    private var collides: Bool {
        guard let p = preview else { return false }
        return newID.nilIfBlank == nil && model.accounts.contains { $0.id == p.id }
    }

    private var canSubmit: Bool { preview != nil && !password.isEmpty && !busy }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Redeem shared login", systemImage: "gift").font(.title2.weight(.semibold))

            if let p = preview {
                HStack(spacing: 12) {
                    SiteIcon(host: p.site.split(separator: ",").first.map(String.init) ?? p.site, size: 36)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(p.id).font(.headline.monospaced())
                        Text(p.site).font(.callout).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Change…") { chooseBundle() }
                }
                .infoCard()
            } else {
                Button { chooseBundle() } label: { Label("Choose .cusession…", systemImage: "doc.badge.plus") }
                    .controlSize(.large)
                if !bundlePath.isEmpty {
                    Text("That file isn’t a cookie-use bundle.").font(.callout).foregroundStyle(.orange)
                }
            }

            SecureField("Password from the sender", text: $password).textFieldStyle(.roundedBorder)

            VStack(alignment: .leading, spacing: 4) {
                TextField("Save under a different id (optional)", text: $newID).textFieldStyle(.roundedBorder)
                if collides {
                    Label("You already have “\(preview!.id)”. Redeeming replaces it — set a different id to keep both.",
                          systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange).fixedSize(horizontal: false, vertical: true)
                }
            }

            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.orange)
            }

            HStack(spacing: 10) {
                if busy { ProgressView().controlSize(.small); Text("Unlocking…").font(.callout).foregroundStyle(.secondary) }
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(busy)
                Button(collides ? "Replace" : "Redeem") { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction).disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    private func chooseBundle() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        if let type = UTType(filenameExtension: "cusession") { panel.allowedContentTypes = [type] }
        if panel.runModal() == .OK, let url = panel.url { bundlePath = url.path }
    }

    private func submit() {
        busy = true
        error = nil
        Task {
            let result = await model.redeem(bundle: bundlePath, password: password, newID: newID)
            busy = false
            switch result {
            case .success: dismiss()
            case let .failure(e): error = e.localizedDescription
            }
        }
    }
}
