import SwiftUI

/// Save (or refresh) a login from a Chrome profile into the vault.
///
/// Instead of asking for a profile name, it probes every local Chrome profile
/// for the site and lists the ones that have cookies for it.
struct CaptureSheet: View {
    @ObservedObject var model: AppModel
    let prefill: AppModel.CapturePrefill?
    @Environment(\.dismiss) private var dismiss

    @StateObject private var probe = ProfileProbe()
    @State private var site = ""
    @State private var label = ""
    @State private var accountID = ""
    @State private var hint = ""
    @State private var withLocalStorage = false
    @State private var showAdvanced = false
    @State private var busy = false
    @State private var error: String?
    @State private var profile: String?

    private var isRefresh: Bool { prefill?.id != nil }
    /// A refresh keeps the account's own site list; a new capture adds base domains.
    private var normalizedSite: String { isRefresh ? site : site.withBaseDomains }
    private var primaryHost: String { normalizedSite.split(separator: ",").first.map(String.init) ?? "" }

    private var suggestedID: String {
        if let id = prefill?.id { return id }
        let base = Favicons.key(primaryHost).split(separator: ".").first.map(String.init) ?? "site"
        let nameSource = label.nilIfBlank ?? probe.profiles.first { $0.directory == profile }?.name ?? "account"
        let slug = nameSource.lowercased().map { $0.isLetter || $0.isNumber ? $0 : "-" }
            .reduce(into: "") { s, c in if !(c == "-" && s.last == "-") { s.append(c) } }
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        var id = "\(base)/\(slug.isEmpty ? "account" : slug)"
        var n = 2
        let taken = Set(model.accounts.map(\.id))
        while taken.contains(id) { id = "\(base)/\(slug)-\(n)"; n += 1 }
        return id
    }

    private var canSubmit: Bool { !busy && !normalizedSite.isEmpty && profile != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 4) {
                Label(isRefresh ? "Refresh login" : "Save a login", systemImage: isRefresh ? "arrow.clockwise.circle" : "plus.circle")
                    .font(.title2.weight(.semibold))
                Text(isRefresh
                    ? "Sign in to \(primaryHost) again in Chrome, then pick that profile. Name, tags and notes are kept."
                    : "Log in to the site in Chrome first. CookieUse copies that login into its encrypted vault.")
                    .font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            SheetStep(number: 1, title: "Website") {
                SiteField(site: $site, disabled: isRefresh, error: $error)
                if !isRefresh, normalizedSite.contains(",") {
                    Text("Covers \(normalizedSite.replacingOccurrences(of: ",", with: ", "))")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }

            SheetStep(number: 2, title: "Chrome profile with the login") {
                ProfileList(probe: probe, selection: $profile)
                if !normalizedSite.isEmpty, !probe.probing {
                    let n = probe.withCookies.count
                    Text(n == 0
                        ? "No profile has cookies for \(primaryHost) yet — sign in to it in Chrome, then come back."
                        : n > 1 ? "\(n) profiles have cookies for \(primaryHost) — pick the one logged in to the account you want." : "")
                        .font(.caption).foregroundStyle(n == 0 ? .orange : .secondary)
                }
            }

            SheetStep(number: 3, title: "Name") {
                TextField(probe.profiles.first { $0.directory == profile }.map { "e.g. \($0.name)" } ?? "e.g. Work admin", text: $label)
                    .textFieldStyle(.roundedBorder)
                    .disabled(isRefresh)
                Text("Saved as \(accountID.nilIfBlank ?? suggestedID)")
                    .font(.caption.monospaced()).foregroundStyle(.secondary)
            }

            DisclosureGroup("Advanced", isExpanded: $showAdvanced) {
                VStack(alignment: .leading, spacing: 8) {
                    if !isRefresh {
                        TextField("Account id (\(suggestedID))", text: $accountID).textFieldStyle(.roundedBorder)
                        TextField("Login hint — email or username (display only)", text: $hint).textFieldStyle(.roundedBorder)
                    }
                    Toggle("Also capture localStorage (for apps that keep tokens there)", isOn: $withLocalStorage)
                    Text("Add more domains with commas, e.g. chatgpt.com,openai.com, to keep one login across them.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .padding(.top, 6)
            }
            .font(.callout)

            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 10) {
                if busy {
                    ProgressView().controlSize(.small)
                    Text(withLocalStorage ? "Saving (opening the site to read localStorage)…" : "Saving…")
                        .font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel") { dismiss() }.controlSize(.large).disabled(busy)
                Button(isRefresh ? "Refresh" : "Save login") { submit() }
                    .buttonStyle(.borderedProminent).controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 500)
        .task {
            if let prefill {
                site = prefill.site
                label = prefill.label ?? ""
            }
            await probe.load()
            reprobe(immediately: true)
        }
        .onChange(of: normalizedSite) { reprobe() }
    }

    private func reprobe(immediately: Bool = false) {
        probe.probe(normalizedSite, immediately: immediately) {
            // Pre-select when the choice is obvious; keep a deliberate pick if still valid.
            let current = probe.profiles.first { $0.directory == profile }
            if current.map({ probe.count($0) == 0 }) ?? true {
                profile = probe.withCookies.count == 1 ? probe.withCookies.first?.directory : nil
            }
        }
    }

    private func submit() {
        guard canSubmit, let profile else { return }
        busy = true
        error = nil
        let id = accountID.nilIfBlank ?? suggestedID
        Task {
            let saved = await model.capture(fromProfile: profile, site: normalizedSite, id: id,
                                            label: isRefresh ? nil : label, hint: hint, withLocalStorage: withLocalStorage)
            busy = false
            if saved != nil { dismiss() } else { error = model.banner?.text }
        }
    }
}

/// Website input with a favicon and "take Chrome's front tab".
struct SiteField: View {
    @Binding var site: String
    var disabled = false
    @Binding var error: String?

    var body: some View {
        HStack(spacing: 8) {
            HStack(spacing: 6) {
                let host = site.split(separator: ",").first.map { String($0).normalizedHost } ?? ""
                if host.contains(".") { SiteIcon(host: host, size: 16) }
                TextField("Paste a URL or domain — e.g. dash.cloudflare.com", text: $site)
                    .textFieldStyle(.plain)
                    .disabled(disabled)
            }
            .insetField()
            if !disabled {
                Button { Task { await useFrontTab() } } label: { Label("Current tab", systemImage: "safari") }
                    .help("Use the site open in Chrome’s front window")
            }
        }
    }

    private func useFrontTab() async {
        guard let url = await ChromeFrontTab.currentURL() else {
            error = "Couldn’t read Chrome’s front tab. Allow CookieUse to control Chrome in System Settings › Privacy & Security › Automation."
            return
        }
        let host = url.normalizedHost
        guard host.contains(".") else { error = "The front tab isn’t a website (\(url))."; return }
        error = nil
        site = host.withoutWWW
    }
}
