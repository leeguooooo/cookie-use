import SwiftUI

/// The full management window: grouped account list on the left, detail + actions
/// on the right. Capture / share / redeem sheets attach here.
struct ManagementView: View {
    @ObservedObject var model: AppModel
    @State private var selection: String?

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            if let selection, let account = model.accounts.first(where: { $0.id == selection }) {
                DetailPane(model: model, summary: account)
                    .id(account.id)
            } else {
                ContentUnavailableView("Select an account", systemImage: "person.crop.circle")
            }
        }
        .frame(minWidth: 720, minHeight: 520)
        .task { await model.refresh() }
        .sheet(item: $model.sheet) { sheet in
            switch sheet {
            case .capture: CaptureSheet(model: model)
            case .importFile: ImportSheet(model: model)
            case .redeem: RedeemSheet(model: model)
            case let .share(account): ShareSheet(account: account, model: model)
            }
        }
        .safeAreaInset(edge: .bottom) {
            if let msg = model.lastMessage {
                Label(msg, systemImage: "checkmark.circle.fill")
                    .font(.caption).foregroundStyle(.secondary)
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.regularMaterial)
            }
        }
    }

    private var sidebar: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search…", text: $model.searchQuery).textFieldStyle(.plain)
            }
            .padding(8)
            .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: DS.rowRadius))
            .padding(12)

            List(selection: $selection) {
                ForEach(model.groupedAccounts, id: \.site) { group in
                    Section(group.site) {
                        ForEach(group.accounts) { account in
                            HStack(spacing: 8) {
                                Circle().fill(account.status.color).frame(width: 7, height: 7)
                                Text(account.displayName)
                                Spacer()
                                if account.id == model.currentAccountID {
                                    Image(systemName: "checkmark.circle.fill").foregroundStyle(.tint)
                                }
                            }
                            .tag(account.id)
                        }
                    }
                }
            }
        }
        .frame(minWidth: 240)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button { model.sheet = .capture } label: { Label("Capture", systemImage: "plus.circle") }
                Menu {
                    Button { model.sheet = .importFile } label: { Label("Import from file…", systemImage: "square.and.arrow.down") }
                    Button { model.sheet = .redeem } label: { Label("Redeem bundle…", systemImage: "gift") }
                } label: { Image(systemName: "ellipsis.circle") }
                Button { Task { await model.refresh() } } label: { Image(systemName: "arrow.clockwise") }
            }
        }
    }
}

/// Right-hand detail: metadata loaded via `show --json`, plus the per-account actions.
struct DetailPane: View {
    @ObservedObject var model: AppModel
    let summary: AccountSummary
    @State private var detail: Account?
    @State private var loadError: String?
    @State private var showRename = false
    @State private var renameText = ""
    @State private var showRemove = false
    @State private var replayOrigin = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                header
                actions
                if let detail { metadata(detail) }
                else if let loadError { Text(loadError).foregroundStyle(.orange) }
                else { ProgressView().padding() }
                Spacer(minLength: 0)
            }
            .padding(24)
        }
        .navigationTitle(summary.displayName)
        .task(id: summary.id) { await load() }
    }

    private var header: some View {
        HStack(spacing: 12) {
            Image(systemName: summary.status.symbol)
                .font(.largeTitle)
                .foregroundStyle(summary.status.color)
            VStack(alignment: .leading, spacing: 3) {
                Text(summary.displayName).font(.title2.weight(.semibold))
                Text(summary.id).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            StatusBadge(status: summary.status)
        }
    }

    private var actions: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                Button { Task { await model.switchInto(summary) } } label: {
                    Label("Switch into Chrome", systemImage: "arrow.right.circle.fill")
                }
                .buttonStyle(.borderedProminent).controlSize(.large)
                .disabled(!model.chromeConnected)

                Button { Task { await model.runAll(site: summary.primarySite) } } label: {
                    Label("Run all", systemImage: "rectangle.split.3x1")
                }
                .buttonStyle(.bordered).controlSize(.large)

                Button { model.sheet = .share(summary) } label: {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(.bordered).controlSize(.large)

                Menu {
                    Button { renameText = summary.id; showRename = true } label: { Label("Rename…", systemImage: "pencil") }
                    Button(role: .destructive) { showRemove = true } label: { Label("Remove", systemImage: "trash") }
                } label: { Image(systemName: "ellipsis.circle") }
                    .menuStyle(.borderlessButton).frame(width: 28)

                Spacer(minLength: 0)
            }

            HStack(spacing: 8) {
                Image(systemName: "arrow.triangle.2.circlepath").foregroundStyle(.secondary)
                TextField("Replay on dev origin (e.g. localhost:8001)", text: $replayOrigin)
                    .textFieldStyle(.roundedBorder)
                Button("Replay") { Task { await model.replay(summary, to: replayOrigin) } }
                    .disabled(replayOrigin.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .alert("Rename account", isPresented: $showRename) {
            TextField("New id", text: $renameText)
            Button("Cancel", role: .cancel) {}
            Button("Rename") { Task { _ = await model.rename(summary, to: renameText) } }
        }
        .confirmationDialog("Remove “\(summary.id)”? This deletes the stored session.",
                            isPresented: $showRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { Task { _ = await model.remove(summary) } }
            Button("Cancel", role: .cancel) {}
        }
    }

    @ViewBuilder
    private func metadata(_ a: Account) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            row("Site", a.site)
            row("Cookies", "\(a.cookies)")
            if !a.localStorage.isEmpty { row("localStorage", a.localStorage.joined(separator: ", ")) }
            row("Domains", a.domains.joined(separator: ", "))
            row("Expires", a.sessionOnly ? "session cookies only" : (a.expires ?? "—"))
            row("Created", a.createdAt)
            row("Updated", a.updatedAt)
            if let last = a.lastUsedAt { row("Last used", last) }
        }
        .font(.callout)
        .infoCard(border: a.status.color)
    }

    private func row(_ key: String, _ value: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text(key).foregroundStyle(.secondary).frame(width: 92, alignment: .leading)
            Text(value).textSelection(.enabled)
            Spacer(minLength: 0)
        }
    }

    private func load() async {
        do { detail = try await CLIBridge.shared.show(id: summary.id); loadError = nil }
        catch { loadError = error.localizedDescription }
    }
}
