import CoreServices
import Foundation

/// Discovers and live-watches `.astroshot/<feature>/*` under one or more roots.
/// Callbacks are always delivered on the main queue.
///
/// Startup:
/// 1. **Indexed** — re-scan `.astroshot` dirs from a persistent index (fast).
/// 2. **Recovery** — recursively discover only when the index is missing,
///    stale, empty without an event cursor, or explicitly rescanned.
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
        var frictionLogs: [FrictionLog]
        var astroshotDirs: [String]
    }

    var onShotsChanged: (@MainActor ([Shot]) -> Void)?
    /// Replaces just one feature directory after its manifest or review
    /// sidecar changes, avoiding a recursive workspace scan per comment.
    var onFeatureShotsChanged: (@MainActor (_ featureDirectoryPath: String, _ shots: [Shot]) -> Void)?
    var onNewShot: (@MainActor (Shot) -> Void)?
    /// Full friction-log catalog under watched `.astroshot` trees.
    var onFrictionLogsChanged: (@MainActor ([FrictionLog]) -> Void)?
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
    /// Live filesystem delivery must never wait behind a recursive workspace
    /// walk. Scan results are serialized back onto `queue` before mutation.
    private let queue = DispatchQueue(label: "ai.archastro.astroshots.fsevents", qos: .utility)
    private let scanQueue: DispatchQueue
    private var scanGeneration = 0
    private var fullScanInFlight = false
    private var fullScanNeedsRerun = false
    private var rerunRoots: Set<URL> = []
    private var rerunShouldPersistCache = false
    private var rerunShouldNotifyNewShots = false

    /// Directory names we never descend into (monorepo / build junk).
    private static let skipDirectoryNames: Set<String> = [
        "node_modules", ".git", "DerivedData", "build", ".build", "Pods",
        ".next", "dist", "out", "target", "vendor", "Checkouts", "xcuserdata",
        ".turbo", ".cache", "coverage", "tmp", ".pnpm-store", "Carthage",
        "bazel-bin", "bazel-out", "bazel-testlogs", ".gradle", "_build", "deps",
    ]

    init(
        configuration: Configuration,
        scanQueue: DispatchQueue = DispatchQueue(label: "ai.archastro.astroshots.scan", qos: .utility)
    ) {
        self.configuration = configuration
        self.scanQueue = scanQueue
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
        fullScanInFlight = false
        fullScanNeedsRerun = false
        rerunRoots.removeAll()
        rerunShouldPersistCache = false
        rerunShouldNotifyNewShots = false
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

            // A consistent index is authoritative, even when empty, if its
            // event cursor lets FSEvents replay changes since it was saved.
            // Legacy non-empty indexes remain usable without a cursor.
            if let cache, !cache.astroshotDirs.isEmpty || cache.lastEventID != nil {
                // An older index may contain trees discovery now excludes.
                // Evict them before publishing state whose events we ignore.
                let eligibleDirs = cache.astroshotDirs.filter { self.relevantPath($0) != nil }
                let warm = self.scanKnownAstroshotDirs(eligibleDirs)
                let warmOrder = ShotIndexCache.reconciledArrivalOrder(
                    existing: cachedArrivalOrder,
                    shots: warm.shots
                )
                self.lock.lock()
                let stillCurrent = self.scanGeneration == generation
                if stillCurrent {
                    self.knownPaths = Set(warm.shots.map(\.path))
                    self.knownAstroshotDirs = Set(warm.astroshotDirs)
                    self.arrivalOrder = warmOrder
                }
                self.lock.unlock()
                if stillCurrent {
                    self.emitShots(
                        ShotIndexCache.orderedShots(
                            warm.shots,
                            arrivalOrder: warmOrder
                        )
                    )
                    self.emitFrictionLogs(warm.frictionLogs)
                }
                guard stillCurrent else { return }

                // Missing indexed directories mean the filesystem moved while
                // Astroshots was closed. Recover once instead of preserving a
                // silently incomplete index.
                // Exclusions are intentional eviction, not missing directories
                // that require another recursive workspace walk.
                if warm.astroshotDirs.count == eligibleDirs.count {
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

            // Missing, stale, or legacy empty index: recover with one full discovery.
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
            self.emitFrictionLogs(full.frictionLogs)
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

    private func scheduleFullScan(
        persistCache: Bool,
        notifyNewShots: Bool = false,
        scanRoots: [URL]? = nil
    ) {
        lock.lock()
        let requestedRoots = scanRoots ?? configuration.roots
        if fullScanInFlight {
            rerunRoots.formUnion(requestedRoots)
            fullScanNeedsRerun = true
            rerunShouldPersistCache = rerunShouldPersistCache || persistCache
            rerunShouldNotifyNewShots =
                rerunShouldNotifyNewShots || notifyNewShots
            lock.unlock()
            return
        }
        fullScanInFlight = true
        let generation = scanGeneration
        let roots = configuration.roots
        let cacheFileURL = configuration.cacheFileURL
        lock.unlock()

        emitScanState(true)
        scanQueue.async { [weak self] in
            guard let self else { return }
            let full = self.scanAllCollectingDirs(roots: requestedRoots)
            self.queue.async { [weak self] in
                self?.applyFullScan(
                    full,
                    generation: generation,
                    roots: roots,
                    cacheFileURL: cacheFileURL,
                    persistCache: persistCache,
                    notifyNewShots: notifyNewShots
                )
            }
        }
    }

    private func applyFullScan(
        _ full: ScanResult,
        generation: Int,
        roots: [URL],
        cacheFileURL: URL,
        persistCache: Bool,
        notifyNewShots: Bool
    ) {
        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        let previouslyKnownPaths = knownPaths
        let existingOrder = arrivalOrder
        let liveDirs = knownAstroshotDirs
        lock.unlock()

        // A file may land after the scan has already walked its directory but
        // before the result is applied. Preserve live-ingested paths that still
        // exist so reconciliation cannot erase a newer event.
        var shotsByPath = Dictionary(
            uniqueKeysWithValues: full.shots
                .filter { FileManager.default.fileExists(atPath: $0.path) }
                .map { ($0.path, $0) }
        )
        for path in previouslyKnownPaths where shotsByPath[path] == nil {
            guard FileManager.default.fileExists(atPath: path),
                  let shot = makeShot(at: path)
            else {
                continue
            }
            shotsByPath[path] = shot
        }
        let reconciledShots = uniqueSortedShots(Array(shotsByPath.values))
        let reconciledOrder = ShotIndexCache.reconciledArrivalOrder(
            existing: existingOrder,
            shots: reconciledShots
        )
        let reconciledDirs = Set(full.astroshotDirs).union(
            liveDirs.filter { FileManager.default.fileExists(atPath: $0) }
        )
        let newShots = reconciledShots.filter {
            !previouslyKnownPaths.contains($0.path)
        }

        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        knownPaths = Set(reconciledShots.map(\.path))
        knownAstroshotDirs = reconciledDirs
        arrivalOrder = reconciledOrder
        let lastEventID = self.lastEventID
        let shouldRerun = fullScanNeedsRerun
        let pendingRoots = Array(rerunRoots)
        let rerunPersistCache = rerunShouldPersistCache
        let rerunNotifyNewShots = rerunShouldNotifyNewShots
        fullScanInFlight = false
        fullScanNeedsRerun = false
        rerunRoots.removeAll()
        rerunShouldPersistCache = false
        rerunShouldNotifyNewShots = false
        lock.unlock()

        // Deliver newly recovered captures before the full replacement. This
        // gives AppState the same unread/overlay behavior as a normal event;
        // the subsequent list remains the authoritative deletion reconciliation.
        let orderedShots = ShotIndexCache.orderedShots(
            reconciledShots,
            arrivalOrder: reconciledOrder
        )
        if notifyNewShots {
            emitRecoveredShots(
                newShots: ShotIndexCache.orderedShots(
                    newShots,
                    arrivalOrder: reconciledOrder
                ),
                allShots: orderedShots,
                generation: generation
            )
        } else {
            emitShots(orderedShots)
        }
        // A scoped recovery must preserve stories in unaffected screenshot trees.
        emitFrictionLogs(sortedFrictionLogs(reconciledDirs.flatMap { path in
            autoreleasepool {
                scanFrictionLogs(in: URL(fileURLWithPath: path, isDirectory: true))
            }
        }))
        if persistCache {
            ShotIndexCache.save(
                roots: roots,
                astroshotDirs: Array(reconciledDirs),
                arrivalOrder: reconciledOrder,
                lastEventID: lastEventID,
                cacheFileURL: cacheFileURL
            )
        }

        if shouldRerun {
            scheduleFullScan(
                persistCache: rerunPersistCache,
                notifyNewShots: rerunNotifyNewShots,
                scanRoots: pendingRoots
            )
        } else {
            emitScanState(false)
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

    /// Public entry for tests / tooling: friction logs newest-first.
    func scanAllFrictionLogs() -> [FrictionLog] {
        scanAllCollectingDirs().frictionLogs
    }

    private func scanAllCollectingDirs(roots requestedRoots: [URL]? = nil) -> ScanResult {
        lock.lock()
        let roots = requestedRoots ?? configuration.roots
        lock.unlock()

        var shots: [Shot] = []
        var frictionLogs: [FrictionLog] = []
        var dirs: [String] = []
        let uniqueRoots = Set(roots.map { URL(fileURLWithPath: Self.canonicalPath($0.path)) })
        let disjointRoots = uniqueRoots.filter { candidate in
            !uniqueRoots.contains { other in
                other != candidate && candidate.path.hasPrefix(other.path + "/")
            }
        }
        for root in disjointRoots {
            guard FileManager.default.fileExists(atPath: root.path) else { continue }
            let partial = autoreleasepool { scanAstroshotTrees(under: root) }
            shots.append(contentsOf: partial.shots)
            frictionLogs.append(contentsOf: partial.frictionLogs)
            dirs.append(contentsOf: partial.astroshotDirs)
        }

        return ScanResult(
            shots: uniqueSortedShots(shots),
            frictionLogs: sortedFrictionLogs(frictionLogs),
            astroshotDirs: Array(Set(dirs)).sorted()
        )
    }

    /// Re-scan only previously discovered `.astroshot` trees (no monorepo walk).
    private func scanKnownAstroshotDirs(_ dirs: [String]) -> ScanResult {
        let fm = FileManager.default
        var shots: [Shot] = []
        var frictionLogs: [FrictionLog] = []
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
            frictionLogs.append(contentsOf: scanFrictionLogs(in: url))
        }

        return ScanResult(
            shots: uniqueSortedShots(shots),
            frictionLogs: sortedFrictionLogs(frictionLogs),
            astroshotDirs: Array(Set(liveDirs)).sorted()
        )
    }

    private func scanAstroshotTrees(under root: URL) -> ScanResult {
        var results: [Shot] = []
        var frictionLogs: [FrictionLog] = []
        var dirs: [String] = []
        let fm = FileManager.default

        func walk(_ dir: URL, depth: Int) {
            guard depth < 10 else { return }
            // Drain Foundation's temporary URLs and resource caches per directory,
            // rather than retaining them until the entire workspace scan finishes.
            autoreleasepool {
                guard let values = try? dir.resourceValues(forKeys: [.isSymbolicLinkKey]),
                      values.isSymbolicLink != true
                else { return }
                if dir.lastPathComponent == ShotPath.astroshotDirName {
                    dirs.append(dir.standardizedFileURL.path)
                    results.append(contentsOf: scanFeatureDirs(in: dir))
                    frictionLogs.append(contentsOf: scanFrictionLogs(in: dir))
                    return
                }
                guard let children = try? fm.contentsOfDirectory(
                    at: dir,
                    includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey],
                    options: [.skipsPackageDescendants]
                ) else { return }

                for child in children {
                    let name = child.lastPathComponent
                    if Self.skipDirectoryNames.contains(name) { continue }
                    if name.hasPrefix("."), name != ShotPath.astroshotDirName { continue }
                    guard let values = try? child.resourceValues(
                        forKeys: [.isDirectoryKey, .isSymbolicLinkKey]
                    ), values.isDirectory == true, values.isSymbolicLink != true
                    else { continue }
                    // .astroshot at the discovery depth boundary is still a target.
                    if name == ShotPath.astroshotDirName {
                        autoreleasepool {
                            dirs.append(child.standardizedFileURL.path)
                            results.append(contentsOf: scanFeatureDirs(in: child))
                            frictionLogs.append(contentsOf: scanFrictionLogs(in: child))
                        }
                    } else {
                        walk(child, depth: depth + 1)
                    }
                }
            }
        }

        walk(root, depth: 0)
        return ScanResult(
            shots: results,
            frictionLogs: frictionLogs,
            astroshotDirs: dirs
        )
    }

    private func sortedFrictionLogs(_ logs: [FrictionLog]) -> [FrictionLog] {
        logs.sorted { $0.updatedAt > $1.updatedAt }
    }

    private func scanFeatureDirs(in astroshot: URL) -> [Shot] {
        guard let features = try? FileManager.default.contentsOfDirectory(
            at: astroshot,
            includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey],
            options: [.skipsHiddenFiles]
        ) else { return [] }

        return features.flatMap { featureDir -> [Shot] in
            autoreleasepool {
                guard let values = try? featureDir.resourceValues(
                    forKeys: [.isDirectoryKey, .isSymbolicLinkKey]
                ), values.isDirectory == true, values.isSymbolicLink != true,
                   !FrictionLogPath.isReserved(directoryName: featureDir.lastPathComponent)
                else { return [] }
                return shotsInFeatureDirectory(featureDir)
            }
        }
    }

    private func scanFrictionLogs(in astroshot: URL) -> [FrictionLog] {
        FrictionLogLoader.loadLogs(inAstroshot: astroshot)
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
        let videoFileName = Self.resolveVideoFileName(
            fileName: parts.fileName,
            featureDirectory: featureDir,
            manifestVideo: meta.video,
            manifestKind: meta.kind
        )

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
            review: review,
            kind: meta.kind ?? (videoFileName != nil ? "movie" : nil),
            videoFileName: videoFileName,
            durationMs: meta.durationMs,
            movieSource: meta.source,
            chapters: meta.chapters
        )
    }

    /// Prefer explicit manifest `video`; otherwise pick a sibling with the same
    /// stem and a known movie extension (so posters stream even if metadata is thin).
    private static func resolveVideoFileName(
        fileName: String,
        featureDirectory: URL,
        manifestVideo: String?,
        manifestKind: String?
    ) -> String? {
        if let manifestVideo, !manifestVideo.isEmpty {
            let explicit = featureDirectory.appendingPathComponent(manifestVideo)
            if FileManager.default.fileExists(atPath: explicit.path) {
                return manifestVideo
            }
            // Still surface the basename so detail can show it even if missing.
            return manifestVideo
        }

        let stem = (fileName as NSString).deletingPathExtension
        for ext in ["webm", "mp4", "mov", "m4v"] {
            let candidate = "\(stem).\(ext)"
            let url = featureDirectory.appendingPathComponent(candidate)
            if FileManager.default.fileExists(atPath: url.path) {
                return candidate
            }
        }

        // kind:movie without a discoverable video still marks the row as a movie.
        if let manifestKind, manifestKind.caseInsensitiveCompare("movie") == .orderedSame {
            return nil
        }
        return nil
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
            watcher.handleFSEvents(
                paths: limited,
                latestEventID: ids.max(),
                flags: flags
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
        requiresFullScan: Bool = false,
        flags: [FSEventStreamEventFlags] = []
    ) {
        let droppedHistoryFlags = FSEventStreamEventFlags(
            kFSEventStreamEventFlagUserDropped | kFSEventStreamEventFlagKernelDropped
                | kFSEventStreamEventFlagEventIdsWrapped
        )
        let requiresFullScan = requiresFullScan || flags.contains { $0 & droppedHistoryFlags != 0 }
        if let latestEventID {
            lock.lock()
            lastEventID = max(lastEventID ?? 0, latestEventID)
            lock.unlock()
        }
        if requiresFullScan {
            // Preserve every usable path in this batch while reconciliation
            // runs independently. Dropped-history flags mean "verify the tree",
            // not "throw away the events we did receive."
            scheduleFullScan(persistCache: true, notifyNewShots: true)
        }

        var featureDirectories = Set<String>()
        var frictionLogTouched = false
        var recoveryRoots = Set<URL>()
        for (index, eventPath) in paths.enumerated() {
            guard let path = relevantPath(eventPath) else { continue }
            let eventFlags = index < flags.count ? flags[index] : 0
            let mustScan = eventFlags & FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs) != 0
            let directoryChanged = eventFlags & FSEventStreamEventFlags(kFSEventStreamEventFlagItemIsDir) != 0
                && eventFlags & FSEventStreamEventFlags(
                    kFSEventStreamEventFlagItemCreated | kFSEventStreamEventFlagItemRemoved
                        | kFSEventStreamEventFlagItemRenamed
                ) != 0
            if !requiresFullScan, mustScan || directoryChanged {
                // Recovery within a feature is the same bounded refresh as a
                // manifest change. Outside .astroshot, discover only this subtree.
                if let range = path.range(of: "/\(ShotPath.astroshotDirName)/") {
                    let tail = path[range.upperBound...]
                    if let feature = tail.split(separator: "/").first {
                        if FrictionLogPath.isReserved(directoryName: String(feature)) {
                            frictionLogTouched = true
                        } else {
                            featureDirectories.insert(String(path[..<range.upperBound]) + feature)
                        }
                    }
                } else {
                    recoveryRoots.insert(URL(fileURLWithPath: path, isDirectory: true))
                }
            }
            if FrictionLogPath.containsPath(path) {
                frictionLogTouched = true
                continue
            }
            if path.contains("/\(ShotPath.astroshotDirName)/"),
               path.hasSuffix("/manifest.json") || path.hasSuffix("/" + ReviewStore.sidecarFileName) {
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
        // Collapse ancestor/descendant requests in one batch before scheduling.
        let scopes = recoveryRoots.filter { candidate in
            !recoveryRoots.contains { other in
                other != candidate && candidate.path.hasPrefix(other.path + "/")
            }
        }
        if !scopes.isEmpty {
            scheduleFullScan(persistCache: true, notifyNewShots: true, scanRoots: Array(scopes))
        }
        for directory in featureDirectories {
            // Never treat a reserved story root as a shot feature.
            if FrictionLogPath.isReserved(
                directoryName: URL(fileURLWithPath: directory).lastPathComponent
            ) {
                frictionLogTouched = true
                continue
            }
            scheduleFeatureRefresh(directoryPath: directory)
        }
        if frictionLogTouched {
            scheduleFrictionLogRefresh()
        }
    }

    /// Apply the discovery exclusions relative to a watch root, not to its
    /// ancestors (a user may explicitly watch a checkout beneath /tmp).
    private func relevantPath(_ candidatePath: String) -> String? {
        let path = Self.canonicalPath(candidatePath)
        lock.lock()
        let roots = configuration.roots
        lock.unlock()
        for root in roots {
            let rootPath = Self.canonicalPath(root.path)
            let rootPrefix = rootPath == "/" ? "/" : rootPath + "/"
            guard path == rootPath || path.hasPrefix(rootPrefix) else { continue }
            let components = path.dropFirst(rootPath.count).split(separator: "/")
            var excluded = false
            for component in components {
                if component == ShotPath.astroshotDirName { break }
                let name = String(component)
                if Self.skipDirectoryNames.contains(name) || name.hasPrefix(".") {
                    excluded = true
                    break
                }
            }
            if !excluded { return path }
        }
        return nil
    }

    private func scheduleFrictionLogRefresh() {
        let key = "stories"
        lock.lock()
        settleWorkItems[key]?.cancel()
        let nanos = configuration.settleNanos
        let generation = scanGeneration
        let work = DispatchWorkItem { [weak self] in
            self?.refreshFrictionLogs(workItemKey: key, generation: generation)
        }
        settleWorkItems[key] = work
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(nanos)), execute: work)
        lock.unlock()
    }

    private func refreshFrictionLogs(workItemKey: String, generation: Int) {
        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        settleWorkItems[workItemKey] = nil
        let dirs = Array(knownAstroshotDirs)
        lock.unlock()

        let logs = sortedFrictionLogs(
            dirs.flatMap { path in
                scanFrictionLogs(in: URL(fileURLWithPath: path, isDirectory: true))
            }
        )
        emitFrictionLogs(logs)
    }

    /// FSEvents may report `/private/var/...` for a file scanned through
    /// `/var/...`. Canonicalize both boundaries so one file cannot enter the
    /// stream twice under symlink-equivalent paths.
    private static func canonicalPath(_ path: String) -> String {
        // Foundation leaves an absent leaf unresolved. Resolve the nearest
        // surviving ancestor so deletion events retain their original identity
        // (including macOS /private/var versus /var aliases).
        var ancestor = URL(fileURLWithPath: path).standardizedFileURL
        var missingComponents: [String] = []
        while ancestor.path != "/", !FileManager.default.fileExists(atPath: ancestor.path) {
            missingComponents.append(ancestor.lastPathComponent)
            ancestor.deleteLastPathComponent()
        }
        var resolved = ancestor.resolvingSymlinksInPath()
        for component in missingComponents.reversed() {
            resolved.appendPathComponent(component)
        }
        return resolved.path
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
        let shots = autoreleasepool { uniqueSortedShots(shotsInFeatureDirectory(directory)) }
        let featurePrefix = directory.standardizedFileURL.path + "/"

        lock.lock()
        guard scanGeneration == generation else {
            lock.unlock()
            return
        }
        let newlyDiscovered = shots.filter {
            !knownPaths.contains($0.path)
        }
        knownPaths = knownPaths.filter { !$0.hasPrefix(featurePrefix) }
        knownPaths.formUnion(shots.map(\.path))
        arrivalOrder.removeAll { !knownPaths.contains($0) }
        arrivalOrder.insert(contentsOf: newlyDiscovered.map(\.path), at: 0)
        if let dir = ShotIndexCache.astroshotDir(forImagePath: path + "/frame.png"),
           FileManager.default.fileExists(atPath: dir) {
            knownAstroshotDirs.insert(dir)
        }
        let roots = configuration.roots
        let cacheFileURL = configuration.cacheFileURL
        let dirs = Array(knownAstroshotDirs)
        let order = arrivalOrder
        let eventID = lastEventID
        lock.unlock()
        ShotIndexCache.save(
            roots: roots, astroshotDirs: dirs, arrivalOrder: order,
            lastEventID: eventID, cacheFileURL: cacheFileURL
        )

        // A manifest rename may be the only coalesced event we receive for a
        // rapid atomic capture batch. Treat frames found by that targeted scan
        // as real arrivals so they still increment unread and flash overlays.
        emitFeatureShots(
            directoryPath: directory.standardizedFileURL.path,
            shots: shots,
            newlyDiscovered: newlyDiscovered,
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
                // Reconcile the containing feature, including its durable order,
                // without rediscovering every repository in the workspace.
                scheduleFeatureRefresh(
                    directoryPath: URL(fileURLWithPath: path).deletingLastPathComponent().path
                )
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
        // AppState insert new paths or refresh replacements in place.
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

    private func emitFrictionLogs(_ logs: [FrictionLog]) {
        let handler = onFrictionLogsChanged
        DispatchQueue.main.async {
            Task { @MainActor in
                handler?(logs)
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
        newlyDiscovered: [Shot] = [],
        generation: Int
    ) {
        let newHandler = onNewShot
        let handler = onFeatureShotsChanged
        DispatchQueue.main.async { [weak self] in
            Task { @MainActor in
                guard self?.isScanCurrent(generation) == true else { return }
                for shot in newlyDiscovered {
                    newHandler?(shot)
                }
                handler?(directoryPath, shots)
            }
        }
    }

    private func emitRecoveredShots(
        newShots: [Shot],
        allShots: [Shot],
        generation: Int
    ) {
        let newHandler = onNewShot
        let listHandler = onShotsChanged
        DispatchQueue.main.async { [weak self] in
            Task { @MainActor in
                guard self?.isScanCurrent(generation) == true else { return }
                for shot in newShots {
                    newHandler?(shot)
                }
                listHandler?(allShots)
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
