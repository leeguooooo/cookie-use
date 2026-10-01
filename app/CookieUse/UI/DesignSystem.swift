import SwiftUI

/// Shared visual language, matched to the ChooseBrowser app (macOS 26 Liquid
/// Glass, accent-driven, restrained). Corner-radius scale:
/// 22 panel · 14 cards · 8 rows/fields · 4 small badges.
enum DS {
    static let panelRadius: CGFloat = 22
    static let cardRadius: CGFloat = 14
    static let rowRadius: CGFloat = 8
    static let badgeRadius: CGFloat = 4
}

extension View {
    /// Accent-tinted interactive glass applied ONLY to a selected/active row;
    /// non-selected rows stay transparent so the card shows through.
    @ViewBuilder
    func selectionGlass(_ isActive: Bool) -> some View {
        if isActive {
            glassEffect(.regular.tint(.accentColor).interactive(), in: .rect(cornerRadius: DS.rowRadius, style: .continuous))
        } else {
            self
        }
    }

    /// A grouped material info card with a hairline tinted border.
    func infoCard(border: Color = .secondary, radius: CGFloat = DS.cardRadius) -> some View {
        padding(16)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: radius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: radius, style: .continuous)
                    .strokeBorder(border.opacity(0.25), lineWidth: 1)
            )
    }

    /// The translucent inset used by search boxes and key caps.
    func insetField() -> some View {
        padding(8).background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: DS.rowRadius, style: .continuous))
    }
}

extension AccountStatus {
    var color: Color {
        switch self {
        case .live: return .green
        case .expired: return .orange
        case .unknown: return .secondary
        }
    }

    var symbol: String {
        switch self {
        case .live: return "checkmark.seal.fill"
        case .expired: return "exclamationmark.triangle.fill"
        case .unknown: return "questionmark.circle"
        }
    }

    var title: String {
        switch self {
        case .live: return "Active"
        case .expired: return "Expired"
        case .unknown: return "Unchecked"
        }
    }
}

/// Site favicon from Chrome's local cache, or a tinted monogram.
struct SiteIcon: View {
    let host: String
    var size: CGFloat = 20
    @ObservedObject private var favicons = Favicons.shared

    var body: some View {
        Group {
            if let img = favicons.icon(for: host) {
                Image(nsImage: img).resizable().interpolation(.high).scaledToFit()
                    .padding(size * 0.08)
            } else {
                Text(monogram)
                    .font(.system(size: size * 0.5, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(tint.gradient)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.24, style: .continuous))
    }

    private var monogram: String { String(Favicons.key(host).prefix(1)).uppercased() }

    private var tint: Color {
        let palette: [Color] = [.blue, .purple, .pink, .orange, .teal, .indigo, .mint, .brown]
        let sum = Favicons.key(host).unicodeScalars.reduce(0) { $0 + Int($1.value) }
        return palette[sum % palette.count]
    }
}

/// Small status capsule used in headers.
struct StatusBadge: View {
    let account: AccountSummary

    var body: some View {
        let status = account.effectiveStatus
        Label(label, systemImage: status.symbol)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .foregroundStyle(account.expiresSoon() ? .yellow : status.color)
            .background((account.expiresSoon() ? Color.yellow : status.color).opacity(0.15), in: Capsule())
    }

    private var label: String {
        if account.expiresSoon(), let until = account.liveUntilDate { return "Expires \(until.relative)" }
        return account.effectiveStatus.title
    }
}

/// The 7pt dot rows use; yellow when the session is about to lapse.
struct StatusDot: View {
    let account: AccountSummary
    var body: some View {
        Circle()
            .fill(account.expiresSoon() ? Color.yellow : account.effectiveStatus.color)
            .frame(width: 7, height: 7)
            .help(account.effectiveStatus.title)
    }
}

struct TagChip: View {
    let tag: String
    var onGlass = false

    var body: some View {
        Text(tag)
            .font(.system(size: 10, weight: .medium))
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .foregroundStyle(onGlass ? Color.white.opacity(0.9) : .secondary)
            .background(onGlass ? Color.white.opacity(0.2) : Color.primary.opacity(0.07),
                        in: RoundedRectangle(cornerRadius: DS.badgeRadius, style: .continuous))
    }
}

/// A key cap for shortcut hints ("↩", "⌘↩").
struct KeyCap: View {
    let key: String
    var body: some View {
        Text(key)
            .font(.system(size: 10, weight: .medium, design: .rounded))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: DS.badgeRadius))
    }
}

/// Inline, auto-dismissing status line (success or error).
struct BannerView: View {
    let banner: AppModel.Banner
    var action: AppModel.BannerAction?

    var body: some View {
        HStack(spacing: 8) {
            Label(banner.text, systemImage: banner.isError ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                .foregroundStyle(banner.isError ? Color.orange : Color.primary)
                .symbolRenderingMode(.multicolor)
                .lineLimit(3)
            Spacer(minLength: 0)
            if let action { Button(action.title, action: action.run).controlSize(.small) }
        }
            .font(.callout)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: DS.rowRadius, style: .continuous))
            .transition(.move(edge: .bottom).combined(with: .opacity))
    }
}

/// Shown instead of the list when a required CLI is missing.
struct MissingToolsView: View {
    let missing: [String]
    var onRetry: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("Finish setup", systemImage: "wrench.and.screwdriver").font(.headline)
            Text("CookieUse drives two command-line tools. Install the missing one, then retry.")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(missing, id: \.self) { name in
                let cmd = "curl -fsSL https://raw.githubusercontent.com/leeguooooo/\(name)/main/install.sh | sh"
                HStack(spacing: 6) {
                    Text(cmd).font(.system(size: 10, design: .monospaced)).lineLimit(2).textSelection(.enabled)
                    Spacer(minLength: 0)
                    Button { Clipboard.copy(cmd) } label: { Image(systemName: "doc.on.doc") }
                        .buttonStyle(.borderless).help("Copy")
                }
                .insetField()
            }
            Button("Retry", action: onRetry).buttonStyle(.borderedProminent)
        }
        .padding(16)
    }
}
