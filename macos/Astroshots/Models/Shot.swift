import Foundation

/// One screenshot frame discovered under `.astroshot/<feature>/`.
struct Shot: Identifiable, Hashable, Sendable {
    /// Stable identity: absolute file path.
    var id: String { path }

    /// Absolute path to the image file.
    let path: String
    /// Parent of `.astroshot` (worktree / project root basename used for badges).
    let worktree: String
    /// Absolute path of the worktree root (parent of `.astroshot`).
    let worktreePath: String
    /// Feature directory name under `.astroshot/`.
    let feature: String
    /// Filename (e.g. `0004-configure.png`).
    let fileName: String
    /// Parsed sequence id when the name looks like `0004-slug.ext`.
    let sequence: String?
    /// Slug from filename or manifest.
    let slug: String
    /// Human title from manifest, else slug.
    let title: String
    /// Description from manifest, else empty.
    let description: String
    /// Optional URL / route from manifest.
    let url: String?
    /// Run id from parent manifest when present.
    let runID: String?
    /// Feature status from manifest when present.
    let status: FeatureStatus?
    /// File modification time (used for ordering + display).
    let capturedAt: Date
    /// App-owned human review state from the feature's `review.json` sidecar.
    var review: ReviewSnapshot?
    /// Manifest `kind` when present (`movie`, …).
    let kind: String?
    /// Video basename next to the poster when this frame is a movie.
    let videoFileName: String?
    /// Movie duration in milliseconds when known.
    let durationMs: Int?
    /// Capture source from the movie harness (`browser`, `desktop.window`, …).
    let movieSource: String?
    /// Chapter markers from the movie harness (slug + offset ms).
    let chapters: [ManifestChapter]

    init(
        path: String,
        worktree: String,
        worktreePath: String,
        feature: String,
        fileName: String,
        sequence: String?,
        slug: String,
        title: String,
        description: String,
        url: String?,
        runID: String?,
        status: FeatureStatus?,
        capturedAt: Date,
        review: ReviewSnapshot? = nil,
        kind: String? = nil,
        videoFileName: String? = nil,
        durationMs: Int? = nil,
        movieSource: String? = nil,
        chapters: [ManifestChapter] = []
    ) {
        self.path = path
        self.worktree = worktree
        self.worktreePath = worktreePath
        self.feature = feature
        self.fileName = fileName
        self.sequence = sequence
        self.slug = slug
        self.title = title
        self.description = description
        self.url = url
        self.runID = runID
        self.status = status
        self.capturedAt = capturedAt
        self.review = review
        self.kind = kind
        self.videoFileName = videoFileName
        self.durationMs = durationMs
        self.movieSource = movieSource
        self.chapters = chapters
    }

    var isFailure: Bool {
        status == .fail || slug.localizedCaseInsensitiveContains("fail")
            || title.localizedCaseInsensitiveContains("fail")
    }

    /// True when this stream row is a journey movie (poster + optional video).
    var isMovie: Bool {
        if let kind, kind.caseInsensitiveCompare("movie") == .orderedSame {
            return true
        }
        return videoFileName != nil
    }

    /// Absolute path to the sibling video file when it exists on disk.
    var videoPath: String? {
        guard let videoFileName, !videoFileName.isEmpty else { return nil }
        let dir = URL(fileURLWithPath: path).deletingLastPathComponent()
        let candidate = dir.appendingPathComponent(videoFileName)
        return FileManager.default.fileExists(atPath: candidate.path)
            ? candidate.path
            : nil
    }

    /// Short duration label for stream/detail chips (`2.8s`, `1:05`).
    var durationLabel: String? {
        guard let durationMs, durationMs > 0 else { return nil }
        let totalSeconds = Double(durationMs) / 1000
        if totalSeconds < 60 {
            let rounded = (totalSeconds * 10).rounded() / 10
            if rounded == rounded.rounded(.towardZero) {
                return "\(Int(rounded))s"
            }
            return String(format: "%.1fs", rounded)
        }
        let minutes = Int(totalSeconds) / 60
        let seconds = Int(totalSeconds) % 60
        return String(format: "%d:%02d", minutes, seconds)
    }

    /// Short badge label: last path segment, trimmed (`firstlanding-wt4` → `wt4` when possible).
    var worktreeShort: String {
        if let range = worktree.range(of: #"wt\d+"#, options: .regularExpression) {
            return String(worktree[range])
        }
        if worktree.count <= 8 { return worktree }
        return String(worktree.prefix(6))
    }
}

enum FeatureStatus: String, Sendable, Hashable {
    case running
    case pass
    case fail
    case idle

    init?(raw: String?) {
        guard let raw else { return nil }
        switch raw.lowercased() {
        case "running", "run", "in_progress", "in-progress": self = .running
        case "pass", "passed", "ok", "success": self = .pass
        case "fail", "failed", "error": self = .fail
        case "idle", "pending": self = .idle
        default: return nil
        }
    }
}

enum ShotPath {
    static let astroshotDirName = ".astroshot"
    static let imageExtensions: Set<String> = ["png", "jpg", "jpeg", "webp", "gif"]

    /// Parse `…/<worktree>/.astroshot/<feature>/<file>` into components.
    ///
    /// Paths under `.astroshot/friction-logs/` are intentionally rejected so
    /// friction-log screenshots never enter the one-off Shots stream.
    static func parse(imagePath: String) -> (worktreePath: String, worktree: String, feature: String, fileName: String)? {
        let url = URL(fileURLWithPath: imagePath)
        let fileName = url.lastPathComponent
        let ext = url.pathExtension.lowercased()
        guard imageExtensions.contains(ext) else { return nil }

        // Friction-log screenshots may nest under runs/; walk ancestors for
        // `.astroshot/friction-logs` and refuse the path as a normal shot.
        if FrictionLogPath.containsImage(path: imagePath) {
            return nil
        }

        let featureURL = url.deletingLastPathComponent()
        let feature = featureURL.lastPathComponent
        guard !feature.isEmpty, feature != astroshotDirName else { return nil }
        // Top-level reserved namespace for friction logs (not a feature).
        guard feature != FrictionLogPath.directoryName else { return nil }

        let astroshotURL = featureURL.deletingLastPathComponent()
        guard astroshotURL.lastPathComponent == astroshotDirName else { return nil }

        let worktreeURL = astroshotURL.deletingLastPathComponent()
        let worktree = worktreeURL.lastPathComponent
        guard !worktree.isEmpty else { return nil }

        return (worktreeURL.path, worktree, feature, fileName)
    }

    /// Split `0004-configure.png` → (`0004`, `configure`).
    static func sequenceAndSlug(fileName: String) -> (sequence: String?, slug: String) {
        let base = (fileName as NSString).deletingPathExtension
        if let dash = base.firstIndex(of: "-") {
            let head = String(base[..<dash])
            let tail = String(base[base.index(after: dash)...])
            if !head.isEmpty, head.allSatisfy(\.isNumber) {
                return (head, tail.isEmpty ? head : tail)
            }
        }
        return (nil, base)
    }
}
