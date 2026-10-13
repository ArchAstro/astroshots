import CoreServices
import Foundation
import Testing
@testable import Astroshots

@MainActor
struct WatcherResourceTests {
    private func workspace() throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-resources-\(UUID().uuidString)")
            .resolvingSymlinksInPath()
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return root
    }

    @discardableResult
    private func capture(_ root: URL, _ relative: String) throws -> URL {
        let image = root.appendingPathComponent(relative)
        try FileManager.default.createDirectory(
            at: image.deletingLastPathComponent(), withIntermediateDirectories: true
        )
        try Data("synthetic screenshot".utf8).write(to: image)
        return image
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        for _ in 0..<500 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(condition(), "Watcher did not deliver the expected filesystem change")
    }

    @Test
    func discoverySkipsGeneratedDirectoriesAndDirectorySymlinks() throws {
        let root = try workspace()
        let outside = try workspace()
        defer {
            try? FileManager.default.removeItem(at: root)
            try? FileManager.default.removeItem(at: outside)
        }
        let visible = try capture(root, "project/.astroshot/feature/visible.png")
        for excluded in ["_build", "deps", "node_modules", "target"] {
            try capture(root, "project/\(excluded)/nested/.astroshot/feature/ignored.png")
        }
        try capture(outside, ".astroshot/external/linked.png")
        try FileManager.default.createSymbolicLink(
            at: root.appendingPathComponent("linked"), withDestinationURL: outside
        )
        try FileManager.default.createSymbolicLink(
            at: root.appendingPathComponent("cycle"), withDestinationURL: root
        )
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], cacheFileURL: root.appendingPathComponent("index.json")
        ))
        #expect(watcher.scanAll().map(\.path) == [visible.path])
    }

    @Test
    func deletedCaptureRefreshesOnlyItsFeatureAndPersistsRemoval() async throws {
        let root = try workspace()
        defer { try? FileManager.default.removeItem(at: root) }
        let deleted = try capture(root, "one/.astroshot/feature/deleted.png")
        let retained = try capture(root, "two/.astroshot/feature/retained.png")
        let cache = root.appendingPathComponent("index.json")
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], settleNanos: 0, cacheFileURL: cache
        ))
        defer { watcher.stop() }
        var arrivals: [String] = []
        var refreshed: [String: [String]] = [:]
        var scans = 0
        watcher.onNewShot = { arrivals.append($0.path) }
        watcher.onFeatureShotsChanged = { refreshed[$0] = $1.map(\.path) }
        watcher.onScanStateChanged = { if $0 { scans += 1 } }
        watcher.handleFSEvents(paths: [deleted.path, retained.path])
        try await waitUntil { arrivals.count == 2 }

        // A silent capture elsewhere is a sentinel: deleting this frame must
        // not discover unrelated trees or replace the global stream.
        try capture(root, "unrelated/.astroshot/feature/not-yet-announced.png")
        try FileManager.default.removeItem(at: deleted)
        watcher.handleFSEvents(paths: [deleted.path])
        try await waitUntil { refreshed[deleted.deletingLastPathComponent().path] != nil }
        #expect(refreshed[deleted.deletingLastPathComponent().path] == [])
        #expect(arrivals.count == 2)
        #expect(scans == 0)
        #expect(ShotIndexCache.load(for: [root], cacheFileURL: cache)?.arrivalOrder == [retained.path])
    }

    @Test
    func localRecoveryDiscoversOnlyAffectedSubtree() async throws {
        let root = try workspace()
        defer { try? FileManager.default.removeItem(at: root) }
        let retained = try capture(root, "other/.astroshot/feature/retained.png")
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], settleNanos: 0, cacheFileURL: root.appendingPathComponent("index.json")
        ))
        defer { watcher.stop() }
        var arrivals: [String] = []
        var stream: [String] = []
        watcher.onNewShot = { arrivals.append($0.path) }
        watcher.onShotsChanged = { stream = $0.map(\.path) }
        watcher.handleFSEvents(paths: [retained.path])
        try await waitUntil { arrivals == [retained.path] }
        let recovered = try capture(root, "affected/.astroshot/feature/recovered.png")
        try capture(root, "other/.astroshot/feature/silent.png")

        // Inject a coalesced OS recovery event, then cross the real scanner,
        // index persistence, and main-queue delivery boundaries.
        watcher.handleFSEvents(
            paths: [root.appendingPathComponent("affected").path],
            flags: [FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs)]
        )
        try await waitUntil { stream.contains(recovered.path) }
        #expect(Set(stream) == Set([retained.path, recovered.path]))
        #expect(Set(arrivals) == Set([retained.path, recovered.path]))
    }

    @Test
    func buildRecoveryIsIgnoredButDroppedHistoryStillReconcilesAllRoots() async throws {
        let root = try workspace()
        defer { try? FileManager.default.removeItem(at: root) }
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], settleNanos: 0, cacheFileURL: root.appendingPathComponent("index.json")
        ))
        defer { watcher.stop() }
        var scans = 0
        var arrivals: [String] = []
        var stream: [String] = []
        watcher.onScanStateChanged = { if $0 { scans += 1 } }
        watcher.onNewShot = { arrivals.append($0.path) }
        watcher.onShotsChanged = { stream = $0.map(\.path) }
        let ignored = try capture(root, "project/_build/.astroshot/feature/build.png")
        watcher.handleFSEvents(
            paths: [ignored.deletingLastPathComponent().path],
            flags: [FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs)]
        )
        // A delivered ordinary capture provides a queue checkpoint after the
        // ignored recovery event, without relying on a fixed sleep.
        let visible = try capture(root, "project/.astroshot/feature/visible.png")
        watcher.handleFSEvents(paths: [visible.path])
        try await waitUntil { arrivals.contains(visible.path) }
        #expect(scans == 0)
        let missed = try capture(root, "another/.astroshot/feature/missed.png")
        watcher.handleFSEvents(
            paths: [root.appendingPathComponent("project/_build").path],
            flags: [FSEventStreamEventFlags(kFSEventStreamEventFlagUserDropped)]
        )
        try await waitUntil { stream.contains(missed.path) }
        #expect(Set(stream) == Set([visible.path, missed.path]))
        #expect(scans == 1)
    }

    @Test
    func liveFilesystemCaptureAndDeletionUpdateStream() async throws {
        let root = try workspace()
        defer { try? FileManager.default.removeItem(at: root) }
        let initial = try capture(root, "project/.astroshot/feature/initial.png")
        let astroshot = root.appendingPathComponent("project/.astroshot")
        let cache = root.appendingPathComponent("index.json")
        // Warm startup uses a known screenshot tree. FSEvents itself, rather
        // than an injected callback, must carry subsequent writes and deletion.
        ShotIndexCache.save(
            roots: [root], astroshotDirs: [astroshot.path],
            arrivalOrder: [initial.path], lastEventID: FSEventsGetCurrentEventId(),
            cacheFileURL: cache
        )
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], settleNanos: 0, cacheFileURL: cache
        ))
        defer { watcher.stop() }
        var stream: [String] = []
        var startupFinished = false
        var scanning = false
        watcher.onShotsChanged = { stream = $0.map(\.path) }
        watcher.onNewShot = { if !stream.contains($0.path) { stream.append($0.path) } }
        watcher.onFeatureShotsChanged = { directory, shots in
            stream.removeAll { $0.hasPrefix(directory + "/") }
            stream.append(contentsOf: shots.map(\.path))
        }
        watcher.onScanStateChanged = {
            scanning = $0
            if !$0 { startupFinished = true }
        }
        watcher.start()
        try await waitUntil { startupFinished && stream == [initial.path] }

        // Cross the writer -> macOS FSEvents -> watcher -> stream boundary.
        let live = try capture(root, "project/.astroshot/feature/live.png")
        try await waitUntil { stream.contains(live.path) && !scanning }
        try FileManager.default.removeItem(at: live)
        try await waitUntil { !stream.contains(live.path) }
        #expect(stream == [initial.path])
        #expect(ShotIndexCache.load(for: [root], cacheFileURL: cache)?.arrivalOrder == [initial.path])
    }

    @Test
    func directoryRenameAndRemovalReconcileOnlyAffectedProjects() async throws {
        let root = try workspace()
        defer { try? FileManager.default.removeItem(at: root) }
        let original = try capture(root, "original/.astroshot/feature/frame.png")
        let retained = try capture(root, "other/.astroshot/feature/retained.png")
        let cache = root.appendingPathComponent("index.json")
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], settleNanos: 0, cacheFileURL: cache
        ))
        defer { watcher.stop() }
        var arrivals: [String] = []
        var stream: [String] = []
        watcher.onNewShot = { arrivals.append($0.path) }
        watcher.onShotsChanged = { stream = $0.map(\.path) }
        watcher.handleFSEvents(paths: [original.path, retained.path])
        try await waitUntil { arrivals.count == 2 }
        try capture(root, "unrelated/.astroshot/feature/silent.png")

        // A rename produces old and new directory paths. Reconcile both,
        // removing old identities while discovering the moved capture.
        let oldDirectory = root.appendingPathComponent("original")
        let newDirectory = root.appendingPathComponent("moved")
        try FileManager.default.moveItem(at: oldDirectory, to: newDirectory)
        let renamed = newDirectory.appendingPathComponent(".astroshot/feature/frame.png")
        let renameFlag = FSEventStreamEventFlags(
            kFSEventStreamEventFlagItemIsDir | kFSEventStreamEventFlagItemRenamed
        )
        watcher.handleFSEvents(paths: [oldDirectory.path, newDirectory.path], flags: [renameFlag, renameFlag])
        try await waitUntil { stream.contains(renamed.path) }
        #expect(Set(stream) == Set([retained.path, renamed.path]))
        #expect(!arrivals.contains { $0.contains("unrelated") })

        try FileManager.default.removeItem(at: newDirectory)
        watcher.handleFSEvents(paths: [newDirectory.path], flags: [FSEventStreamEventFlags(
            kFSEventStreamEventFlagItemIsDir | kFSEventStreamEventFlagItemRemoved
        )])
        try await waitUntil { stream == [retained.path] }
        #expect(ShotIndexCache.load(for: [root], cacheFileURL: cache)?.astroshotDirs == [
            root.appendingPathComponent("other/.astroshot").path,
        ])
    }

    @Test
    func scopedRecoveryPreservesStoriesOutsideItsSubtree() async throws {
        let root = try workspace()
        defer { try? FileManager.default.removeItem(at: root) }
        let retained = try capture(root, "other/.astroshot/feature/retained.png")
        let story = root.appendingPathComponent("other/.astroshot/stories/checkout")
        try FileManager.default.createDirectory(at: story, withIntermediateDirectories: true)
        try Data("# Checkout\nExisting story".utf8).write(to: story.appendingPathComponent("prompt.md"))
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], settleNanos: 0, cacheFileURL: root.appendingPathComponent("index.json")
        ))
        defer { watcher.stop() }
        var arrivals: [String] = []
        var storyCount: Int?
        watcher.onNewShot = { arrivals.append($0.path) }
        watcher.onFrictionLogsChanged = { storyCount = $0.count }
        watcher.handleFSEvents(paths: [retained.path])
        try await waitUntil { arrivals.contains(retained.path) }
        let recovered = try capture(root, "affected/.astroshot/feature/recovered.png")
        watcher.handleFSEvents(
            paths: [root.appendingPathComponent("affected").path],
            flags: [FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs)]
        )
        try await waitUntil { arrivals.contains(recovered.path) && storyCount != nil }
        #expect(storyCount == 1)
    }

    @Test
    func droppedHistoryDuringScopedScanRetainsFullRecoveryRequest() async throws {
        let root = try workspace()
        defer { try? FileManager.default.removeItem(at: root) }
        let local = try capture(root, "local/.astroshot/feature/local.png")
        let elsewhere = try capture(root, "elsewhere/.astroshot/feature/missed.png")
        let scanQueue = DispatchQueue(label: "astroshots-test-paused-discovery")
        let watcher = AstroshotWatcher(configuration: .init(
            roots: [root], settleNanos: 0, cacheFileURL: root.appendingPathComponent("index.json")
        ), scanQueue: scanQueue)
        defer { watcher.stop() }
        var stream: [String] = []
        watcher.onShotsChanged = { stream = $0.map(\.path) }

        // Hold the worker before traversal: the second request is guaranteed
        // to arrive while scoped discovery is in flight, without a timing race.
        scanQueue.suspend()
        watcher.handleFSEvents(
            paths: [root.appendingPathComponent("local").path],
            flags: [FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs)]
        )
        watcher.handleFSEvents(
            paths: [], flags: [FSEventStreamEventFlags(kFSEventStreamEventFlagKernelDropped)]
        )
        scanQueue.resume()
        try await waitUntil { stream.contains(elsewhere.path) }
        #expect(Set(stream) == Set([local.path, elsewhere.path]))
    }
}
