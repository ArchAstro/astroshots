import Foundation

/// Persists locations of `.astroshot` directories discovered under watched roots.
///
/// Warm start re-scans only these paths (cheap) so the tray fills before a full
/// recursive walk of a large monorepo finishes. The full scan still runs and
/// rewrites the cache with any newly found trees.
struct ShotIndexCacheDocument: Codable, Equatable, Sendable {
    var version: Int
    /// Absolute watch-root paths this index was built for (sorted).
    var roots: [String]
    /// Absolute paths to `.astroshot` directories.
    var astroshotDirs: [String]
    var updatedAt: Date

    static let currentVersion = 1
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
        fileManager: FileManager = .default
    ) -> ShotIndexCacheDocument? {
        let url = cacheFileURL(fileManager: fileManager)
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
        fileManager: FileManager = .default
    ) {
        let uniqueDirs = Array(Set(astroshotDirs)).sorted()
        let doc = ShotIndexCacheDocument(
            version: ShotIndexCacheDocument.currentVersion,
            roots: normalizeRoots(roots),
            astroshotDirs: uniqueDirs,
            updatedAt: Date()
        )
        let url = cacheFileURL(fileManager: fileManager)
        guard let data = try? encoder.encode(doc) else { return }
        try? data.write(to: url, options: [.atomic])
    }

    /// Drop the on-disk index (e.g. after a manual “forget” if added later).
    static func clear(fileManager: FileManager = .default) {
        let url = cacheFileURL(fileManager: fileManager)
        try? fileManager.removeItem(at: url)
    }

    /// Parent `.astroshot` directory for an image path, if any.
    static func astroshotDir(forImagePath path: String) -> String? {
        guard let parts = ShotPath.parse(imagePath: path) else { return nil }
        return URL(fileURLWithPath: parts.worktreePath, isDirectory: true)
            .appendingPathComponent(ShotPath.astroshotDirName, isDirectory: true)
            .path
    }

    private static func normalizeRoots(_ roots: [URL]) -> [String] {
        roots
            .map { $0.standardizedFileURL.path }
            .sorted()
    }
}
