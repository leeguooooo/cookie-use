import Foundation

/// The single boundary between the GUI and the `cookie-use` / `chrome-use` CLIs.
///
/// Every command is driven with `--json`; the GUI never parses human text. The
/// GUI is non-interactive, so injection commands pass `--no-confirm` and set
/// `COOKIE_USE_YES=1` — the GUI owns its own Touch ID confirmation UX.
actor CLIBridge {
    static let shared = CLIBridge()

    enum BridgeError: LocalizedError {
        case binaryNotFound(String)
        var errorDescription: String? {
            switch self {
            case let .binaryNotFound(name): return "Could not find “\(name)” on this Mac. Install it first."
            }
        }
    }

    private let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    private var cachedCookieUse: URL?
    private var cachedChromeUse: URL?

    // MARK: Binary resolution

    private static let searchDirs = [
        "\(NSHomeDirectory())/.local/bin",
        "\(NSHomeDirectory())/.cargo/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
    ]

    private func resolve(_ name: String) throws -> URL {
        if name == "cookie-use", let c = cachedCookieUse { return c }
        if name == "chrome-use", let c = cachedChromeUse { return c }
        for dir in Self.searchDirs {
            let url = URL(fileURLWithPath: dir).appendingPathComponent(name)
            if FileManager.default.isExecutableFile(atPath: url.path) {
                if name == "cookie-use" { cachedCookieUse = url } else { cachedChromeUse = url }
                return url
            }
        }
        throw BridgeError.binaryNotFound(name)
    }

    // MARK: Process plumbing

    private struct Output { let status: Int32; let stdout: Data; let stderr: Data }

    private func runRaw(_ binary: String, _ args: [String], extraEnv: [String: String] = [:]) async throws -> Output {
        let url = try resolve(binary)
        var env = ProcessInfo.processInfo.environment
        for (k, v) in extraEnv { env[k] = v }

        let process = Process()
        process.executableURL = url
        process.arguments = args
        process.environment = env
        let outPipe = Pipe()
        let errPipe = Pipe()
        process.standardOutput = outPipe
        process.standardError = errPipe

        try process.run()
        // Drain both pipes off the actor before waiting, so large output can't deadlock.
        async let out = Self.readToEnd(outPipe.fileHandleForReading)
        async let err = Self.readToEnd(errPipe.fileHandleForReading)
        let (od, ed) = await (out, err)
        process.waitUntilExit()
        return Output(status: process.terminationStatus, stdout: od, stderr: ed)
    }

    private static func readToEnd(_ handle: FileHandle) async -> Data {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let data = handle.readDataToEndOfFile()
                continuation.resume(returning: data)
            }
        }
    }

    /// Run cookie-use and decode its JSON, mapping the `{"error":...}` envelope to a thrown error.
    private func json<T: Decodable>(_ args: [String], as type: T.Type, inject: Bool = false) async throws -> T {
        let env = inject ? ["COOKIE_USE_YES": "1"] : [:]
        let out = try await runRaw("cookie-use", args + ["--json"], extraEnv: env)
        guard out.status == 0 else {
            throw decodeError(out.stderr)
        }
        return try decoder.decode(T.self, from: out.stdout)
    }

    /// Run a cookie-use command that we don't need a typed result from.
    @discardableResult
    private func voidJSON(_ args: [String], inject: Bool = false) async throws -> Data {
        let env = inject ? ["COOKIE_USE_YES": "1"] : [:]
        let out = try await runRaw("cookie-use", args + ["--json"], extraEnv: env)
        guard out.status == 0 else { throw decodeError(out.stderr) }
        return out.stdout
    }

    private func decodeError(_ stderr: Data) -> Error {
        if let env = try? decoder.decode([String: String].self, from: stderr), let msg = env["error"] {
            return CLIError(message: msg)
        }
        let text = String(data: stderr, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
        return CLIError(message: text?.isEmpty == false ? text! : "cookie-use failed")
    }

    // MARK: Reads

    private struct ListResponse: Decodable { let accounts: [AccountSummary] }
    private struct CheckResponse: Decodable { let status: AccountStatus }

    func listAccounts(filter: String? = nil) async throws -> [AccountSummary] {
        var args = ["list"]
        if let filter, !filter.isEmpty { args.append(filter) }
        return try await json(args, as: ListResponse.self).accounts
    }

    func show(id: String) async throws -> Account {
        try await json(["show", id], as: Account.self)
    }

    func check(id: String) async throws -> AccountStatus {
        try await json(["check", id], as: CheckResponse.self).status
    }

    /// Active chrome-use sessions (to show "is Chrome connected?").
    func chromeSessions() async throws -> ChromeSessions {
        let out = try await runRaw("chrome-use", ["session", "list", "--json"])
        guard out.status == 0 else { throw decodeError(out.stderr) }
        return try decoder.decode(ChromeSessions.self, from: out.stdout)
    }

    /// The chrome-use session name currently in focus (the live Chrome the user
    /// is driving), used as the default injection target. `nil` if none.
    func currentChromeSession() async -> String? {
        guard let out = try? await runRaw("chrome-use", ["session"]), out.status == 0,
              let name = String(data: out.stdout, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
              !name.isEmpty
        else { return nil }
        return name
    }

    // MARK: Capture / import

    private struct AddResponse: Decodable { let id: String; let site: String; let localstorageCaptured: Int }

    func add(fromProfile: String, site: String, id: String? = nil, label: String? = nil,
             hint: String? = nil, withLocalStorage: Bool = false) async throws -> String {
        var args = ["add", "--from-profile", fromProfile, "--site", site]
        if let id { args += ["--id", id] }
        if let label { args += ["--label", label] }
        if let hint { args += ["--hint", hint] }
        if withLocalStorage { args.append("--with-localstorage") }
        return try await json(args, as: AddResponse.self).id
    }

    // MARK: Injection (Touch ID handled by the GUI; CLI runs unattended)

    func apply(_ verb: String, id: String, target: InjectTarget = .session("default"),
               rewriteDomain: String? = nil, openURL: String? = nil,
               injectLocalStorage: Bool = true) async throws -> ApplyResult {
        var args = [verb, id, "--target", target.cliValue, "--no-confirm"]
        if let rewriteDomain { args += ["--rewrite-domain", rewriteDomain] }
        if let openURL { args += ["--open-url", openURL] }
        if !injectLocalStorage { args.append("--no-localstorage") }
        return try await json(args, as: ApplyResult.self, inject: true)
    }

    func use(id: String, target: InjectTarget = .session("default")) async throws -> ApplyResult {
        try await apply("use", id: id, target: target)
    }

    func switchTo(id: String, target: InjectTarget = .session("default")) async throws -> ApplyResult {
        try await apply("switch", id: id, target: target)
    }

    private struct RunResponse: Decodable { let results: [RunResult] }

    func run(id: String? = nil, site: String? = nil, all: Bool = false) async throws -> [RunResult] {
        var args = ["run"]
        if all {
            args.append("--all")
            if let site { args += ["--site", site] }
        } else if let id {
            args.append(id)
        } else if let site {
            args += ["--site", site]
        }
        return try await json(args, as: RunResponse.self, inject: true).results
    }

    // MARK: Lifecycle

    func rename(id: String, to newID: String) async throws { try await voidJSON(["rename", id, newID]) }
    func remove(id: String) async throws { try await voidJSON(["rm", id]) }

    private struct WipeResponse: Decodable { let removed: Int }
    func wipe() async throws -> Int { try await json(["wipe", "--yes"], as: WipeResponse.self).removed }

    // MARK: Share / redeem

    private struct ShareResponse: Decodable { let path: String; let redeemCmd: String }
    private struct RedeemResponse: Decodable { let id: String; let site: String; let overwroteExisting: Bool }

    func share(id: String, out: String? = nil, password: String) async throws -> URL {
        var args = ["share", id, "--password", password]
        if let out { args += ["--out", out] }
        let resp = try await json(args, as: ShareResponse.self)
        return URL(fileURLWithPath: resp.path)
    }

    func redeem(bundle: String, password: String, newID: String? = nil) async throws -> (id: String, overwrote: Bool) {
        var args = ["redeem", bundle, "--password", password]
        if let newID { args += ["--id", newID] }
        let resp = try await json(args, as: RedeemResponse.self)
        return (resp.id, resp.overwroteExisting)
    }
}
