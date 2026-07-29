import CoreServices
import Foundation

/// Discovers and live-watches `.astroshot/<feature>/*` under one or more roots.
/// Callbacks are always delivered on the main queue.
///
/// Startup:
/// 1. **Indexed** — re-scan `.astroshot` dirs from a persistent index (fast).
/// 2. **Recovery** — recursively discover only when the index is missing,
///    empty, stale, or the user explicitly requests a rescan.
///
/// Full scans run on a background queue so the menu-bar status item can appear
/// immediately even when a watched root is a large monorepo.
final class AstroshotWatcher: @unchecked Sendable {
    struct Configuration: Sendable {
        var roots: [URL]
        /// Wait for partial PNG writes to settle before ingesting.
        var settleNanos: UInt64 = 250_000_000
        /// Injectable so tests never read or write the user's live index.
        var cacheFileURL: URL = ShotIndexCache.cacheFileURL()
    }

    private struct ScanResult: Sendable {
        var shots: [Shot]
        var astroshotDirs: [String]
    }

    var onShotsChanged: (@MainActor ([Shot]) -> Void)?
    /// Replaces just one feature directory after its manifest or review
    /// sidecar changes, avoiding a recursive workspace scan per comment.
    var onFeatureShotsChanged: (@MainActor (_ featureDirectoryPath: String, _ shots: [Shot]) -> Void)?
    var onNewShot: (@MainActor (Shot) -> Void)?
    /// Fired on main when a background full scan begins / ends.
    var onScanStateChanged: (@MainActor (Bool) -> Void)?

    private var configuration: Configuration
    private var stream: FSEventStreamRef?
    private var knownPaths: Set<String> = []
    /// `.astroshot` directories known from cache and/or the last full scan.
    private var knownAstroshotDirs: Set<String> = []
    /// Durable newest-arrival-first image paths from `shot-index.json`.
    private var arrivalOrder: [String] = []
    /// Most recent FSEvent represented by the in-memory/indexed state.
    private var lastEventID: FSEventStreamEventId?
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

    /// Non-blocking: FSEvents first, then indexed startup or recovery discovery.
    func start() {
        stop()
        lock.lock()
        let roots = configuration.roots
        let cacheFileURL = configuration.cacheFileURL
        lock.unlock()
        let cachedEventID = ShotIndexCache.load(
            for: roots,
            cacheFileURL: cacheFileURL
        )?.lastEventID
        // Legacy indexes have no cursor. Establish one before scanning known
        // directories so the event stream closes the scan/startup race without
        // forcing a one-time recursive walk during this upgrade.
        let resumeEventID = cachedEventID ?? FSEventsGetCurrentEventId()
        lock.lock()
        lastEventID = resumeEventID
        lock.unlock()
        scheduleStartupScan()
        startFSEvents(since: resumeEventID)
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
        scheduleFullScan(persistCache: true)
    }

    // MARK: Startup / full scan

    private func scheduleStartupScan() {
        lock.lock()
        scanGeneration += 1
        let generation = scanGeneration
        let roots = configuration.roots
        let cacheFileURL = configuration.cacheFileURL
        lock.unlock()

        emitScanState(true)
        queue.async { [weak self] in
            guard let self else { return }
            let cache = ShotIndexCache.load(
                for: roots,
                cacheFileURL: cacheFileURL
            )
            let cachedArrivalOrder = cache?.arrivalOrder ?? []
            self.lock.lock()
            if self.scanGeneration == generation {
                self.arrivalOrder = cachedArrivalOrder
            }
            self.lock.unlock()

            // A non-empty, internally consistent index is authoritative at
            // launch. FSEvents keeps it current after the stream starts.
            if let cache, !cache.astroshotDirs.isEmpty {
                let warm = self.scanKnownAstroshotDirs(cache.astroshotDirs)
                let warmOrder = ShotIndexCache.reconciledArrivalOrder(
                    existing: cachedArrivalOrder,
                    shots: warm.shots
                )
                self.lock.lock()
                let stillCurrent = self.scanGeneration == generation
                if stillCurrent {
                    self.knownPaths = Set(warm.shots.map(\.path))
                    self.knownAstroshotDirs = Set(warm.astroshotDirs)
                }
                self.lock.unlock()
                if stillCurrent {
                    self.emitShots(
                        ShotIndexCache.orderedShots(
                            warm.shots,
                            arrivalOrder: warmOrder
                        )
                    )
                }
                guard stillCurrent else { return }

                // Missing indexed directories mean the filesystem moved while
                // Astroshots was closed. Recover once instead of preserving a
                // silently incomplete index.
                if warm.astroshotDirs.count == cache.astroshotDirs.count {
                    ShotIndexCache.save(
                        roots: roots,
                        astroshotDirs: warm.astroshotDirs,
                        arrivalOrder: warmOrder,
                        lastEventID: self.currentLastEventID(),
                        cacheFileURL: cacheFileURL
                    )
                    self.emitScanState(false)
                    return
                }
            }

            guard self.isScanCurrent(generation) else { return }

            // Missing, empty, or stale index: recover with one full discovery.
            let full = self.scanAllCollectingDirs()
            let reconciledOrder = ShotIndexCache.reconciledArrivalOrder(
                existing: cachedArrivalOrder,
                shots: full.shots
            )
            self.lock.lock()
            let stillCurrent = self.scanGeneration == generation
            if stillCurrent {
                self.knownPaths = Set(full.shots.map(\.path))
                self.knownAstroshotDirs = Set(full.astroshotDirs)
                self.arrivalOrder = reconciledOrder
            }
            self.lock.unlock()
            guard stillCurrent else { return }

            self.emitShots(
                ShotIndexCache.orderedShots(
                    full.shots,
                    arrivalOrder: reconciledOrder
                )
            )
            ShotIndexCache.save(
                roots: roots,
                astroshotDirs: full.astroshotDirs,
                arrivalOrder: reconciledOrder,
                lastEventID: self.currentLastEventID(),
                cacheFileURL: cacheFileURL
            )
            self.emitScanState(false)
        }
    }

    private func scheduleFullScan(persistCache: Bool) {
        lock.lock()
        scanGeneration += 1
        let generation = scanGeneration
        let roots = configuration.roots
        let cacheFileURL = configuration.cacheFileURL
        lock.unlock()

        emitScanState(true)
        queue.async { [weak self] in
            guard let self else { return }
            let full = self.scanAllCollectingDirs()
            self.lock.lock()
            let existingOrder = self.arrivalOrder
            self.lock.unlock()
            let reconciledOrder = ShotIndexCache.reconciledArrivalOrder(
                existing: existingOrder,
                shots: full.shots
            )
            self.lock.lock()
            let stillCurrent = self.scanGeneration == generation
            if stillCurrent {
                self.knownPaths = Set(full.shots.map(\.path))
                self.knownAstroshotDirs = Set(full.astroshotDirs)
                self.arrivalOrder = reconciledOrder
            }
            self.lock.unlock()
            guard stillCurrent else { return }
            self.emitShots(
                ShotIndexCache.orderedShots(
                    full.shots,
                    arrivalOrder: reconciledOrder
                )
            )
            if persistCache {
                ShotIndexCache.save(
                    roots: roots,
                    astroshotDirs: full.astroshotDirs,
                    arrivalOrder: reconciledOrder,
                    lastEventID: self.currentLastEventID(),
                    cacheFileURL: cacheFileURL
                )
            }
            self.emitScanState(false)
        }
    }

    private func isScanCurrent(_ generation: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return scanGeneration == generation
    }

    private func currentLastEventID() -> UInt64? {
        lock.lock()
        defer { lock.unlock() }
        return lastEventID
    }

    // MARK: Scan

    /// Public entry for tests / tooling: unique shots newest-first.
    func scanAll() -> [Shot] {
        scanAllCollectingDirs().shots
    }

    private func scanAllCollectingDirs() -> ScanResult {
        lock.lock()
        let roots = configuration.roots
        lock.unlock()

        var shots: [Shot] = []
        var dirs: [String] = []
        for root in roots {
            guard FileManager.default.fileExists(atPath: root.path) else { continue }
            let partial = scanAstroshotTrees(under: root)
            shots.append(contentsOf: partial.shots)
            dirs.append(contentsOf: partial.astroshotDirs)
        }

        return ScanResult(
            shots: uniqueSortedShots(shots),
            astroshotDirs: Array(Set(dirs)).sorted()
        )
    }

    /// Re-scan only previously discovered `.astroshot` trees (no monorepo walk).
    private func scanKnownAstroshotDirs(_ dirs: [String]) -> ScanResult {
        let fm = FileManager.default
        var shots: [Shot] = []
        var liveDirs: [String] = []

        for path in dirs {
            var isDir: ObjCBool = false
            guard fm.fileExists(atPath: path, isDirectory: &isDir), isDir.boolValue else {
                continue
            }
            let url = URL(fileURLWithPath: path, isDirectory: true)
            guard url.lastPathComponent == ShotPath.astroshotDirName else { continue }
            liveDirs.append(url.standardizedFileURL.path)
            shots.append(contentsOf: scanFeatureDirs(in: url))
        }

        return ScanResult(
            shots: uniqueSortedShots(shots),
            astroshotDirs: Array(Set(liveDirs)).sorted()
        )
    }

    private func scanAstroshotTrees(under root: URL) -> ScanResult {
        var results: [Shot] = []
        var dirs: [String] = []
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
                    dirs.append(child.standardizedFileURL.path)
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
        return ScanResult(shots: results, astroshotDirs: dirs)
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

    private func uniqueSortedShots(_ shots: [Shot]) -> [Shot] {
        var seen = Set<String>()
        var unique: [Shot] = []
        for shot in shots.sorted(by: { $0.capturedAt > $1.capturedAt }) {
            if seen.insert(shot.path).inserted {
                unique.append(shot)
            }
        }
        return unique
    }

    func makeShot(at path: String, manifest: FeatureManifest? = nil) -> Shot? {
        let canonicalPath = Self.canonicalPath(path)
        guard let parts = ShotPath.parse(imagePath: canonicalPath) else { return nil }
        let featureDir = URL(fileURLWithPath: canonicalPath).deletingLastPathComponent()
        let resolvedManifest = manifest ?? ManifestParser.load(from: featureDir)
        let meta = ManifestParser.shotMetadata(fileName: parts.fileName, manifest: resolvedManifest)
        let review = try? ReviewStore.readReview(
            forImage: URL(fileURLWithPath: canonicalPath),
            featureDirectory: featureDir,
            expectedRunID: meta.runID
        )

        let attrs = try? FileManager.default.attributesOfItem(atPath: canonicalPath)
        let mtime = (attrs?[.modificationDate] as? Date) ?? Date()

        return Shot(
            path: canonicalPath,
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
            capturedAt: meta.capturedAt ?? mtime,
            review: review
        )
    }

    // MARK: FSEvents

    private func startFSEvents(since lastEventID: FSEventStreamEventId?) {
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

        let callback: FSEventStreamCallback = {
            _, info, numEvents, eventPaths, eventFlags, eventIDs in
            guard let info else { return }
            let watcher = Unmanaged<AstroshotWatcher>.fromOpaque(info).takeUnretainedValue()
            let paths = unsafeBitCast(eventPaths, to: NSArray.self) as? [String] ?? []
            let limited = Array(paths.prefix(numEvents))
            let flags = Array(
                UnsafeBufferPointer(start: eventFlags, count: numEvents)
            )
            let ids = Array(
                UnsafeBufferPointer(start: eventIDs, count: numEvents)
            )
            let recoveryFlags =
                FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs)
                | FSEventStreamEventFlags(kFSEventStreamEventFlagUserDropped)
                | FSEventStreamEventFlags(kFSEventStreamEventFlagKernelDropped)
                | FSEventStreamEventFlags(kFSEventStreamEventFlagEventIdsWrapped)
            watcher.handleFSEvents(
                paths: limited,
                latestEventID: ids.max(),
                requiresFullScan: flags.contains { $0 & recoveryFlags != 0 }
            )
        }

        guard let stream = FSEventStreamCreate(
            kCFAllocatorDefault,
            callback,
            &context,
            paths,
            lastEventID ?? FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
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

    /// Internal so focused tests can exercise settle/cancellation behavior
    /// without relying on the host FSEvents daemon.
    func handleFSEvents(
        paths: [String],
        latestEventID: FSEventStreamEventId? = nil,
        requiresFullScan: Bool = false
    ) {
        if let latestEventID {
            lock.lock()
            lastEventID = max(lastEventID ?? 0, latestEventID)
            lock.unlock()
        }
        if requiresFullScan {
            scheduleFullScan(persistCache: true)
            return
        }

        var featureDirectories = Set<String>()
        for eventPath in paths {
            let path = Self.canonicalPath(eventPath)
            if path.hasSuffix("manifest.json") || path.hasSuffix(ReviewStore.sidecarFileName) {
                featureDirectories.insert(
                    URL(fileURLWithPath: path)
                        .deletingLastPathComponent()
                        .standardizedFileURL.path
                )
                continue
            }

            let ext = URL(fileURLWithPath: path).pathExtension.lowercased()
            guard ShotPath.imageExtensions.contains(ext) else { continue }
            guard ShotPath.parse(imagePath: path) != nil else { continue }
            scheduleSettledIngest(path: path)
        }
        for directory in featureDirectories {
            scheduleFeatureRefresh(directoryPath: directory)
        }
    }

    /// FSEvents may report `/private/var/...` for a file scanned through
    /// `/var/...`. Canonicalize both boundaries so one file cannot enter the
    /// stream twice under symlink-equivalent paths.
    private static func canonicalPath(_ path: String) -> String {
        URL(fileURLWithPath: path)
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .path
    }

    private func scheduleFeatureRefresh(directoryPath: String) {
        let key = "feature:\(directoryPath)"
        lock.lock()
        settleWorkItems[key]?.cancel()
        let nanos = configuration.settleNanos
        let generation = scanGeneration
        let work = DispatchWorkItem { [weak self] in
            self?.refreshFeatureDirectory(
                path: directoryPath,
                workItemKey: key,
                generation: generation
            )
        }
        settleWorkItems[key] = work
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(nanos)), execute: work)
        lock.unlock()
    }

    private func refreshFeatureDirectory(
        path: String,
        workItemKey: String,
        generation: Int
    ) {
        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        settleWorkItems[workItemKey] = nil
        lock.unlock()

        let directory = URL(fileURLWithPath: path, isDirectory: true)
        let shots = uniqueSortedShots(shotsInFeatureDirectory(directory))
        let featurePrefix = directory.standardizedFileURL.path + "/"

        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        knownPaths = knownPaths.filter { !$0.hasPrefix(featurePrefix) }
        knownPaths.formUnion(shots.map(\.path))
        lock.unlock()

        emitFeatureShots(
            directoryPath: directory.standardizedFileURL.path,
            shots: shots,
            generation: generation
        )
    }

    private func scheduleSettledIngest(path: String) {
        lock.lock()
        settleWorkItems[path]?.cancel()
        let nanos = configuration.settleNanos
        let generation = scanGeneration
        let work = DispatchWorkItem { [weak self] in
            self?.ingestIfReady(path: path, generation: generation)
        }
        settleWorkItems[path] = work
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(nanos)), execute: work)
        lock.unlock()
    }

    private func ingestIfReady(path: String, generation: Int) {
        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        settleWorkItems[path] = nil
        lock.unlock()

        guard FileManager.default.fileExists(atPath: path) else {
            lock.lock()
            let known = knownPaths.contains(path)
            if known { knownPaths.remove(path) }
            lock.unlock()
            if known {
                // Drop from UI via full list refresh (background).
                scheduleFullScan(persistCache: true)
            }
            return
        }

        guard let shot = makeShot(at: path) else { return }

        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        knownPaths.insert(path)
        if let dir = ShotIndexCache.astroshotDir(forImagePath: path) {
            knownAstroshotDirs.insert(dir)
        }
        arrivalOrder.removeAll { $0 == path }
        arrivalOrder.insert(path, at: 0)
        let roots = configuration.roots
        let cacheFileURL = configuration.cacheFileURL
        let lastEventID = self.lastEventID
        let dirsSnapshot = Array(knownAstroshotDirs)
        let orderSnapshot = arrivalOrder
        lock.unlock()

        ShotIndexCache.save(
            roots: roots,
            astroshotDirs: dirsSnapshot,
            arrivalOrder: orderSnapshot,
            lastEventID: lastEventID,
            cacheFileURL: cacheFileURL
        )

        // Do not re-walk the whole tree for every PNG — emit the shot and let
        // AppState merge it into the list.
        emitNew(shot, generation: generation)
    }

    private func emitShots(_ shots: [Shot]) {
        let handler = onShotsChanged
        DispatchQueue.main.async {
            Task { @MainActor in
                handler?(shots)
            }
        }
    }

    /// Internal so tests can prove a callback queued before a root replacement
    /// is rejected at MainActor delivery time.
    func emitNew(_ shot: Shot, generation: Int) {
        let handler = onNewShot
        DispatchQueue.main.async { [weak self] in
            Task { @MainActor in
                guard self?.isScanCurrent(generation) == true else { return }
                handler?(shot)
            }
        }
    }

    private func emitFeatureShots(
        directoryPath: String,
        shots: [Shot],
        generation: Int
    ) {
        let handler = onFeatureShotsChanged
        DispatchQueue.main.async { [weak self] in
            Task { @MainActor in
                guard self?.isScanCurrent(generation) == true else { return }
                handler?(directoryPath, shots)
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
