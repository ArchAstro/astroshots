import CoreServices
import Foundation

/// Discovers and live-watches `.astroshot/<feature>/*` under one or more roots.
/// Callbacks are always delivered on the main queue.
final class AstroshotWatcher: @unchecked Sendable {
    struct Configuration: Sendable {
        var roots: [URL]
        /// Wait for partial PNG writes to settle before ingesting.
        var settleNanos: UInt64 = 250_000_000
    }

    var onShotsChanged: (@MainActor ([Shot]) -> Void)?
    var onNewShot: (@MainActor (Shot) -> Void)?

    private var configuration: Configuration
    private var stream: FSEventStreamRef?
    private var knownPaths: Set<String> = []
    private var settleWorkItems: [String: DispatchWorkItem] = [:]
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "ai.archastro.astroshots.fsevents")

    init(configuration: Configuration) {
        self.configuration = configuration
    }

    deinit {
        if let stream {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
        }
    }

    func updateRoots(_ roots: [URL]) {
        stop()
        lock.lock()
        configuration.roots = roots
        lock.unlock()
        start()
    }

    func start() {
        stop()
        let existing = scanAll()
        lock.lock()
        knownPaths = Set(existing.map(\.path))
        lock.unlock()
        emitShots(existing)
        startFSEvents()
    }

    func stop() {
        if let stream {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
            self.stream = nil
        }
        lock.lock()
        settleWorkItems.values.forEach { $0.cancel() }
        settleWorkItems.removeAll()
        lock.unlock()
    }

    func rescan() {
        let shots = scanAll()
        lock.lock()
        knownPaths = Set(shots.map(\.path))
        lock.unlock()
        emitShots(shots)
    }

    // MARK: Scan

    func scanAll() -> [Shot] {
        lock.lock()
        let roots = configuration.roots
        lock.unlock()

        var shots: [Shot] = []
        for root in roots {
            guard FileManager.default.fileExists(atPath: root.path) else { continue }
            shots.append(contentsOf: scanAstroshotTrees(under: root))
        }

        var seen = Set<String>()
        var unique: [Shot] = []
        for shot in shots.sorted(by: { $0.capturedAt > $1.capturedAt }) {
            if seen.insert(shot.path).inserted {
                unique.append(shot)
            }
        }
        return unique
    }

    private func scanAstroshotTrees(under root: URL) -> [Shot] {
        var results: [Shot] = []
        let fm = FileManager.default

        func walk(_ dir: URL, depth: Int) {
            guard depth < 12 else { return }
            guard let children = try? fm.contentsOfDirectory(
                at: dir,
                includingPropertiesForKeys: [.isDirectoryKey],
                options: [.skipsPackageDescendants]
            ) else { return }

            for child in children {
                let name = child.lastPathComponent
                if name == ShotPath.astroshotDirName {
                    results.append(contentsOf: scanFeatureDirs(in: child))
                    continue
                }
                if name == "node_modules" || name == ".git" || name == "DerivedData"
                    || name == "build" || name == ".build" || name == "Pods"
                {
                    continue
                }
                var isDir: ObjCBool = false
                guard fm.fileExists(atPath: child.path, isDirectory: &isDir), isDir.boolValue else {
                    continue
                }
                if name.hasPrefix("."), name != ShotPath.astroshotDirName { continue }
                walk(child, depth: depth + 1)
            }
        }

        walk(root, depth: 0)
        return results
    }

    private func scanFeatureDirs(in astroshot: URL) -> [Shot] {
        let fm = FileManager.default
        guard let features = try? fm.contentsOfDirectory(
            at: astroshot,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else { return [] }

        var shots: [Shot] = []
        for featureDir in features {
            var isDir: ObjCBool = false
            guard fm.fileExists(atPath: featureDir.path, isDirectory: &isDir), isDir.boolValue else {
                continue
            }
            shots.append(contentsOf: shotsInFeatureDirectory(featureDir))
        }
        return shots
    }

    private func shotsInFeatureDirectory(_ featureDir: URL) -> [Shot] {
        let fm = FileManager.default
        let manifest = ManifestParser.load(from: featureDir)
        guard let files = try? fm.contentsOfDirectory(
            at: featureDir,
            includingPropertiesForKeys: [.contentModificationDateKey, .isRegularFileKey],
            options: [.skipsHiddenFiles]
        ) else { return [] }

        return files.compactMap { fileURL -> Shot? in
            let ext = fileURL.pathExtension.lowercased()
            guard ShotPath.imageExtensions.contains(ext) else { return nil }
            return makeShot(at: fileURL.path, manifest: manifest)
        }
    }

    func makeShot(at path: String, manifest: FeatureManifest? = nil) -> Shot? {
        guard let parts = ShotPath.parse(imagePath: path) else { return nil }
        let featureDir = URL(fileURLWithPath: path).deletingLastPathComponent()
        let resolvedManifest = manifest ?? ManifestParser.load(from: featureDir)
        let meta = ManifestParser.shotMetadata(fileName: parts.fileName, manifest: resolvedManifest)

        let attrs = try? FileManager.default.attributesOfItem(atPath: path)
        let mtime = (attrs?[.modificationDate] as? Date) ?? Date()

        return Shot(
            path: path,
            worktree: parts.worktree,
            worktreePath: parts.worktreePath,
            feature: parts.feature,
            fileName: parts.fileName,
            sequence: meta.sequence,
            slug: meta.slug,
            title: meta.title,
            description: meta.description,
            url: meta.url,
            runID: meta.runID,
            status: meta.status,
            capturedAt: meta.capturedAt ?? mtime
        )
    }

    // MARK: FSEvents

    private func startFSEvents() {
        lock.lock()
        let rootPaths = configuration.roots.map(\.path)
        lock.unlock()
        guard !rootPaths.isEmpty else { return }

        let paths = rootPaths as CFArray
        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil,
            release: nil,
            copyDescription: nil
        )

        let callback: FSEventStreamCallback = { _, info, numEvents, eventPaths, _, _ in
            guard let info else { return }
            let watcher = Unmanaged<AstroshotWatcher>.fromOpaque(info).takeUnretainedValue()
            let paths = unsafeBitCast(eventPaths, to: NSArray.self) as? [String] ?? []
            let limited = Array(paths.prefix(numEvents))
            watcher.handleFSEvents(paths: limited)
        }

        guard let stream = FSEventStreamCreate(
            kCFAllocatorDefault,
            callback,
            &context,
            paths,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            0.4,
            FSEventStreamCreateFlags(
                kFSEventStreamCreateFlagFileEvents
                    | kFSEventStreamCreateFlagUseCFTypes
                    | kFSEventStreamCreateFlagNoDefer
            )
        ) else {
            return
        }

        FSEventStreamSetDispatchQueue(stream, queue)
        FSEventStreamStart(stream)
        self.stream = stream
    }

    private func handleFSEvents(paths: [String]) {
        for path in paths {
            if path.hasSuffix("manifest.json") {
                rescan()
                continue
            }

            let ext = URL(fileURLWithPath: path).pathExtension.lowercased()
            guard ShotPath.imageExtensions.contains(ext) else { continue }
            guard ShotPath.parse(imagePath: path) != nil else { continue }
            scheduleSettledIngest(path: path)
        }
    }

    private func scheduleSettledIngest(path: String) {
        lock.lock()
        settleWorkItems[path]?.cancel()
        let nanos = configuration.settleNanos
        lock.unlock()

        let work = DispatchWorkItem { [weak self] in
            self?.ingestIfReady(path: path)
        }
        lock.lock()
        settleWorkItems[path] = work
        lock.unlock()
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(nanos)), execute: work)
    }

    private func ingestIfReady(path: String) {
        lock.lock()
        settleWorkItems[path] = nil
        lock.unlock()

        guard FileManager.default.fileExists(atPath: path) else {
            lock.lock()
            let known = knownPaths.contains(path)
            lock.unlock()
            if known { rescan() }
            return
        }

        guard let shot = makeShot(at: path) else { return }

        lock.lock()
        let isNew = !knownPaths.contains(path)
        knownPaths.insert(path)
        lock.unlock()

        rescan()
        if isNew {
            emitNew(shot)
        }
    }

    private func emitShots(_ shots: [Shot]) {
        let handler = onShotsChanged
        DispatchQueue.main.async {
            Task { @MainActor in
                handler?(shots)
            }
        }
    }

    private func emitNew(_ shot: Shot) {
        let handler = onNewShot
        DispatchQueue.main.async {
            Task { @MainActor in
                handler?(shot)
            }
        }
    }
}
