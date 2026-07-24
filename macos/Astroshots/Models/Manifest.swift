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

    enum CodingKeys: String, CodingKey {
        case id, file, slug, title, description, url, viewport
        case capturedAt = "captured_at"
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
        capturedAt: Date?
    ) {
        let (seq, fileSlug) = ShotPath.sequenceAndSlug(fileName: fileName)
        let entry = manifest?.shots?.first { shot in
            if let file = shot.file, file == fileName { return true }
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

        return (seq ?? entry?.id, slug, title, description, url, runID, status, capturedAt)
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
