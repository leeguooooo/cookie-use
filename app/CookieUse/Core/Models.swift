import Foundation

/// Liveness of a stored account, derived by the CLI from cookie expiry.
enum AccountStatus: String, Codable, Equatable {
    case live
    case expired
    case unknown

    /// Unknown statuses from a future CLI decode as `.unknown` rather than failing.
    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = AccountStatus(rawValue: raw) ?? .unknown
    }
}

/// One row from `cookie-use list --json`.
struct AccountSummary: Codable, Equatable, Identifiable, Hashable {
    let id: String
    let site: String
    let label: String?
    let accountHint: String?
    let status: AccountStatus
    let cookies: Int
    let lastUsedAt: String?
    // Added in the GUI overhaul; optional so an older CLI still decodes.
    let note: String?
    let tags: [String]?
    let liveUntil: String?
    let updatedAt: String?

    /// All hosts this session covers.
    var hosts: [String] {
        site.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    /// The base site (first comma-separated host) used to group accounts.
    var primarySite: String { hosts.first ?? site }

    /// Best human label for a row: explicit label, else the id's trailing segment.
    var displayName: String {
        if let label, !label.isEmpty { return label }
        return id.split(separator: "/").last.map(String.init) ?? id
    }

    var tagList: [String] { tags ?? [] }

    var liveUntilDate: Date? { liveUntil.flatMap(ISO8601.parse) }
    var lastUsedDate: Date? { lastUsedAt.flatMap(ISO8601.parse) }

    /// The stored status can go stale (it is computed at capture / check time),
    /// so re-derive it from `live_until` when the CLI provides it.
    var effectiveStatus: AccountStatus {
        if let until = liveUntilDate { return until < Date() ? .expired : .live }
        return status
    }

    /// True when the session dies within `days` — worth a gentle heads-up.
    func expiresSoon(within days: Double = 3) -> Bool {
        guard let until = liveUntilDate, effectiveStatus == .live else { return false }
        return until.timeIntervalSinceNow < days * 86_400
    }

    func matches(_ query: String) -> Bool {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return true }
        // Every whitespace-separated term must hit somewhere ("cf work" → cloudflare/p3-work).
        let hay = ([id, site, displayName, accountHint ?? "", note ?? ""] + tagList)
            .joined(separator: " ").lowercased()
        return q.split(separator: " ").allSatisfy { hay.contains($0) }
    }
}

/// Full detail from `cookie-use show <id> --json`.
struct Account: Codable, Equatable, Identifiable {
    let id: String
    let site: String
    let label: String?
    let hint: String?
    let note: String?
    let tags: [String]?
    let status: AccountStatus
    let cookies: Int
    let domains: [String]
    let expires: String?
    let sessionOnly: Bool
    let localStorage: [String]
    let createdAt: String
    let updatedAt: String
    let lastUsedAt: String?
}

/// Result of an injection (`use`/`switch`/`replay` `--json`).
struct ApplyResult: Codable, Equatable {
    let id: String
    let session: String
    let openedUrl: String?
    let cookies: Int
    let localstorage: Int
    let ok: Bool
}

/// One account's outcome from `run --json`.
struct RunResult: Codable, Equatable, Identifiable {
    let id: String
    let session: String
    let openedUrl: String?
    let ok: Bool
    let error: String?
}

/// Where a session is applied.
enum InjectTarget: Equatable {
    case session(String)
    case isolated

    var cliValue: String {
        switch self {
        case let .session(name): return "session:\(name)"
        case .isolated: return "isolated"
        }
    }
}

/// A local Chrome profile (`chrome-use profiles --json`).
struct ChromeProfile: Codable, Equatable, Hashable, Identifiable {
    let directory: String
    let name: String
    var id: String { directory }
}

/// The Chrome the extension relay is attached to (`chrome-use browsers --json`).
struct ConnectedBrowser: Codable, Equatable {
    let id: String
    let email: String?
    let `default`: Bool?
}

/// A surfaced error from the CLI's `{"error": "..."}` envelope.
struct CLIError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

enum ISO8601 {
    private static let fractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private static let plain = ISO8601DateFormatter()

    static func parse(_ s: String) -> Date? { fractional.date(from: s) ?? plain.date(from: s) }
}

extension Date {
    /// "3 days ago" / "in 2 hours".
    var relative: String {
        if abs(timeIntervalSinceNow) < 60 { return "just now" }
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .full
        return f.localizedString(for: self, relativeTo: Date())
    }
}

extension String {
    var nilIfBlank: String? {
        let t = trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty ? nil : t
    }

    /// "https://dash.cloudflare.com/login?x" → "dash.cloudflare.com". Mirrors the CLI's
    /// `normalize_site_filter`, so a pasted URL works anywhere a domain is expected.
    var normalizedHost: String {
        var s = trimmingCharacters(in: .whitespacesAndNewlines)
        if let r = s.range(of: "://") { s = String(s[r.upperBound...]) }
        let host = s.split(whereSeparator: { "/?:#".contains($0) }).first.map(String.init) ?? s
        return host.lowercased()
    }

    /// Strip a leading "www." for nicer default site names.
    var withoutWWW: String { hasPrefix("www.") ? String(dropFirst(4)) : self }
}
