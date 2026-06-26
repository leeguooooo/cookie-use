import SwiftUI

/// The dropdown shown from the menu-bar item: search + accounts grouped by site,
/// one tap to switch into the live Chrome session.
struct MenuBarView: View {
    @ObservedObject var model: AppModel
    var onOpenWindow: () -> Void
    var onCapture: () -> Void
    @State private var hoveredID: String?

    var body: some View {
        GlassEffectContainer(spacing: 10) {
            VStack(spacing: 0) {
                header
                searchBar
                Divider().padding(.horizontal, 14)
                accountList
                Divider()
                footer
            }
            .frame(width: 360)
            .glassEffect(.regular, in: .rect(cornerRadius: DS.panelRadius))
        }
        .padding(8)
        .task { await model.refresh() }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Image(systemName: "person.2.badge.key.fill")
                .font(.title3)
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 1) {
                Text("cookie-use").font(.headline)
                HStack(spacing: 5) {
                    Circle()
                        .fill(model.chromeConnected ? Color.green : Color.orange)
                        .frame(width: 7, height: 7)
                    Text(model.chromeConnected ? "Chrome connected · \(model.targetSession)" : "Chrome not connected")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            Button { Task { await model.refresh() } } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.top, 14)
        .padding(.bottom, 10)
    }

    private var searchBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search accounts…", text: $model.searchQuery)
                .textFieldStyle(.plain)
        }
        .padding(8)
        .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: DS.rowRadius))
        .padding(.horizontal, 14)
        .padding(.bottom, 8)
    }

    private var accountList: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 2) {
                if let err = model.lastError {
                    Label(err, systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange)
                        .padding(.horizontal, 14).padding(.vertical, 8)
                } else if model.groupedAccounts.isEmpty {
                    Text(model.isLoading ? "Loading…" : "No accounts yet.")
                        .font(.callout).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 120)
                }
                ForEach(model.groupedAccounts, id: \.site) { group in
                    Text(group.site)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 14)
                        .padding(.top, 8)
                    ForEach(group.accounts) { account in
                        row(account)
                    }
                }
            }
            .padding(.bottom, 6)
        }
        .frame(height: 300)
    }

    private func row(_ account: AccountSummary) -> some View {
        let isCurrent = account.id == model.currentAccountID
        return Button {
            Task { await model.switchInto(account) }
        } label: {
            HStack(spacing: 10) {
                Circle().fill(account.status.color).frame(width: 7, height: 7)
                VStack(alignment: .leading, spacing: 0) {
                    Text(account.displayName)
                        .font(.body)
                        .foregroundStyle(isCurrent ? Color.white : .primary)
                    if let hint = account.accountHint, !hint.isEmpty {
                        Text(hint).font(.system(size: 10)).foregroundStyle(isCurrent ? Color.white.opacity(0.7) : .secondary)
                    }
                }
                Spacer()
                if isCurrent {
                    Image(systemName: "checkmark").font(.caption.weight(.bold)).foregroundStyle(.white)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .contentShape(.rect(cornerRadius: DS.rowRadius))
            .selectionGlass(isCurrent)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 6)
    }

    private var footer: some View {
        HStack(spacing: 10) {
            Button(action: onCapture) { Label("Capture", systemImage: "plus.circle") }
            Button(action: onOpenWindow) { Label("Manage", systemImage: "gearshape") }
            Spacer()
            Button { NSApp.terminate(nil) } label: { Image(systemName: "power") }
                .foregroundStyle(.secondary)
        }
        .buttonStyle(.plain)
        .font(.callout)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }
}
