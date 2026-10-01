import SwiftUI

/// Overwrite one Chrome profile's login for a site with another profile's.
/// Only that site's cookies change; the destination's previous login is saved
/// to the vault first, so the banner can offer Undo.
struct CopySheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @StateObject private var probe = ProfileProbe()
    @State private var site = ""
    @State private var from: String?
    @State private var to: String?
    @State private var replace = true
    @State private var preview: CopyResult?
    @State private var previewTask: Task<Void, Never>?
    @State private var busy = false
    @State private var error: String?

    private var normalizedSite: String { site.withBaseDomains }
    private var primaryHost: String { normalizedSite.split(separator: ",").first.map(String.init) ?? "" }
    private var connected: Set<String> { Set(model.connectedBrowsers.compactMap { $0.email?.lowercased() }) }
    private var fromProfile: ChromeProfile? { probe.profiles.first { $0.directory == from } }
    private var toProfile: ChromeProfile? { probe.profiles.first { $0.directory == to } }
    private var canSubmit: Bool { !busy && !normalizedSite.isEmpty && from != nil && to != nil && error == nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 4) {
                Label("Copy a login between profiles", systemImage: "arrow.right.doc.on.clipboard")
                    .font(.title2.weight(.semibold))
                Text("Replaces one profile’s login for a site with another’s. Other sites stay signed in, and the old login is saved so you can undo.")
                    .font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            SheetStep(number: 1, title: "Website") {
                SiteField(site: $site, error: $error)
            }

            SheetStep(number: 2, title: "Copy from") {
                ProfileList(probe: probe, selection: $from,
                            unavailable: { probe.site.isEmpty || probe.count($0) > 0 ? nil : "no login here" },
                            maxHeight: 130)
            }

            SheetStep(number: 3, title: "Into") {
                ProfileList(probe: probe, selection: $to, sortByCookies: false, unavailable: destinationProblem, maxHeight: 130)
                Text("CookieUse writes through the chrome-use extension, so the destination profile must be open in Chrome with the extension connected.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Toggle(isOn: $replace) {
                Text("Remove \(primaryHost.isEmpty ? "the site" : primaryHost)’s other cookies in the destination")
                Text("So the old account is fully signed out. Off: only add and overwrite.")
            }
            .font(.callout)

            if let preview {
                Label {
                    Text("Copies \(preview.copied) cookies\(replace ? " and removes \(preview.removed) stale ones" : "") for \(primaryHost). Nothing else changes.")
                } icon: { Image(systemName: "info.circle") }
                    .font(.callout)
                    .infoCard()
            }

            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 10) {
                if busy { ProgressView().controlSize(.small); Text("Copying…").font(.callout).foregroundStyle(.secondary) }
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(busy)
                Button("Copy login") { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 520)
        .task {
            await probe.load()
            await model.refresh()
        }
        .onChange(of: normalizedSite) {
            preview = nil
            probe.probe(normalizedSite) {
                if from.map({ dir in probe.profiles.first { $0.directory == dir }.map(probe.count) ?? 0 }) ?? 0 == 0 {
                    from = probe.withCookies.count == 1 ? probe.withCookies.first?.directory : nil
                }
                refreshPreview()
            }
        }
        .onChange(of: from) { if to == from { to = nil }; refreshPreview() }
        .onChange(of: to) { refreshPreview() }
        .onChange(of: replace) { refreshPreview() }
    }

    private func destinationProblem(_ p: ChromeProfile) -> String? {
        if p.directory == from { return "the source" }
        guard let email = p.email else { return "not signed in to a Google account" }
        if !connected.contains(email.lowercased()) { return "open it in Chrome with the chrome-use extension" }
        return nil
    }

    private func refreshPreview() {
        previewTask?.cancel()
        error = nil
        preview = nil
        guard let from, let to, !normalizedSite.isEmpty else { return }
        let site = normalizedSite, keep = !replace
        previewTask = Task {
            do {
                let r = try await CLIBridge.shared.copy(site: site, from: from, to: to, keepExtra: keep, dryRun: true)
                if !Task.isCancelled { preview = r }
            } catch {
                if !Task.isCancelled { self.error = error.localizedDescription }
            }
        }
    }

    private func submit() {
        guard let from = fromProfile, let to = toProfile else { return }
        busy = true
        error = nil
        Task {
            let ok = await model.copyLogin(site: normalizedSite, from: from, to: to, keepExtra: !replace)
            busy = false
            if ok { dismiss() } else { error = model.banner?.text }
        }
    }
}
