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
struct AccountSummary: Codable, Equatable, Identifiable {
    let id: String
    let site: String
    let label: String?
    let accountHint: String?
    let status: AccountStatus
    let cookies: Int
    let lastUsedAt: String?

    /// The base site (first comma-separated host) used to group accounts.
    var primarySite: String {
        site.split(separator: ",").first.map(String.init) ?? site
    }

    /// Best human label for a row: explicit label, else the id's trailing segment.
    var displayName: String {
        if let label, !label.isEmpty { return label }
        return id.split(separator: "/").last.map(String.init) ?? id
    }
}

/// Full detail from `cookie-use show <id> --json`.
struct Account: Codable, Equatable, Identifiable {
    let id: String
    let site: String
    let label: String?
    let hint: String?
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

/// chrome-use `session list --json` payload (drives "is Chrome connected?").
struct ChromeSessions: Codable, Equatable {
    struct Data: Codable, Equatable {
        let sessions: [String]
        let relay: Bool?
    }
    let success: Bool
    let data: Data?
}

/// A surfaced error from the CLI's `{"error": "..."}` envelope.
struct CLIError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}
