import SwiftUI

/// Shared visual language, matched verbatim to the ChooseBrowser app
/// (macOS 26 Liquid Glass, accent-driven, restrained). Corner-radius scale:
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
            glassEffect(
                .regular.tint(.accentColor).interactive(),
                in: .rect(cornerRadius: DS.rowRadius, style: .continuous)
            )
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
}

/// Small status capsule used in rows and headers.
struct StatusBadge: View {
    let status: AccountStatus
    var onGlass: Bool = false

    var body: some View {
        Text(status.rawValue)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .foregroundStyle(onGlass ? Color.white.opacity(0.9) : status.color)
            .background(
                (onGlass ? Color.white.opacity(0.2) : status.color.opacity(0.15)),
                in: Capsule()
            )
    }
}
