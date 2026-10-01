import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Bring a `.cusession` file into the vault: one shared login (v1) or a whole
/// export from another computer (v2). Both carry a cleartext index, so we show
/// what's inside — and what's new — before the password is needed.
struct RedeemSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State var bundlePath: String
    @State private var password = ""
    @State private var newID = ""
    @State private var busy = false
    @State private var error: String?

    private struct Entry: Decodable { let id: String; let site: String }
    private struct Preview: Decodable {
        let cookieUseBundle: Int?
        let id: String?
        let site: String?
        let accounts: [Entry]?
        var entries: [Entry] {
            if let accounts { return accounts }
            if let id, let site { return [Entry(id: id, site: site)] }
            return []
        }
    }

    private var preview: Preview? {
        guard !bundlePath.isEmpty, let data = FileManager.default.contents(atPath: bundlePath) else { return nil }
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return (try? d.decode(Preview.self, from: data)).flatMap { $0.entries.isEmpty ? nil : $0 }
    }

    private var isMulti: Bool { preview?.cookieUseBundle == 2 }
    private var existing: Set<String> { Set(model.accounts.map(\.id)) }

    private var collides: Bool {
        guard let p = preview, !isMulti, let id = p.entries.first?.id else { return false }
        return newID.nilIfBlank == nil && existing.contains(id)
    }

    private var canSubmit: Bool { preview != nil && !password.isEmpty && !busy }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label(isMulti ? "Import logins" : "Redeem shared login", systemImage: isMulti ? "square.and.arrow.down.on.square" : "gift")
                .font(.title2.weight(.semibold))

            if let p = preview {
                if isMulti { multiPreview(p.entries) } else { singlePreview(p.entries[0]) }
            } else {
                Button { chooseBundle() } label: { Label("Choose .cusession…", systemImage: "doc.badge.plus") }
                    .controlSize(.large)
                if !bundlePath.isEmpty {
                    Text("That file isn’t a cookie-use bundle.").font(.callout).foregroundStyle(.orange)
                }
            }

            SecureField("Password", text: $password).textFieldStyle(.roundedBorder)

            if !isMulti {
                VStack(alignment: .leading, spacing: 4) {
                    TextField("Save under a different id (optional)", text: $newID).textFieldStyle(.roundedBorder)
                    if collides, let id = preview?.entries.first?.id {
                        Label("You already have “\(id)”. Redeeming replaces it — set a different id to keep both.",
                              systemImage: "exclamationmark.triangle")
                            .font(.caption).foregroundStyle(.orange).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }

            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.orange)
            }

            HStack(spacing: 10) {
                if busy { ProgressView().controlSize(.small); Text("Unlocking…").font(.callout).foregroundStyle(.secondary) }
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(busy)
                Button(isMulti ? "Import" : (collides ? "Replace" : "Redeem")) { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction).disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    private func singlePreview(_ e: Entry) -> some View {
        HStack(spacing: 12) {
            SiteIcon(host: e.site.split(separator: ",").first.map(String.init) ?? e.site, size: 36)
            VStack(alignment: .leading, spacing: 2) {
                Text(e.id).font(.headline.monospaced())
                Text(e.site).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Change…") { chooseBundle() }
        }
        .infoCard()
    }

    private func multiPreview(_ entries: [Entry]) -> some View {
        let fresh = entries.filter { !existing.contains($0.id) }.count
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("\(entries.count) accounts").font(.headline)
                Text("· \(fresh) new, \(entries.count - fresh) already here").font(.callout).foregroundStyle(.secondary)
                Spacer()
                Button("Change…") { chooseBundle() }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(entries, id: \.id) { e in
                        HStack(spacing: 8) {
                            SiteIcon(host: e.site.split(separator: ",").first.map(String.init) ?? e.site, size: 14)
                            Text(e.id).font(.callout.monospaced()).lineLimit(1)
                            Spacer()
                            if !existing.contains(e.id) { Text("new").font(.caption).foregroundStyle(.green) }
                        }
                    }
                }
            }
            .frame(maxHeight: 140)
            Text("For accounts you already have, the newer copy wins.").font(.caption).foregroundStyle(.secondary)
        }
        .infoCard()
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
            let result = isMulti
                ? await model.redeemMany(bundle: bundlePath, password: password)
                : await model.redeem(bundle: bundlePath, password: password, newID: newID)
            busy = false
            switch result {
            case .success: dismiss()
            case let .failure(e): error = e.localizedDescription
            }
        }
    }
}
