import SwiftUI

/// The menu-bar quick switcher (also summoned with ⌥⌘K). Keyboard-first:
/// type to filter, ↑↓ to move, ↩ signs in to Chrome, ⌘↩ opens a separate
/// window, ⌘1–9 jump straight to pinned/recent accounts.
struct MenuBarView: View {
    @ObservedObject var model: AppModel
    var onOpenWindow: (String?) -> Void
    var onCapture: () -> Void
    var onSettings: () -> Void

    @State private var query = ""
    @State private var selectedKey: String?
    @State private var hoveredKey: String?
    @FocusState private var searchFocused: Bool

    private struct Section: Identifiable {
        let title: String
        let accounts: [AccountSummary]
        var id: String { title }
    }

    private struct Entry: Identifiable {
        let key: String
        let account: AccountSummary
        var id: String { key }
    }

    var body: some View {
        GlassEffectContainer(spacing: 10) {
            VStack(spacing: 0) {
                header
                searchBar
                Divider().padding(.horizontal, 14)
                content
                if let banner = model.banner {
                    BannerView(banner: banner).padding(.horizontal, 10).padding(.bottom, 6)
                }
                Divider()
                footer
            }
            .frame(width: 380)
            .glassEffect(.regular, in: .rect(cornerRadius: DS.panelRadius))
        }
        .padding(8)
        .animation(.snappy(duration: 0.2), value: model.banner)
        .onAppear {
            query = ""
            searchFocused = true
            selectedKey = entries.first?.key
        }
        .onChange(of: query) { selectedKey = entries.first?.key }
        .onChange(of: model.accounts) {
            if selectedKey == nil || !entries.contains(where: { $0.key == selectedKey }) {
                selectedKey = entries.first?.key
            }
        }
    }

    // MARK: Data

    private var sections: [Section] {
        let q = query.trimmingCharacters(in: .whitespaces)
        if !q.isEmpty {
            let hits = model.accounts.filter { $0.matches(q) }
                .sorted { rank($0, q) == rank($1, q) ? $0.displayName < $1.displayName : rank($0, q) < rank($1, q) }
            return hits.isEmpty ? [] : [Section(title: "Results", accounts: hits)]
        }
        var out: [Section] = []
        if !model.pinnedAccounts.isEmpty { out.append(Section(title: "Pinned", accounts: model.pinnedAccounts)) }
        let recent = Array(model.recentAccounts.prefix(4))
        if !recent.isEmpty { out.append(Section(title: "Recent", accounts: recent)) }
        let grouped = Dictionary(grouping: model.accounts, by: { Favicons.key($0.primarySite) })
        for site in grouped.keys.sorted() {
            out.append(Section(title: site, accounts: grouped[site]!.sorted { $0.displayName.lowercased() < $1.displayName.lowercased() }))
        }
        return out
    }

    /// Prefix hits on name/site beat substring hits anywhere.
    private func rank(_ a: AccountSummary, _ q: String) -> Int {
        let q = q.lowercased()
        if a.displayName.lowercased().hasPrefix(q) { return 0 }
        if a.primarySite.lowercased().hasPrefix(q) || Favicons.key(a.primarySite).hasPrefix(q) { return 1 }
        if a.id.lowercased().contains(q) { return 2 }
        return 3
    }

    private var entries: [Entry] {
        sections.flatMap { s in s.accounts.map { Entry(key: "\(s.title)|\($0.id)", account: $0) } }
    }

    private var selectedAccount: AccountSummary? {
        entries.first { $0.key == selectedKey }?.account
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: 10) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable().frame(width: 26, height: 26)
            VStack(alignment: .leading, spacing: 1) {
                Text("CookieUse").font(.headline)
                HStack(spacing: 5) {
                    Circle().fill(model.chromeConnected ? Color.green : Color.orange).frame(width: 6, height: 6)
                    Text(model.chromeLabel).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer()
            if model.isLoading { ProgressView().controlSize(.small) }
            Menu {
                Button("Save a login…", action: onCapture)
                Button("Open manager") { onOpenWindow(nil) }
                Button("Check all sessions") { Task { await model.checkAll() } }
                Divider()
                Button("Settings…", action: onSettings)
                Button("Quit CookieUse") { NSApp.terminate(nil) }
            } label: {
                Image(systemName: "ellipsis.circle").font(.title3)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
        }
        .padding(.horizontal, 14)
        .padding(.top, 14)
        .padding(.bottom, 10)
    }

    private var searchBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search accounts, sites, tags…", text: $query)
                .textFieldStyle(.plain)
                .focused($searchFocused)
                .onKeyPress(.downArrow) { move(1); return .handled }
                .onKeyPress(.upArrow) { move(-1); return .handled }
                .onKeyPress(.return, phases: .down) { press in
                    guard let a = selectedAccount else { return .ignored }
                    let dest: AppModel.Destination? = press.modifiers.contains(.command)
                        ? (model.prefs.openIsolatedByDefault ? .chrome : .isolated) : nil
                    Task { await model.open(a, in: dest) }
                    return .handled
                }
                .onKeyPress(phases: .down) { press in
                    // ⌘1–9: jump to the nth visible account.
                    guard press.modifiers.contains(.command), let n = Int(String(press.characters)), (1...9).contains(n),
                          n <= entries.count else { return .ignored }
                    Task { await model.open(entries[n - 1].account) }
                    return .handled
                }
            if !query.isEmpty {
                Button { query = "" } label: { Image(systemName: "xmark.circle.fill") }
                    .buttonStyle(.plain).foregroundStyle(.tertiary)
            }
        }
        .insetField()
        .padding(.horizontal, 14)
        .padding(.bottom, 8)
    }

    private func move(_ delta: Int) {
        let keys = entries.map(\.key)
        guard !keys.isEmpty else { return }
        let i = keys.firstIndex(of: selectedKey ?? "") ?? -1
        selectedKey = keys[max(0, min(keys.count - 1, i + delta))]
    }

    // MARK: List

    @ViewBuilder
    private var content: some View {
        if !model.missingBinaries.isEmpty {
            MissingToolsView(missing: model.missingBinaries) { Task { await model.refresh() } }
        } else if model.accounts.isEmpty {
            emptyState
        } else if sections.isEmpty {
            VStack(spacing: 8) {
                Image(systemName: "magnifyingglass").font(.title2).foregroundStyle(.tertiary)
                Text("No account matches “\(query)”").font(.callout).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, minHeight: 160)
        } else {
            list
        }
    }

    private var emptyState: some View {
        VStack(spacing: 10) {
            if model.hasLoaded {
                Image(systemName: "person.crop.circle.badge.plus").font(.system(size: 34)).foregroundStyle(.tint)
                Text("No saved logins yet").font(.headline)
                Text("Log in to a site in Chrome, then save that login here. Switch back to it any time.")
                    .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Save a login…", action: onCapture).buttonStyle(.borderedProminent)
            } else {
                ProgressView()
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, minHeight: 200)
    }

    private var list: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    let numbered = Dictionary(uniqueKeysWithValues: entries.prefix(9).enumerated().map { ($1.key, $0 + 1) })
                    ForEach(sections) { section in
                        sectionHeader(section)
                        ForEach(section.accounts) { account in
                            let key = "\(section.title)|\(account.id)"
                            row(account, key: key, number: numbered[key], inSiteGroup: section.title.contains(".")).id(key)
                        }
                    }
                }
                .padding(.bottom, 6)
            }
            .frame(height: min(400, CGFloat(entries.count) * 42 + CGFloat(sections.count) * 28 + 12))
            .onChange(of: selectedKey) { _, key in
                if let key { withAnimation(.easeOut(duration: 0.1)) { proxy.scrollTo(key) } }
            }
        }
    }

    private func sectionHeader(_ section: Section) -> some View {
        HStack(spacing: 6) {
            if section.title.contains(".") { SiteIcon(host: section.title, size: 12) }
            Text(section.title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            Spacer()
            if section.title.contains("."), section.accounts.count > 1 {
                Button {
                    Task { await model.openSideBySide(section.accounts) }
                } label: {
                    Image(systemName: "rectangle.split.3x1").font(.caption)
                }
                .buttonStyle(.plain).foregroundStyle(.tertiary)
                .help("Open all \(section.accounts.count) side by side")
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 2)
    }

    private func row(_ account: AccountSummary, key: String, number: Int?, inSiteGroup: Bool) -> some View {
        let selected = key == selectedKey
        let hovered = key == hoveredKey
        let busy = model.busyID == account.id
        return HStack(spacing: 10) {
            SiteIcon(host: account.primarySite, size: 22)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(account.displayName).lineLimit(1)
                        .foregroundStyle(selected ? Color.white : .primary)
                    ForEach(account.tagList.prefix(2), id: \.self) { TagChip(tag: $0, onGlass: selected) }
                }
                Text(subtitle(account, inSiteGroup: inSiteGroup)).font(.system(size: 10)).lineLimit(1)
                    .foregroundStyle(selected ? Color.white.opacity(0.7) : .secondary)
            }
            Spacer(minLength: 4)
            if busy {
                ProgressView().controlSize(.small)
            } else if hovered || selected {
                Button { model.prefs.togglePin(account.id) } label: {
                    Image(systemName: model.prefs.isPinned(account.id) ? "pin.fill" : "pin")
                }
                .buttonStyle(.plain).help(model.prefs.isPinned(account.id) ? "Unpin" : "Pin")
                Button { Task { await model.open(account, in: .isolated) } } label: {
                    Image(systemName: "macwindow.badge.plus")
                }
                .buttonStyle(.plain).help("Open in a separate window (⌘↩)")
            } else if let number {
                KeyCap(key: "⌘\(number)")
            }
            StatusDot(account: account)
        }
        .font(.callout)
        .foregroundStyle(selected ? Color.white.opacity(0.9) : .secondary)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(.rect(cornerRadius: DS.rowRadius))
        .selectionGlass(selected)
        .padding(.horizontal, 6)
        .onHover { hoveredKey = $0 ? key : (hoveredKey == key ? nil : hoveredKey) }
        .onTapGesture { selectedKey = key; Task { await model.open(account) } }
        .contextMenu { rowMenu(account) }
    }

    /// Inside a site group the header already names the site, so show what
    /// tells accounts apart instead: the login hint, else the id.
    private func subtitle(_ a: AccountSummary, inSiteGroup: Bool) -> String {
        var parts: [String] = []
        if let hint = a.accountHint?.nilIfBlank { parts.append(hint) }
        if parts.isEmpty { parts.append(inSiteGroup ? a.id : a.primarySite) }
        if a.effectiveStatus == .expired { parts.append("expired — refresh login") }
        else if a.expiresSoon(), let d = a.liveUntilDate { parts.append("expires \(d.relative)") }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private func rowMenu(_ a: AccountSummary) -> some View {
        Button("Sign in to Chrome") { Task { await model.open(a, in: .chrome) } }
        Button("Open in a separate window") { Task { await model.open(a, in: .isolated) } }
        Divider()
        Button(model.prefs.isPinned(a.id) ? "Unpin" : "Pin") { model.prefs.togglePin(a.id) }
        Button("Copy id") { Clipboard.copy(a.id) }
        Button("Copy agent command") { Clipboard.copy("cookie-use as \(a.id) -- <command>") }
        Divider()
        Button("Show in manager") { onOpenWindow(a.id) }
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: 12) {
            Button(action: onCapture) { Label("Save login", systemImage: "plus.circle") }
            Button { onOpenWindow(nil) } label: { Label("Manage", systemImage: "sidebar.left") }
            Spacer()
            HStack(spacing: 4) {
                KeyCap(key: "↩")
                Text(model.prefs.openIsolatedByDefault ? "window" : "Chrome")
                KeyCap(key: "⌘↩")
                Text(model.prefs.openIsolatedByDefault ? "Chrome" : "window")
            }
            .font(.caption2).foregroundStyle(.tertiary)
        }
        .buttonStyle(.plain)
        .font(.callout)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }
}
