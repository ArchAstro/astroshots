import Foundation

/// On-disk `manifest.json` under `.astroshot/<feature>/`.
struct FeatureManifest: Codable, Sendable, Equatable {
    var version: Int?
    var feature: String?
    var runID: String?
    var status: String?
    var description: String?
    var shots: [ManifestShot]?

    enum CodingKeys: String, CodingKey {
        case version, feature, status, description, shots
        case runID = "run_id"
    }
}

struct ManifestShot: Codable, Sendable, Equatable {
    var id: String?
    var file: String?
    var slug: String?
    var title: String?
    var description: String?
    var capturedAt: String?
    var url: String?
    var viewport: String?
    /// `"movie"` when this entry is a journey movie (poster + video).
    var kind: String?
    /// Basename of the video next to the poster (e.g. `0001-journey.webm`).
    var video: String?
    /// Optional explicit poster basename (defaults to `file`).
    var poster: String?
    /// Movie duration in milliseconds when known.
    var durationMs: Int?
    /// Capture source (`browser`, `pty`, `desktop.window`, `frames`, …).
    var source: String?
    /// Optional chapter markers (`slug` + `t_ms` / `tMs`).
    var chapters: [ManifestChapter]?

    enum CodingKeys: String, CodingKey {
        case id, file, slug, title, description, url, viewport, kind, video, poster, source, chapters
        case capturedAt = "captured_at"
        case durationMs = "duration_ms"
    }
}

struct ManifestChapter: Codable, Sendable, Equatable, Hashable {
    var slug: String?
    var tMs: Int?
    var title: String?

    enum CodingKeys: String, CodingKey {
        case slug, title
        case tMs = "t_ms"
        // Also accept camelCase from some harness writers.
        case tMsCamel = "tMs"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        slug = try c.decodeIfPresent(String.self, forKey: .slug)
        title = try c.decodeIfPresent(String.self, forKey: .title)
        if let ms = try c.decodeIfPresent(Int.self, forKey: .tMs) {
            tMs = ms
        } else {
            tMs = try c.decodeIfPresent(Int.self, forKey: .tMsCamel)
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(slug, forKey: .slug)
        try c.encodeIfPresent(title, forKey: .title)
        try c.encodeIfPresent(tMs, forKey: .tMs)
    }
}

enum ManifestParser {
    static func load(from directory: URL) -> FeatureManifest? {
        let url = directory.appendingPathComponent("manifest.json")
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(FeatureManifest.self, from: data)
    }

    static func shotMetadata(
        fileName: String,
        manifest: FeatureManifest?
    ) -> (
        sequence: String?,
        slug: String,
        title: String,
        description: String,
        url: String?,
        runID: String?,
        status: FeatureStatus?,
        capturedAt: Date?,
        kind: String?,
        video: String?,
        durationMs: Int?,
        source: String?,
        chapters: [ManifestChapter]
    ) {
        let (seq, fileSlug) = ShotPath.sequenceAndSlug(fileName: fileName)
        let entry = manifest?.shots?.first { shot in
            if let file = shot.file, file == fileName { return true }
            if let poster = shot.poster, poster == fileName { return true }
            if let id = shot.id, id == seq { return true }
            if let slug = shot.slug, slug == fileSlug { return true }
            return false
        }

        let slug = entry?.slug ?? fileSlug
        let title = entry?.title ?? humanize(slug)
        let description = entry?.description ?? ""
        let url = entry?.url
        let runID = manifest?.runID
        let status = FeatureStatus(raw: manifest?.status)
        let capturedAt = entry?.capturedAt.flatMap(parseISO8601)
        let kind = entry?.kind
        let video = entry?.video
        let durationMs = entry?.durationMs
        let source = entry?.source
        let chapters = entry?.chapters ?? []

        return (
            seq ?? entry?.id,
            slug,
            title,
            description,
            url,
            runID,
            status,
            capturedAt,
            kind,
            video,
            durationMs,
            source,
            chapters
        )
    }

    private static func humanize(_ slug: String) -> String {
        slug
            .replacingOccurrences(of: "-", with: " ")
            .replacingOccurrences(of: "_", with: " ")
            .split(separator: " ")
            .map { part in
                guard let first = part.first else { return String(part) }
                return String(first).uppercased() + part.dropFirst()
            }
            .joined(separator: " ")
    }

    private static func parseISO8601(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) { return date }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: value)
    }
}
