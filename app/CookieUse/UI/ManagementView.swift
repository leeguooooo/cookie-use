import SwiftUI
import UniformTypeIdentifiers

/// The full manager: scopes (library / sites / tags) → accounts → detail.
/// Drop a `.cusession` bundle or a cookie export anywhere to bring it in.
struct ManagementView: View {
    @ObservedObject var model: AppModel
    @Binding var selection: String?
    @State private var scope: AppModel.Scope? = .all
    @State private var query = ""
    @State private var dropTargeted = false

    var body: some View {
        NavigationSplitView {
            sidebar
        } content: {
            accountList
        } detail: {
            if let account = model.account(selection) {
                DetailPane(model: model, account: account).id(account.id)
            } else if model.accounts.isEmpty, model.hasLoaded {
                ContentUnavailableView {
                    Label("No saved logins", systemImage: "person.crop.circle.badge.plus")
                } description: {
                    Text("Save a login from Chrome, or drop a .cusession bundle or cookie export here.")
                } actions: {
                    Button("Save a login…") { model.sheet = .capture(prefill: nil) }.buttonStyle(.borderedProminent)
                }
            } else {
                ContentUnavailableView("Select an account", systemImage: "person.crop.circle")
            }
        }
        .frame(minWidth: 860, minHeight: 540)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button { model.sheet = .capture(prefill: nil) } label: { Label("Save login", systemImage: "plus") }
                    .help("Save a login from Chrome (⌘N)")
                    .keyboardShortcut("n")
                Button { model.sheet = .copy } label: { Label("Copy between profiles", systemImage: "arrow.right.doc.on.clipboard") }
                    .help("Copy a site’s login from one Chrome profile into another")
                Button { model.sheet = .cloud } label: {
                    Label("Cloud sync", systemImage: model.syncing ? "arrow.triangle.2.circlepath" : "arrow.triangle.2.circlepath.icloud")
                }
                .help(model.cloud?.configured == true ? "Cloud sync is on — click to sync or change it" : "Sync logins between your Macs")
                Menu {
                    Button("Export logins…") { model.sheet = .export(site: nil) }
                    Button("Import a .cusession file…") { model.sheet = .redeem(nil) }
                    Button("Import a cookie file…") { model.sheet = .importFile(nil) }
                    Divider()
                    Button("Check all sessions") { Task { await model.checkAll() } }
                } label: { Label("More", systemImage: "square.and.arrow.down") }
                Button { Task { await model.refresh() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                    .keyboardShortcut("r")
            }
        }
        .overlay(alignment: .bottom) {
            if let banner = model.banner {
                BannerView(banner: banner, action: model.bannerAction).frame(maxWidth: 560).padding(14)
            }
        }
        .overlay {
            if dropTargeted {
                RoundedRectangle(cornerRadius: DS.cardRadius).strokeBorder(Color.accentColor, style: StrokeStyle(lineWidth: 3, dash: [8]))
                    .background(Color.accentColor.opacity(0.06))
                    .overlay(Label("Drop to import", systemImage: "square.and.arrow.down").font(.title2.weight(.semibold)))
                    .padding(8)
                    .allowsHitTesting(false)
            }
        }
        .animation(.snappy(duration: 0.2), value: model.banner)
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            model.handleFile(url)
            return true
        } isTargeted: { dropTargeted = $0 }
        .task { await model.refresh(); await model.loadCloud() }
    }

    // MARK: Sidebar

    private var sidebar: some View {
        List(selection: $scope) {
            Section("Library") {
                Label { HStack { Text("All accounts"); Spacer(); count(model.accounts.count) } } icon: { Image(systemName: "tray.full") }
                    .tag(AppModel.Scope.all)
                Label { HStack { Text("Pinned"); Spacer(); count(model.pinnedAccounts.count) } } icon: { Image(systemName: "pin") }
                    .tag(AppModel.Scope.pinned)
                Label {
                    HStack { Text("Needs attention"); Spacer(); count(model.needsAttention.count, warn: true) }
                } icon: { Image(systemName: "exclamationmark.triangle") }
                    .tag(AppModel.Scope.attention)
            }
            if !model.sites.isEmpty {
                Section("Sites") {
                    ForEach(model.sites, id: \.site) { item in
                        Label {
                            HStack { Text(item.site); Spacer(); count(item.count) }
                        } icon: { SiteIcon(host: item.site, size: 16) }
                            .tag(AppModel.Scope.site(item.site))
                    }
                }
            }
            if !model.allTags.isEmpty {
                Section("Tags") {
                    ForEach(model.allTags, id: \.self) { tag in
                        Label(tag, systemImage: "tag").tag(AppModel.Scope.tag(tag))
                    }
                }
            }
        }
        .navigationSplitViewColumnWidth(min: 190, ideal: 210)
        .safeAreaInset(edge: .bottom) {
            HStack(spacing: 6) {
                Circle().fill(model.chromeConnected ? Color.green : Color.orange).frame(width: 7, height: 7)
                Text(model.chromeLabel).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Spacer()
            }
            .padding(12)
        }
    }

    private func count(_ n: Int, warn: Bool = false) -> some View {
        Text("\(n)").font(.caption.monospacedDigit())
            .foregroundStyle(warn && n > 0 ? Color.orange : .secondary)
    }

    // MARK: Account list

    private var visible: [AccountSummary] { model.accounts(in: scope ?? .all, matching: query) }

    private var accountList: some View {
        List(selection: $selection) {
            ForEach(visible) { account in
                AccountRow(account: account, pinned: model.prefs.isPinned(account.id), busy: model.busyID == account.id)
                    .tag(account.id)
                    .contextMenu {
                        Button("Sign in to Chrome") { Task { await model.open(account, in: .chrome) } }
                        Button("Open in a separate window") { Task { await model.open(account, in: .isolated) } }
                        Divider()
                        Button(model.prefs.isPinned(account.id) ? "Unpin" : "Pin") { model.prefs.togglePin(account.id) }
                        Button("Refresh login…") {
                            model.sheet = .capture(prefill: .init(site: account.site, id: account.id, label: account.label))
                        }
                        Button("Share…") { model.sheet = .share(account) }
                        Button("Export all \(Favicons.key(account.primarySite)) logins…") { model.sheet = .export(site: account.primarySite) }
                    }
            }
        }
        .overlay {
            if visible.isEmpty, !model.accounts.isEmpty {
                ContentUnavailableView.search(text: query)
            }
        }
        .searchable(text: $query, placement: .sidebar, prompt: "Search")
        .navigationSplitViewColumnWidth(min: 250, ideal: 290)
        .toolbar {
            if case let .site(site) = scope ?? .all, visible.count > 1 {
                ToolbarItem {
                    Button { Task { await model.openSideBySide(visible) } } label: {
                        Label("Open all side by side", systemImage: "rectangle.split.3x1")
                    }
                    .help("Open every \(site) account in its own window")
                }
            }
        }
    }
}

private struct AccountRow: View {
    let account: AccountSummary
    let pinned: Bool
    let busy: Bool

    var body: some View {
        HStack(spacing: 10) {
            SiteIcon(host: account.primarySite, size: 24)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    Text(account.displayName).lineLimit(1)
                    if pinned { Image(systemName: "pin.fill").font(.system(size: 9)).foregroundStyle(.secondary) }
                }
                HStack(spacing: 4) {
                    Text(account.accountHint?.nilIfBlank ?? account.primarySite)
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    ForEach(account.tagList.prefix(2), id: \.self) { TagChip(tag: $0) }
                }
            }
            Spacer(minLength: 0)
            if busy { ProgressView().controlSize(.small) } else { StatusDot(account: account) }
        }
        .padding(.vertical, 2)
    }
}

// MARK: - Detail

/// Right-hand detail: editable metadata, session facts from `show --json`,
/// and the per-account actions.
struct DetailPane: View {
    @ObservedObject var model: AppModel
    let account: AccountSummary

    @State private var detail: Account?
    @State private var loadError: String?
    @State private var label = ""
    @State private var hint = ""
    @State private var tags = ""
    @State private var note = ""
    @State private var showRename = false
    @State private var renameText = ""
    @State private var showRemove = false
    @State private var showReplay = false
    @State private var replayOrigin = "localhost:3000"

    private var dirty: Bool {
        label != (account.label ?? "") || hint != (account.accountHint ?? "")
            || normalizedTags != account.tagList || note != (account.note ?? "")
    }

    private var normalizedTags: [String] {
        var out: [String] = []
        for t in tags.split(separator: ",").map({ $0.trimmingCharacters(in: .whitespaces).lowercased() }) where !t.isEmpty && !out.contains(t) {
            out.append(t)
        }
        return out
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                header
                actions
                aboutCard
                sessionCard
                agentCard
            }
            .padding(24)
            .frame(maxWidth: 680, alignment: .leading)
        }
        .navigationTitle(account.displayName)
        .task(id: account) { await load() }
        .onAppear(perform: resetFields)
        .onChange(of: account) { resetFields() }
        .alert("Rename account id", isPresented: $showRename) {
            TextField("New id", text: $renameText)
            Button("Cancel", role: .cancel) {}
            Button("Rename") { Task { _ = await model.rename(account, to: renameText) } }
        } message: {
            Text("Agents and scripts refer to accounts by id.")
        }
        .alert("Replay on a dev origin", isPresented: $showReplay) {
            TextField("localhost:3000", text: $replayOrigin)
            Button("Cancel", role: .cancel) {}
            Button("Replay") { Task { await model.replay(account, to: replayOrigin) } }
        } message: {
            Text("Rewrites the cookies onto a local origin and opens it — test your dev build as this account.")
        }
        .confirmationDialog("Remove “\(account.displayName)”?", isPresented: $showRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { Task { await model.remove(account) } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The saved session is deleted from the vault. Your Chrome stays signed in.")
        }
    }

    private var header: some View {
        HStack(spacing: 14) {
            SiteIcon(host: account.primarySite, size: 48)
            VStack(alignment: .leading, spacing: 3) {
                Text(account.displayName).font(.title2.weight(.semibold))
                HStack(spacing: 6) {
                    Text(account.id).font(.callout.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                    Button { Clipboard.copy(account.id) } label: { Image(systemName: "doc.on.doc") }
                        .buttonStyle(.borderless).help("Copy id")
                }
            }
            Spacer()
            StatusBadge(account: account)
        }
    }

    private var actions: some View {
        HStack(spacing: 10) {
            Button { Task { await model.open(account, in: .chrome) } } label: {
                Label("Sign in to Chrome", systemImage: "arrow.right.circle.fill")
            }
            .buttonStyle(.borderedProminent).controlSize(.large)
            .disabled(model.busyID != nil)
            .help(model.chromeConnected ? "Apply this session to \(model.chromeLabel)" : "Chrome isn’t connected")

            Button { Task { await model.open(account, in: .isolated) } } label: {
                Label("Separate window", systemImage: "macwindow.badge.plus")
            }
            .buttonStyle(.bordered).controlSize(.large)
            .help("A fresh browser with only this account — your Chrome is untouched")

            if account.effectiveStatus == .expired || account.expiresSoon() {
                Button { model.sheet = .capture(prefill: .init(site: account.site, id: account.id, label: account.label)) } label: {
                    Label("Refresh login", systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered).controlSize(.large).tint(.orange)
            }

            Menu {
                Button(model.prefs.isPinned(account.id) ? "Unpin" : "Pin to quick switcher") { model.prefs.togglePin(account.id) }
                Button("Refresh login…") {
                    model.sheet = .capture(prefill: .init(site: account.site, id: account.id, label: account.label))
                }
                Button("Replay on dev origin…") { showReplay = true }
                Button("Share as encrypted bundle…") { model.sheet = .share(account) }
                Divider()
                Button("Rename id…") { renameText = account.id; showRename = true }
                Button("Remove…", role: .destructive) { showRemove = true }
            } label: { Image(systemName: "ellipsis") }
                .menuStyle(.button).menuIndicator(.hidden).controlSize(.large).fixedSize()

            if model.busyID == account.id { ProgressView().controlSize(.small) }
            Spacer(minLength: 0)
        }
    }

    private var aboutCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("About this account").font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
                GridRow {
                    Text("Name").foregroundStyle(.secondary)
                    TextField(account.id.split(separator: "/").last.map(String.init) ?? account.id, text: $label)
                }
                GridRow {
                    Text("Login").foregroundStyle(.secondary)
                    TextField("email or username (display only)", text: $hint)
                }
                GridRow {
                    Text("Tags").foregroundStyle(.secondary)
                    TextField("prod, admin, free-tier", text: $tags)
                }
                GridRow(alignment: .top) {
                    Text("Note").foregroundStyle(.secondary).padding(.top, 4)
                    TextField("e.g. 2FA on the work phone", text: $note, axis: .vertical)
                        .lineLimit(2...5)
                }
            }
            .textFieldStyle(.roundedBorder)
            if dirty {
                HStack {
                    Spacer()
                    Button("Revert", action: resetFields)
                    Button("Save") { save() }.buttonStyle(.borderedProminent).keyboardShortcut("s")
                }
            }
        }
        .font(.callout)
        .infoCard()
    }

    @ViewBuilder
    private var sessionCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Session").font(.headline)
            if let d = detail {
                fact("Sites", d.site.replacingOccurrences(of: ",", with: ", "))
                fact("Cookies", "\(d.cookies) across \(d.domains.count) domain\(d.domains.count == 1 ? "" : "s")")
                if !d.localStorage.isEmpty { fact("localStorage", "\(d.localStorage.count) key(s): \(d.localStorage.prefix(4).joined(separator: ", "))") }
                if d.sessionOnly {
                    fact("Expires", "Session cookies only — the site decides")
                } else if let until = account.liveUntilDate {
                    fact("Valid until", "\(until.formatted(date: .abbreviated, time: .shortened)) (\(until.relative))")
                }
                fact("Updated", relative(d.updatedAt))
                if let last = d.lastUsedAt { fact("Last used", relative(last)) }
                Text("Stored only on this Mac, AES-256-GCM encrypted. Cookie values are never shown.")
                    .font(.caption).foregroundStyle(.tertiary).padding(.top, 2)
            } else if let loadError {
                Text(loadError).foregroundStyle(.orange)
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .font(.callout)
        .infoCard(border: account.effectiveStatus.color)
    }

    private var agentCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Use from scripts and agents", systemImage: "terminal").font(.headline)
            Text("Same vault, same account — an agent can act as it without ever seeing the cookies.")
                .font(.caption).foregroundStyle(.secondary)
            command("cookie-use as \(account.id) -- <your command>")
            command("cookie-use use \(account.id) --target isolated")
            command("cookie-use replay \(account.id) --to localhost:3000")
        }
        .infoCard()
    }

    private func command(_ cmd: String) -> some View {
        HStack {
            Text(cmd).font(.system(size: 11, design: .monospaced)).textSelection(.enabled).lineLimit(1)
            Spacer(minLength: 4)
            Button { Clipboard.copy(cmd) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless).help("Copy")
        }
        .insetField()
    }

    private func fact(_ key: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(key).foregroundStyle(.secondary).frame(width: 96, alignment: .leading)
            Text(value).textSelection(.enabled)
            Spacer(minLength: 0)
        }
    }

    private func relative(_ iso: String) -> String {
        ISO8601.parse(iso).map { "\($0.formatted(date: .abbreviated, time: .shortened)) (\($0.relative))" } ?? iso
    }

    private func resetFields() {
        label = account.label ?? ""
        hint = account.accountHint ?? ""
        tags = account.tagList.joined(separator: ", ")
        note = account.note ?? ""
    }

    private func save() {
        // Only send the fields that changed; "" clears on the CLI side.
        let l = label != (account.label ?? "") ? label : nil
        let h = hint != (account.accountHint ?? "") ? hint : nil
        let n = note != (account.note ?? "") ? note : nil
        let t = normalizedTags != account.tagList ? normalizedTags : nil
        Task { await model.edit(account, label: l, hint: h, note: n, tags: t) }
    }

    private func load() async {
        do { detail = try await CLIBridge.shared.show(id: account.id); loadError = nil }
        catch { loadError = error.localizedDescription }
    }
}
