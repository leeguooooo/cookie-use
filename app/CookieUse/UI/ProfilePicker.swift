import SwiftUI

/// Probes every local Chrome profile for how many cookies it holds for a site
/// (shared by "Save a login" and "Copy between profiles").
@MainActor
final class ProfileProbe: ObservableObject {
    @Published private(set) var profiles: [ChromeProfile] = []
    @Published private(set) var counts: [String: Int] = [:]
    @Published private(set) var probing = false
    private var task: Task<Void, Never>?
    private(set) var site = ""

    func load() async {
        if profiles.isEmpty { profiles = await CLIBridge.shared.chromeProfiles() }
    }

    /// Re-probe for `site` (debounced unless `immediately`); `onDone` runs once all answered.
    func probe(_ site: String, immediately: Bool = false, onDone: @escaping () -> Void = {}) {
        task?.cancel()
        counts = [:]
        self.site = site
        guard !site.isEmpty, !profiles.isEmpty else { probing = false; return }
        probing = true
        let list = profiles
        task = Task {
            if !immediately { try? await Task.sleep(for: .milliseconds(450)) }
            guard !Task.isCancelled else { return }
            await withTaskGroup(of: (String, Int).self) { group in
                for p in list {
                    group.addTask { (p.directory, await CLIBridge.shared.cookieCount(profile: p.directory, site: site)) }
                }
                for await (dir, n) in group {
                    guard !Task.isCancelled else { return }
                    counts[dir] = n
                }
            }
            guard !Task.isCancelled else { return }
            probing = false
            onDone()
        }
    }

    func count(_ p: ChromeProfile) -> Int { counts[p.directory] ?? 0 }
    var withCookies: [ChromeProfile] { profiles.filter { count($0) > 0 } }

    /// Most cookies first (a real login sets far more than analytics), then the rest.
    var sorted: [ChromeProfile] {
        profiles.filter { count($0) > 0 }.sorted { count($0) > count($1) } + profiles.filter { count($0) == 0 }
    }
}

/// A radio list of Chrome profiles with their cookie count for the probed site.
struct ProfileList: View {
    @ObservedObject var probe: ProfileProbe
    @Binding var selection: String?
    var sortByCookies = true
    /// Non-nil → the row is disabled and shows this reason.
    var unavailable: (ChromeProfile) -> String? = { _ in nil }
    var maxHeight: CGFloat = 168

    var body: some View {
        if probe.profiles.isEmpty {
            Text("No Chrome profiles found.").font(.callout).foregroundStyle(.secondary)
        } else {
            ScrollView {
                VStack(spacing: 1) {
                    ForEach(sortByCookies ? probe.sorted : probe.profiles) { row($0) }
                }
            }
            .frame(maxHeight: maxHeight)
            .padding(4)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: DS.rowRadius))
        }
    }

    private func row(_ p: ChromeProfile) -> some View {
        let selected = selection == p.directory
        let reason = unavailable(p)
        let n = probe.counts[p.directory]
        return HStack(spacing: 8) {
            Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                .foregroundStyle(selected ? Color.accentColor : .secondary)
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 6) {
                    Text(p.name)
                    Text(p.directory).font(.caption).foregroundStyle(.tertiary)
                }
                if let reason { Text(reason).font(.caption2).foregroundStyle(.secondary) }
            }
            Spacer()
            if probe.site.isEmpty {
                EmptyView()
            } else if probe.probing && n == nil {
                ProgressView().controlSize(.mini)
            } else if let n, n > 0 {
                Label("\(n) cookies", systemImage: "checkmark.circle.fill")
                    .font(.caption.monospacedDigit()).foregroundStyle(.green)
            } else {
                Text("no cookies").font(.caption).foregroundStyle(.tertiary)
            }
        }
        .font(.callout)
        .opacity(reason == nil ? 1 : 0.5)
        .padding(.horizontal, 8).padding(.vertical, 5)
        .contentShape(Rectangle())
        .background(selected ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 6))
        .onTapGesture { if reason == nil { selection = p.directory } }
    }
}

/// "1 Website" style numbered step used by the multi-step sheets.
struct SheetStep<Content: View>: View {
    let number: Int
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Text("\(number)")
                .font(.caption.weight(.bold)).foregroundStyle(.white)
                .frame(width: 18, height: 18)
                .background(Color.accentColor, in: Circle())
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.headline)
                content
            }
        }
    }
}

extension String {
    /// "dash.cloudflare.com" (or a URL) → "dash.cloudflare.com,cloudflare.com":
    /// logins usually live on the parent domain, and the CLI opens the first host.
    var withBaseDomains: String {
        var hosts: [String] = []
        for h in split(separator: ",").map({ String($0).normalizedHost }) where !h.isEmpty {
            for candidate in [h, Favicons.key(h)] where candidate.contains(".") && !hosts.contains(candidate) {
                hosts.append(candidate)
            }
        }
        return hosts.joined(separator: ",")
    }
}
