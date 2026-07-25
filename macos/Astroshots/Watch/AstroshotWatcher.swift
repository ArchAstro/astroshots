import CoreServices
import Foundation

/// Discovers and live-watches `.astroshot/<feature>/*` under one or more roots.
/// Callbacks are always delivered on the main queue.
///
/// Initial and full scans run on a background queue so the menu-bar app can
/// appear immediately (scanning a large home/worktree tree on the main thread
/// made the app look like it crashed with no status item).
final class AstroshotWatcher: @unchecked Sendable {
    struct Configuration: Sendable {
        var roots: [URL]
        /// Wait for partial PNG writes to settle before ingesting.
        var settleNanos: UInt64 = 250_000_000
    }

    var onShotsChanged: (@MainActor ([Shot]) -> Void)?
    var onNewShot: (@MainActor (Shot) -> Void)?
    /// Fired on main when a background full scan begins / ends.
    var onScanStateChanged: (@MainActor (Bool) -> Void)?

    private var configuration: Configuration
    private var stream: FSEventStreamRef?
    private var knownPaths: Set<String> = []
    private var settleWorkItems: [String: DispatchWorkItem] = [:]
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "ai.archastro.astroshots.fsevents", qos: .utility)
    private var scanGeneration = 0

    /// Directory names we never descend into (monorepo / build junk).
    private static let skipDirectoryNames: Set<String> = [
        "node_modules", ".git", "DerivedData", "build", ".build", "Pods",
        ".next", "dist", "out", "target", "vendor", "Checkouts", "xcuserdata",
        ".turbo", ".cache", "coverage", "tmp", ".pnpm-store", "Carthage",
        "bazel-bin", "bazel-out", "bazel-testlogs", ".gradle",
    ]

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

    /// Non-blocking: starts FSEvents immediately, scans on a background queue.
    func start() {
        stop()
        startFSEvents()
        scheduleFullScan()
    }

    func stop() {
        if let stream {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
            self.stream = nil
        }
        lock.lock()
        scanGeneration += 1
        settleWorkItems.values.forEach { $0.cancel() }
        settleWorkItems.removeAll()
        lock.unlock()
        emitScanState(false)
    }

    func rescan() {
        scheduleFullScan()
    }

    private func scheduleFullScan() {
        lock.lock()
        scanGeneration += 1
        let generation = scanGeneration
        lock.unlock()

        emitScanState(true)
        queue.async { [weak self] in
            guard let self else { return }
            let shots = self.scanAll()
            self.lock.lock()
            let stillCurrent = self.scanGeneration == generation
            if stillCurrent {
                self.knownPaths = Set(shots.map(\.path))
            }
            self.lock.unlock()
            guard stillCurrent else { return }
            self.emitShots(shots)
            self.emitScanState(false)
        }
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
            guard depth < 10 else { return }
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
                if Self.skipDirectoryNames.contains(name) {
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
        var needsFeatureRefresh = false
        for path in paths {
            if path.hasSuffix("manifest.json") {
                needsFeatureRefresh = true
                continue
            }

            let ext = URL(fileURLWithPath: path).pathExtension.lowercased()
            guard ShotPath.imageExtensions.contains(ext) else { continue }
            guard ShotPath.parse(imagePath: path) != nil else { continue }
            scheduleSettledIngest(path: path)
        }
        // Manifest-only updates: cheap re-scan of that feature dir is enough,
        // but a full scan is rare; schedule one background rescan.
        if needsFeatureRefresh {
            scheduleFullScan()
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
            if known { knownPaths.remove(path) }
            lock.unlock()
            if known {
                // Drop from UI via full list refresh (background).
                scheduleFullScan()
            }
            return
        }

        guard let shot = makeShot(at: path) else { return }

        lock.lock()
        let isNew = !knownPaths.contains(path)
        knownPaths.insert(path)
        lock.unlock()

        // Do not re-walk the whole tree for every PNG — emit the shot and let
        // AppState merge it into the list.
        if isNew {
            emitNew(shot)
        } else {
            emitNew(shot) // treat overwrite as update; AppState upserts by path
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

    private func emitScanState(_ scanning: Bool) {
        let handler = onScanStateChanged
        DispatchQueue.main.async {
            Task { @MainActor in
                handler?(scanning)
            }
        }
    }
}
