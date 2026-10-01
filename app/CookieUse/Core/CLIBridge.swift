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
            case let .binaryNotFound(name): return "Could not find “\(name)” on this Mac."
            }
        }
    }

    private let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    private var resolved: [String: URL] = [:]

    // MARK: Binary resolution

    /// A Finder-launched app inherits a bare PATH (/usr/bin:/bin:…), so look
    /// where the installers actually put things.
    static let searchDirs = [
        "\(NSHomeDirectory())/.local/bin",
        "\(NSHomeDirectory())/.cargo/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
    ]

    func resolve(_ name: String) throws -> URL {
        if let url = resolved[name] { return url }
        for dir in Self.searchDirs {
            let url = URL(fileURLWithPath: dir).appendingPathComponent(name)
            if FileManager.default.isExecutableFile(atPath: url.path) {
                resolved[name] = url
                return url
            }
        }
        throw BridgeError.binaryNotFound(name)
    }

    /// Which of the two CLIs are missing (drives the onboarding empty state).
    func missingBinaries() -> [String] {
        resolved.removeAll()
        return ["cookie-use", "chrome-use"].filter { (try? resolve($0)) == nil }
    }

    // MARK: Process plumbing

    private struct Output { let status: Int32; let stdout: Data; let stderr: Data }

    private func runRaw(_ binary: String, _ args: [String], extraEnv: [String: String] = [:]) async throws -> Output {
        let url = try resolve(binary)
        var env = ProcessInfo.processInfo.environment
        // cookie-use shells out to chrome-use by name: hand it the exact binary and
        // a PATH that includes the user-level install dirs.
        let path = (Self.searchDirs + [env["PATH"] ?? ""]).joined(separator: ":")
        env["PATH"] = path
        if env["CHROME_USE_BIN"] == nil, let chrome = try? resolve("chrome-use") {
            env["CHROME_USE_BIN"] = chrome.path
        }
        // The daily "update available" notice is for terminals, not the GUI.
        env["COOKIE_USE_NO_UPDATE_CHECK"] = "1"
        for (k, v) in extraEnv { env[k] = v }

        let process = Process()
        process.executableURL = url
        process.arguments = args
        process.environment = env
        process.standardInput = FileHandle.nullDevice
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
                continuation.resume(returning: handle.readDataToEndOfFile())
            }
        }
    }

    /// Run cookie-use and decode its JSON, mapping the `{"error":...}` envelope to a thrown error.
    private func json<T: Decodable>(_ args: [String], as type: T.Type, inject: Bool = false) async throws -> T {
        let data = try await voidJSON(args, inject: inject)
        return try decoder.decode(T.self, from: data)
    }

    @discardableResult
    private func voidJSON(_ args: [String], inject: Bool = false) async throws -> Data {
        let env = inject ? ["COOKIE_USE_YES": "1"] : [:]
        let out = try await runRaw("cookie-use", args + ["--json"], extraEnv: env)
        guard out.status == 0 else { throw decodeError(out.stderr) }
        return out.stdout
    }

    private func decodeError(_ stderr: Data) -> Error {
        // stderr may carry notes before the final JSON envelope; take the last line that parses.
        let text = String(data: stderr, encoding: .utf8) ?? ""
        for line in text.split(separator: "\n").reversed() {
            if let d = line.data(using: .utf8),
               let env = try? decoder.decode([String: String].self, from: d), let msg = env["error"] {
                return CLIError(message: msg)
            }
        }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "error: ", with: "")
        return CLIError(message: trimmed.isEmpty ? "cookie-use failed" : trimmed)
    }

    // MARK: Reads

    private struct ListResponse: Decodable { let accounts: [AccountSummary] }
    private struct CheckResponse: Decodable { let status: AccountStatus }

    func listAccounts() async throws -> [AccountSummary] {
        try await json(["list"], as: ListResponse.self).accounts
    }

    func show(id: String) async throws -> Account {
        try await json(["show", id], as: Account.self)
    }

    @discardableResult
    func check(id: String) async throws -> AccountStatus {
        try await json(["check", id], as: CheckResponse.self).status
    }

    // MARK: chrome-use

    private struct Envelope<T: Decodable>: Decodable { let success: Bool?; let data: T? }
    private struct BrowsersData: Decodable { let browsers: [ConnectedBrowser] }

    /// The Chrome(s) the chrome-use extension relay is attached to. Empty = not connected.
    func connectedBrowsers() async -> [ConnectedBrowser] {
        guard let out = try? await runRaw("chrome-use", ["browsers", "--json"]), out.status == 0,
              let env = try? decoder.decode(Envelope<BrowsersData>.self, from: out.stdout)
        else { return [] }
        return env.data?.browsers ?? []
    }

    func chromeProfiles() async -> [ChromeProfile] {
        guard let out = try? await runRaw("chrome-use", ["profiles", "--json"]), out.status == 0,
              let env = try? decoder.decode(Envelope<[ChromeProfile]>.self, from: out.stdout)
        else { return [] }
        return env.data ?? []
    }

    private struct CookieCountOnly: Decodable {
        let data: [Ignored]?
        struct Ignored: Decodable {}
    }

    /// How many cookies a Chrome profile holds for `site` — a cheap "is it logged
    /// in there?" probe for the capture sheet. Values are decoded away, never kept.
    func cookieCount(profile: String, site: String) async -> Int {
        guard let out = try? await runRaw("chrome-use", ["cookies", "export", "--from", profile, "--domain", site, "--json"]),
              out.status == 0,
              let parsed = try? decoder.decode(CookieCountOnly.self, from: out.stdout)
        else { return 0 }
        return parsed.data?.count ?? 0
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

    private struct ImportResponse: Decodable { let id: String; let site: String }

    func importFile(_ path: String, site: String, id: String, label: String? = nil, hint: String? = nil) async throws -> String {
        var args = ["import", "--file", path, "--site", site, "--id", id]
        if let label { args += ["--label", label] }
        if let hint { args += ["--hint", hint] }
        return try await json(args, as: ImportResponse.self).id
    }

    // MARK: Injection (Touch ID handled by the GUI; CLI runs unattended)

    /// `clean: true` → `switch` (signs the site's previous account out first —
    /// scoped to that site, never the whole browser); `false` → `use` (layer on top).
    func open(id: String, target: InjectTarget, clean: Bool) async throws -> ApplyResult {
        try await json([clean ? "switch" : "use", id, "--target", target.cliValue, "--no-confirm"],
                       as: ApplyResult.self, inject: true)
    }

    /// Cross-origin QA: rewrite the cookie domain + open a dev origin in one shot.
    func replay(id: String, to devOrigin: String, target: InjectTarget) async throws -> ApplyResult {
        try await json(["replay", id, "--to", devOrigin, "--target", target.cliValue, "--no-confirm"],
                       as: ApplyResult.self, inject: true)
    }

    private struct RunResponse: Decodable { let results: [RunResult] }

    /// Open each id in its own side-by-side isolated window.
    func run(ids: [String]) async throws -> [RunResult] {
        var results: [RunResult] = []
        for id in ids {
            results += try await json(["run", id], as: RunResponse.self, inject: true).results
        }
        return results
    }

    // MARK: Lifecycle

    func rename(id: String, to newID: String) async throws { try await voidJSON(["rename", id, newID]) }
    func remove(id: String) async throws { try await voidJSON(["rm", id]) }

    /// `nil` leaves a field alone; "" clears it.
    func edit(id: String, label: String? = nil, hint: String? = nil, note: String? = nil, tags: [String]? = nil) async throws {
        var args = ["edit", id]
        if let label { args += ["--label", label] }
        if let hint { args += ["--hint", hint] }
        if let note { args += ["--note", note] }
        if let tags { args += ["--tags", tags.joined(separator: ",")] }
        try await voidJSON(args)
    }

    // MARK: Share / redeem

    private struct ShareResponse: Decodable { let path: String; let redeemCmd: String }
    private struct RedeemResponse: Decodable { let id: String; let site: String; let overwroteExisting: Bool }

    func share(id: String, out: String, password: String) async throws -> URL {
        let resp = try await json(["share", id, "--password", password, "--out", out], as: ShareResponse.self)
        return URL(fileURLWithPath: resp.path)
    }

    func redeem(bundle: String, password: String, newID: String? = nil) async throws -> (id: String, overwrote: Bool) {
        var args = ["redeem", bundle, "--password", password]
        if let newID { args += ["--id", newID] }
        let resp = try await json(args, as: RedeemResponse.self)
        return (resp.id, resp.overwroteExisting)
    }
}
