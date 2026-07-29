import Foundation

/// Persists locations of `.astroshot` directories discovered under watched roots.
///
/// A valid index is the startup source of truth: launch re-scans only these
/// paths, while FSEvents adds newly created `.astroshot` directories. A full
/// recursive walk is reserved for cache misses and explicit rescans.
struct ShotIndexCacheDocument: Codable, Equatable, Sendable {
    var version: Int
    /// Absolute watch-root paths this index was built for (sorted).
    var roots: [String]
    /// Absolute paths to `.astroshot` directories.
    var astroshotDirs: [String]
    /// Image paths newest-arrival-first. Unlike file timestamps, this remains
    /// stable when existing captures are rewritten or metadata changes.
    var arrivalOrder: [String]
    /// Last filesystem event incorporated into this index. Replaying from this
    /// cursor discovers captures written while Astroshots was not running.
    var lastEventID: UInt64?
    var updatedAt: Date

    static let currentVersion = 1

    init(
        version: Int,
        roots: [String],
        astroshotDirs: [String],
        arrivalOrder: [String] = [],
        lastEventID: UInt64? = nil,
        updatedAt: Date
    ) {
        self.version = version
        self.roots = roots
        self.astroshotDirs = astroshotDirs
        self.arrivalOrder = arrivalOrder
        self.lastEventID = lastEventID
        self.updatedAt = updatedAt
    }

    private enum CodingKeys: String, CodingKey {
        case version, roots, astroshotDirs, arrivalOrder, lastEventID, updatedAt
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        version = try container.decode(Int.self, forKey: .version)
        roots = try container.decode([String].self, forKey: .roots)
        astroshotDirs = try container.decode([String].self, forKey: .astroshotDirs)
        arrivalOrder = try container.decodeIfPresent(
            [String].self,
            forKey: .arrivalOrder
        ) ?? []
        lastEventID = try container.decodeIfPresent(UInt64.self, forKey: .lastEventID)
        updatedAt = try container.decode(Date.self, forKey: .updatedAt)
    }
}

enum ShotIndexCache {
    private static let fileName = "shot-index.json"
    private static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        e.dateEncodingStrategy = .iso8601
        return e
    }()
    private static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }()

    /// Application Support directory for this app (`…/ai.archastro.Astroshots/`).
    static func supportDirectory(fileManager: FileManager = .default) -> URL {
        let base = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fileManager.temporaryDirectory
        let dir = base.appendingPathComponent("ai.archastro.Astroshots", isDirectory: true)
        try? fileManager.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    static func cacheFileURL(fileManager: FileManager = .default) -> URL {
        supportDirectory(fileManager: fileManager).appendingPathComponent(fileName)
    }

    /// Load a cache only when it matches `roots` and the schema version.
    static func load(
        for roots: [URL],
        cacheFileURL: URL? = nil,
        fileManager: FileManager = .default
    ) -> ShotIndexCacheDocument? {
        let url = cacheFileURL ?? self.cacheFileURL(fileManager: fileManager)
        guard let data = try? Data(contentsOf: url),
              let doc = try? decoder.decode(ShotIndexCacheDocument.self, from: data)
        else {
            return nil
        }
        guard doc.version == ShotIndexCacheDocument.currentVersion else { return nil }
        let normalized = normalizeRoots(roots)
        guard doc.roots == normalized else { return nil }
        return doc
    }

    static func save(
        roots: [URL],
        astroshotDirs: [String],
        arrivalOrder: [String]? = nil,
        lastEventID: UInt64? = nil,
        cacheFileURL: URL? = nil,
        fileManager: FileManager = .default
    ) {
        let uniqueDirs = Array(Set(astroshotDirs)).sorted()
        let existing = load(
            for: roots,
            cacheFileURL: cacheFileURL,
            fileManager: fileManager
        )
        let resolvedArrivalOrder = uniquePaths(
            arrivalOrder ?? existing?.arrivalOrder ?? []
        )
        let doc = ShotIndexCacheDocument(
            version: ShotIndexCacheDocument.currentVersion,
            roots: normalizeRoots(roots),
            astroshotDirs: uniqueDirs,
            arrivalOrder: resolvedArrivalOrder,
            lastEventID: lastEventID ?? existing?.lastEventID,
            updatedAt: Date()
        )
        let url = cacheFileURL ?? self.cacheFileURL(fileManager: fileManager)
        try? fileManager.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        guard let data = try? encoder.encode(doc) else { return }
        try? data.write(to: url, options: [.atomic])
    }

    /// Drop the on-disk index (e.g. after a manual “forget” if added later).
    static func clear(
        cacheFileURL: URL? = nil,
        fileManager: FileManager = .default
    ) {
        let url = cacheFileURL ?? self.cacheFileURL(fileManager: fileManager)
        try? fileManager.removeItem(at: url)
    }

    /// Parent `.astroshot` directory for an image path, if any.
    static func astroshotDir(forImagePath path: String) -> String? {
        guard let parts = ShotPath.parse(imagePath: path) else { return nil }
        return URL(fileURLWithPath: parts.worktreePath, isDirectory: true)
            .appendingPathComponent(ShotPath.astroshotDirName, isDirectory: true)
            .path
    }

    /// Reconcile the durable order with a filesystem snapshot. Paths first
    /// discovered in this snapshot are arrivals now, ordered by their capture
    /// timestamp; known paths retain their exact prior relative order.
    static func reconciledArrivalOrder(
        existing: [String],
        shots: [Shot]
    ) -> [String] {
        let livePaths = Set(shots.map(\.path))
        let retained = uniquePaths(existing).filter(livePaths.contains)
        let retainedPaths = Set(retained)
        let newPaths = shots
            .filter { !retainedPaths.contains($0.path) }
            .sorted { $0.capturedAt > $1.capturedAt }
            .map(\.path)
        return uniquePaths(newPaths + retained)
    }

    static func orderedShots(
        _ shots: [Shot],
        arrivalOrder: [String]
    ) -> [Shot] {
        let rank = Dictionary(
            uniqueKeysWithValues: uniquePaths(arrivalOrder)
                .enumerated()
                .map { ($0.element, $0.offset) }
        )
        return shots.sorted { lhs, rhs in
            switch (rank[lhs.path], rank[rhs.path]) {
            case let (left?, right?):
                return left < right
            case (_?, nil):
                return true
            case (nil, _?):
                return false
            case (nil, nil):
                return lhs.capturedAt > rhs.capturedAt
            }
        }
    }

    private static func normalizeRoots(_ roots: [URL]) -> [String] {
        roots
            .map { $0.standardizedFileURL.path }
            .sorted()
    }

    private static func uniquePaths(_ paths: [String]) -> [String] {
        var seen = Set<String>()
        return paths.filter { seen.insert($0).inserted }
    }
}
