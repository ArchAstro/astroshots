import Foundation

/// Reserved feature directory under `.astroshot/` for agentic UX scenario runs.
/// Images and JSONL under this tree never appear in the Shots stream.
enum FrictionLogPath {
    static let directoryName = "friction-logs"
    static let promptFileName = "prompt.md"
    static let metaFileName = "meta.json"
    static let logFileName = "log.jsonl"
    static let runsDirectoryName = "runs"

    /// `…/<worktree>/.astroshot/friction-logs`
    static func root(inAstroshot astroshot: URL) -> URL {
        astroshot.appendingPathComponent(directoryName, isDirectory: true)
    }

    /// True when an image path lives under `.astroshot/friction-logs/…`.
    static func containsImage(path: String) -> Bool {
        let markers = [
            "/\(ShotPath.astroshotDirName)/\(directoryName)/",
            "/\(ShotPath.astroshotDirName)/\(directoryName)",
        ]
        let normalized = path.replacingOccurrences(of: "\\", with: "/")
        return markers.contains { normalized.contains($0) }
    }
}

enum FrictionLogStatus: String, Sendable, Hashable, Codable {
    case draft
    case ready
    case running
    case complete
    case failed

    init?(raw: String?) {
        guard let raw else { return nil }
        switch raw.lowercased() {
        case "draft": self = .draft
        case "ready", "idle": self = .ready
        case "running", "run", "in_progress", "in-progress": self = .running
        case "complete", "completed", "pass", "passed", "done", "success":
            self = .complete
        case "failed", "fail", "error": self = .failed
        default: return nil
        }
    }

    var label: String {
        switch self {
        case .draft: return "Draft"
        case .ready: return "Ready"
        case .running: return "Running"
        case .complete: return "Complete"
        case .failed: return "Failed"
        }
    }
}

/// One authored friction-log scenario under `.astroshot/friction-logs/<slug>/`.
struct FrictionLog: Identifiable, Hashable, Sendable {
    /// Stable identity: worktree path + slug.
    var id: String { "\(worktreePath)::\(slug)" }

    let worktree: String
    let worktreePath: String
    let slug: String
    let title: String
    let description: String
    let promptPath: String?
    let promptMarkdown: String?
    let status: FrictionLogStatus?
    let runs: [FrictionLogRun]
    /// Newest activity across prompt, meta, and runs.
    let updatedAt: Date

    var worktreeShort: String {
        if let range = worktree.range(of: #"wt\d+"#, options: .regularExpression) {
            return String(worktree[range])
        }
        if worktree.count <= 8 { return worktree }
        return String(worktree.prefix(6))
    }

    var latestRun: FrictionLogRun? { runs.first }

    var stepCount: Int { latestRun?.steps.count ?? 0 }

    var improveCount: Int {
        latestRun?.steps.reduce(0) { $0 + $1.improve.count } ?? 0
    }

    var goodCount: Int {
        latestRun?.steps.reduce(0) { $0 + $1.good.count } ?? 0
    }
}

/// One execution of a friction log (JSONL + screenshots).
struct FrictionLogRun: Identifiable, Hashable, Sendable {
    var id: String { runID }

    let runID: String
    let directoryPath: String
    let steps: [FrictionLogStep]
    let capturedAt: Date
    let status: FrictionLogStatus?
}

/// One JSONL line: what the agent did, screenshots, and UX notes.
struct FrictionLogStep: Identifiable, Hashable, Sendable {
    var id: String { "\(step)-\(stepID)" }

    let step: Int
    let stepID: String
    let title: String
    let description: String
    /// Absolute paths to screenshot files for this step.
    let screenshotPaths: [String]
    let good: [String]
    let improve: [String]
    let url: String?
    let capturedAt: Date?

    var primaryScreenshotPath: String? { screenshotPaths.first }
}

// MARK: - Loading

enum FrictionLogLoader {
    private struct MetaDocument: Codable {
        var version: Int?
        var slug: String?
        var title: String?
        var description: String?
        var status: String?
        var updated_at: String?
    }

    private struct StepDocument: Codable {
        var step: Int?
        var id: String?
        var title: String?
        var description: String?
        var screenshots: [String]?
        var screenshot: String?
        var good: [String]?
        var improve: [String]?
        /// Accept alternate keys agents may emit.
        var looks_good: [String]?
        var can_improve: [String]?
        var improvements: [String]?
        var url: String?
        var captured_at: String?
    }

    /// Scan `.astroshot/friction-logs/*` under a single `.astroshot` directory.
    static func loadLogs(inAstroshot astroshot: URL) -> [FrictionLog] {
        let root = FrictionLogPath.root(inAstroshot: astroshot)
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: root.path, isDirectory: &isDir), isDir.boolValue else {
            return []
        }

        let worktreeURL = astroshot.deletingLastPathComponent()
        let worktree = worktreeURL.lastPathComponent
        let worktreePath = worktreeURL.standardizedFileURL.path

        guard let children = try? fm.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey, .contentModificationDateKey],
            options: [.skipsHiddenFiles]
        ) else { return [] }

        return children.compactMap { slugDir -> FrictionLog? in
            var childIsDir: ObjCBool = false
            guard fm.fileExists(atPath: slugDir.path, isDirectory: &childIsDir),
                  childIsDir.boolValue
            else { return nil }
            return loadLog(
                slugDirectory: slugDir,
                worktree: worktree,
                worktreePath: worktreePath
            )
        }
        .sorted { $0.updatedAt > $1.updatedAt }
    }

    static func loadLog(
        slugDirectory: URL,
        worktree: String,
        worktreePath: String
    ) -> FrictionLog? {
        let fm = FileManager.default
        let slug = slugDirectory.lastPathComponent
        guard !slug.isEmpty else { return nil }

        let metaURL = slugDirectory.appendingPathComponent(FrictionLogPath.metaFileName)
        let meta = loadMeta(from: metaURL)
        let promptURL = slugDirectory.appendingPathComponent(FrictionLogPath.promptFileName)
        let promptMarkdown: String?
        let promptPath: String?
        if fm.fileExists(atPath: promptURL.path) {
            promptMarkdown = try? String(contentsOf: promptURL, encoding: .utf8)
            promptPath = promptURL.path
        } else {
            promptMarkdown = nil
            promptPath = nil
        }

        // Require at least a prompt or a run so empty placeholder dirs stay hidden.
        let runs = loadRuns(in: slugDirectory)
        guard promptPath != nil || !runs.isEmpty else { return nil }

        let title = meta?.title?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
            ?? humanize(slug)
        let description = meta?.description?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let status = FrictionLogStatus(raw: meta?.status)
            ?? runs.first?.status

        var updatedAt = directoryMTime(slugDirectory) ?? Date.distantPast
        if let promptPath,
           let m = fileMTime(promptPath),
           m > updatedAt
        {
            updatedAt = m
        }
        if let m = fileMTime(metaURL.path), m > updatedAt {
            updatedAt = m
        }
        if let run = runs.first, run.capturedAt > updatedAt {
            updatedAt = run.capturedAt
        }
        if let iso = meta?.updated_at, let parsed = parseISO8601(iso), parsed > updatedAt {
            updatedAt = parsed
        }

        return FrictionLog(
            worktree: worktree,
            worktreePath: worktreePath,
            slug: slug,
            title: title,
            description: description,
            promptPath: promptPath,
            promptMarkdown: promptMarkdown,
            status: status,
            runs: runs,
            updatedAt: updatedAt
        )
    }

    /// Parse a JSONL file into ordered steps. Screenshots resolve relative to `runDirectory`.
    static func parseJSONL(at url: URL, runDirectory: URL) -> [FrictionLogStep] {
        guard let data = try? Data(contentsOf: url),
              let text = String(data: data, encoding: .utf8)
        else { return [] }

        let decoder = JSONDecoder()
        var steps: [FrictionLogStep] = []
        var lineIndex = 0
        for rawLine in text.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty, !line.hasPrefix("#") else { continue }
            lineIndex += 1
            guard let lineData = line.data(using: .utf8),
                  let doc = try? decoder.decode(StepDocument.self, from: lineData)
            else { continue }

            let stepNumber = doc.step ?? lineIndex
            let stepID = doc.id?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
                ?? "step-\(stepNumber)"
            let title = doc.title?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
                ?? humanize(stepID)
            let description = doc.description?.trimmingCharacters(in: .whitespacesAndNewlines)
                ?? ""
            let names: [String]
            if let list = doc.screenshots, !list.isEmpty {
                names = list
            } else if let single = doc.screenshot, !single.isEmpty {
                names = [single]
            } else {
                names = []
            }
            let absolute = names.compactMap { name -> String? in
                let base = (name as NSString).lastPathComponent
                let candidate = runDirectory.appendingPathComponent(base)
                if FileManager.default.fileExists(atPath: candidate.path) {
                    return candidate.standardizedFileURL.path
                }
                // Allow nested paths written in the JSONL.
                let nested = runDirectory.appendingPathComponent(name)
                if FileManager.default.fileExists(atPath: nested.path) {
                    return nested.standardizedFileURL.path
                }
                return nil
            }
            let good = doc.good ?? doc.looks_good ?? []
            let improve = doc.improve ?? doc.can_improve ?? doc.improvements ?? []
            steps.append(
                FrictionLogStep(
                    step: stepNumber,
                    stepID: stepID,
                    title: title,
                    description: description,
                    screenshotPaths: absolute,
                    good: good.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                        .filter { !$0.isEmpty },
                    improve: improve.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                        .filter { !$0.isEmpty },
                    url: doc.url,
                    capturedAt: doc.captured_at.flatMap(parseISO8601)
                )
            )
        }
        return steps.sorted { $0.step < $1.step }
    }

    // MARK: Private helpers

    private static func loadRuns(in slugDirectory: URL) -> [FrictionLogRun] {
        let fm = FileManager.default
        let runsRoot = slugDirectory.appendingPathComponent(
            FrictionLogPath.runsDirectoryName,
            isDirectory: true
        )
        var runs: [FrictionLogRun] = []

        var isDir: ObjCBool = false
        if fm.fileExists(atPath: runsRoot.path, isDirectory: &isDir), isDir.boolValue,
           let children = try? fm.contentsOfDirectory(
               at: runsRoot,
               includingPropertiesForKeys: [.isDirectoryKey, .contentModificationDateKey],
               options: [.skipsHiddenFiles]
           )
        {
            for runDir in children {
                var childIsDir: ObjCBool = false
                guard fm.fileExists(atPath: runDir.path, isDirectory: &childIsDir),
                      childIsDir.boolValue
                else { continue }
                if let run = loadRun(directory: runDir, runID: runDir.lastPathComponent) {
                    runs.append(run)
                }
            }
        }

        // Flat layout: log.jsonl directly under the slug directory.
        let flatLog = slugDirectory.appendingPathComponent(FrictionLogPath.logFileName)
        if fm.fileExists(atPath: flatLog.path),
           let run = loadRun(directory: slugDirectory, runID: "latest")
        {
            // Prefer nested runs; only use flat if no nested runs exist.
            if runs.isEmpty {
                runs.append(run)
            }
        }

        return runs.sorted { $0.capturedAt > $1.capturedAt }
    }

    private static func loadRun(directory: URL, runID: String) -> FrictionLogRun? {
        let logURL = directory.appendingPathComponent(FrictionLogPath.logFileName)
        let steps = FileManager.default.fileExists(atPath: logURL.path)
            ? parseJSONL(at: logURL, runDirectory: directory)
            : []
        // A run directory with only screenshots and no JSONL is still useful to surface.
        guard !steps.isEmpty || hasImages(in: directory) else { return nil }

        let mtime = directoryMTime(directory) ?? fileMTime(logURL.path) ?? Date()
        let metaURL = directory.appendingPathComponent(FrictionLogPath.metaFileName)
        let runMeta = loadMeta(from: metaURL)
        let status = FrictionLogStatus(raw: runMeta?.status)

        return FrictionLogRun(
            runID: runID,
            directoryPath: directory.standardizedFileURL.path,
            steps: steps,
            capturedAt: mtime,
            status: status
        )
    }

    private static func hasImages(in directory: URL) -> Bool {
        guard let files = try? FileManager.default.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        ) else { return false }
        return files.contains {
            ShotPath.imageExtensions.contains($0.pathExtension.lowercased())
        }
    }

    private static func loadMeta(from url: URL) -> MetaDocument? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(MetaDocument.self, from: data)
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

    private static func directoryMTime(_ url: URL) -> Date? {
        let values = try? url.resourceValues(forKeys: [.contentModificationDateKey])
        return values?.contentModificationDate
    }

    private static func fileMTime(_ path: String) -> Date? {
        let attrs = try? FileManager.default.attributesOfItem(atPath: path)
        return attrs?[.modificationDate] as? Date
    }

    private static func parseISO8601(_ raw: String) -> Date? {
        let withFractional = ISO8601DateFormatter()
        withFractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withFractional.date(from: raw) { return date }
        let basic = ISO8601DateFormatter()
        basic.formatOptions = [.withInternetDateTime]
        return basic.date(from: raw)
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
