import AppKit
import SQLite3

/// Site icons read from Chrome's *local* favicon cache — never fetched from a
/// favicon service, which would tell a third party which sites you keep
/// accounts for. Falls back to a monogram when Chrome has no icon.
@MainActor
final class Favicons: ObservableObject {
    static let shared = Favicons()

    @Published private(set) var icons: [String: NSImage] = [:]
    private var attempted: Set<String> = []

    func icon(for host: String) -> NSImage? {
        let key = Self.key(host)
        if let img = icons[key] { return img }
        if !attempted.contains(key) {
            attempted.insert(key)
            Task.detached(priority: .utility) {
                let data = Self.lookup(key)
                await MainActor.run {
                    if let data, let img = NSImage(data: data) { self.icons[key] = img }
                }
            }
        }
        return nil
    }

    /// Registrable-ish base: "dash.cloudflare.com" → "cloudflare.com" so every
    /// subdomain shares the site's icon.
    nonisolated static func key(_ host: String) -> String {
        let parts = host.normalizedHost.split(separator: ".")
        guard parts.count > 2 else { return parts.joined(separator: ".") }
        let twoLevelTLDs: Set<String> = ["co.uk", "com.cn", "com.au", "co.jp", "com.hk", "com.tw", "co.kr"]
        let lastTwo = parts.suffix(2).joined(separator: ".")
        return (twoLevelTLDs.contains(lastTwo) ? parts.suffix(3) : parts.suffix(2)).joined(separator: ".")
    }

    nonisolated private static func chromeDirs() -> [URL] {
        let root = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Google/Chrome")
        let names = (try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? []
        let profiles = names.filter { $0 == "Default" || $0.hasPrefix("Profile ") }.sorted { a, _ in a == "Default" }
        return profiles.map { root.appendingPathComponent($0).appendingPathComponent("Favicons") }
    }

    /// Opening Chrome's favicon DBs costs ~2s each, so found icons are cached
    /// on disk (refreshed weekly) and later launches are instant.
    nonisolated private static let cacheDir: URL = {
        let dir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("com.cookieuse.app/favicons", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }()

    nonisolated private static func lookup(_ base: String) -> Data? {
        let cached = cacheDir.appendingPathComponent(base)
        if let attrs = try? FileManager.default.attributesOfItem(atPath: cached.path),
           let modified = attrs[.modificationDate] as? Date, modified.timeIntervalSinceNow > -7 * 86_400,
           let data = try? Data(contentsOf: cached) {
            return data
        }
        for db in chromeDirs() where FileManager.default.fileExists(atPath: db.path) {
            if let data = query(db: db, base: base) {
                try? data.write(to: cached, options: .atomic)
                return data
            }
        }
        return nil
    }

    /// Chrome holds the DB open; `immutable=1` reads it without taking a lock.
    nonisolated private static func query(db: URL, base: String) -> Data? {
        var handle: OpaquePointer?
        let uri = "file:\(db.path.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? db.path)?immutable=1"
        guard sqlite3_open_v2(uri, &handle, SQLITE_OPEN_READONLY | SQLITE_OPEN_URI, nil) == SQLITE_OK else {
            sqlite3_close(handle)
            return nil
        }
        defer { sqlite3_close(handle) }
        let sql = """
        SELECT b.image_data FROM icon_mapping m
        JOIN favicon_bitmaps b ON b.icon_id = m.icon_id
        JOIN favicons f ON f.id = m.icon_id
        WHERE m.page_url LIKE ?1 OR m.page_url LIKE ?2
        -- Prefer an icon served by the site itself: login pages that bounced
        -- through an OAuth provider get mapped to *its* icon (claude.ai → Google).
        ORDER BY (f.url LIKE ?3 OR f.url LIKE ?4) DESC,
                 (m.page_url IN ('https://' || ?5 || '/', 'https://www.' || ?5 || '/')) DESC,
                 length(m.page_url) ASC,
                 (b.width BETWEEN 32 AND 64) DESC, b.width DESC
        LIMIT 1
        """
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(handle, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        sqlite3_bind_text(stmt, 1, "https://\(base)/%", -1, transient)
        sqlite3_bind_text(stmt, 2, "https://%.\(base)/%", -1, transient)
        sqlite3_bind_text(stmt, 3, "%://\(base)/%", -1, transient)
        sqlite3_bind_text(stmt, 4, "%.\(base)/%", -1, transient)
        sqlite3_bind_text(stmt, 5, base, -1, transient)
        guard sqlite3_step(stmt) == SQLITE_ROW, let blob = sqlite3_column_blob(stmt, 0) else { return nil }
        return Data(bytes: blob, count: Int(sqlite3_column_bytes(stmt, 0)))
    }
}
