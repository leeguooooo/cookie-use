import SwiftUI

/// Save (or refresh) a login from a Chrome profile into the vault.
///
/// Instead of asking for a profile name, it probes every local Chrome profile
/// for the site and shows which ones are actually signed in.
struct CaptureSheet: View {
    @ObservedObject var model: AppModel
    let prefill: AppModel.CapturePrefill?
    @Environment(\.dismiss) private var dismiss

    @State private var site = ""
    @State private var label = ""
    @State private var accountID = ""
    @State private var hint = ""
    @State private var withLocalStorage = false
    @State private var showAdvanced = false
    @State private var busy = false
    @State private var error: String?

    @State private var profiles: [ChromeProfile] = []
    @State private var counts: [String: Int] = [:]
    @State private var probing = false
    @State private var profile: String?
    @State private var probeTask: Task<Void, Never>?

    private var isRefresh: Bool { prefill?.id != nil }
    /// What gets passed to the CLI: comma-joined hosts, URLs normalized. A
    /// subdomain also pulls in its base domain — logins usually live on the
    /// parent (`.cloudflare.com`), and the CLI opens the first host listed.
    private var normalizedSite: String {
        var hosts: [String] = []
        for h in site.split(separator: ",").map({ String($0).normalizedHost }) where !h.isEmpty {
            for candidate in [h, Favicons.key(h)] where candidate.contains(".") && !hosts.contains(candidate) {
                hosts.append(candidate)
            }
        }
        return hosts.joined(separator: ",")
    }

    private var primaryHost: String { normalizedSite.split(separator: ",").first.map(String.init) ?? "" }

    private var signedIn: [ChromeProfile] { profiles.filter { (counts[$0.directory] ?? 0) > 0 } }

    private var suggestedID: String {
        if let id = prefill?.id { return id }
        let base = Favicons.key(primaryHost).split(separator: ".").first.map(String.init) ?? "site"
        let nameSource = label.nilIfBlank ?? profiles.first { $0.directory == profile }?.name ?? "account"
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

            step(1, "Website") {
                HStack(spacing: 8) {
                    HStack(spacing: 6) {
                        if !primaryHost.isEmpty { SiteIcon(host: primaryHost, size: 16) }
                        TextField("Paste a URL or domain — e.g. dash.cloudflare.com", text: $site)
                            .textFieldStyle(.plain)
                            .disabled(isRefresh)
                    }
                    .insetField()
                    if !isRefresh {
                        Button { Task { await useFrontTab() } } label: { Label("Current tab", systemImage: "safari") }
                            .help("Use the site open in Chrome’s front window")
                    }
                }
            }

            if !isRefresh, normalizedSite.contains(",") {
                Text("Covers \(normalizedSite.replacingOccurrences(of: ",", with: ", "))")
                    .font(.caption).foregroundStyle(.secondary).padding(.leading, 28).padding(.top, -12)
            }

            step(2, "Chrome profile with the login") { profilePicker }

            step(3, "Name") {
                TextField(profiles.first { $0.directory == profile }.map { "e.g. \($0.name)" } ?? "e.g. Work admin", text: $label)
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
            profiles = await CLIBridge.shared.chromeProfiles()
            scheduleProbe(immediately: true)
        }
        .onChange(of: normalizedSite) { scheduleProbe() }
    }

    // MARK: Profile picker

    @ViewBuilder
    private var profilePicker: some View {
        if profiles.isEmpty {
            Text("No Chrome profiles found.").font(.callout).foregroundStyle(.secondary)
        } else {
            VStack(alignment: .leading, spacing: 2) {
                ScrollView {
                    VStack(spacing: 1) {
                        ForEach(sortedProfiles) { p in profileRow(p) }
                    }
                }
                .frame(maxHeight: 168)
                .padding(4)
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: DS.rowRadius))
                if !normalizedSite.isEmpty, !probing {
                    Text(signedIn.isEmpty
                        ? "No profile has a login for \(primaryHost) yet — sign in to it in Chrome, then come back."
                        : signedIn.count > 1 ? "\(signedIn.count) profiles have cookies for \(primaryHost) — pick the one logged in to the account you want." : "")
                        .font(.caption).foregroundStyle(signedIn.isEmpty ? .orange : .secondary)
                }
            }
        }
    }

    /// Profiles with the most cookies for the site first (a real login sets
    /// many more than analytics does), then the rest in Chrome's order.
    private var sortedProfiles: [ChromeProfile] {
        let n = { (p: ChromeProfile) in counts[p.directory] ?? 0 }
        return profiles.filter { n($0) > 0 }.sorted { n($0) > n($1) } + profiles.filter { n($0) == 0 }
    }

    private func profileRow(_ p: ChromeProfile) -> some View {
        let selected = profile == p.directory
        let n = counts[p.directory]
        return HStack(spacing: 8) {
            Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                .foregroundStyle(selected ? Color.accentColor : .secondary)
            Text(p.name)
            Text(p.directory).font(.caption).foregroundStyle(.tertiary)
            Spacer()
            if normalizedSite.isEmpty {
                EmptyView()
            } else if probing && n == nil {
                ProgressView().controlSize(.mini)
            } else if let n, n > 0 {
                Label("\(n) cookies", systemImage: "checkmark.circle.fill").font(.caption.monospacedDigit()).foregroundStyle(.green)
                    .help("This profile has \(n) cookies for \(primaryHost) — a logged-in profile usually has the most")
            } else {
                Text("no login").font(.caption).foregroundStyle(.tertiary)
            }
        }
        .font(.callout)
        .padding(.horizontal, 8).padding(.vertical, 5)
        .contentShape(Rectangle())
        .background(selected ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 6))
        .onTapGesture { profile = p.directory }
    }

    // MARK: Actions

    private func scheduleProbe(immediately: Bool = false) {
        probeTask?.cancel()
        counts = [:]
        let target = normalizedSite
        guard !target.isEmpty, !profiles.isEmpty else { probing = false; return }
        probing = true
        let list = profiles
        probeTask = Task {
            if !immediately { try? await Task.sleep(for: .milliseconds(450)) }
            guard !Task.isCancelled else { return }
            await withTaskGroup(of: (String, Int).self) { group in
                for p in list {
                    group.addTask { (p.directory, await CLIBridge.shared.cookieCount(profile: p.directory, site: target)) }
                }
                for await (dir, n) in group {
                    guard !Task.isCancelled else { return }
                    counts[dir] = n
                }
            }
            guard !Task.isCancelled else { return }
            probing = false
            // Pre-select when the choice is obvious; keep a deliberate pick if still valid.
            if profile == nil || (counts[profile!] ?? 0) == 0 {
                profile = signedIn.count == 1 ? signedIn.first?.directory : (signedIn.isEmpty ? nil : profile)
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

    private func step<Content: View>(_ n: Int, _ title: String, @ViewBuilder content: () -> Content) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text("\(n)")
                .font(.caption.weight(.bold)).foregroundStyle(.white)
                .frame(width: 18, height: 18)
                .background(Color.accentColor, in: Circle())
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.headline)
                content()
            }
        }
    }
}
