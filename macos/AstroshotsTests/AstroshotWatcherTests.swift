import Foundation
import Testing
@testable import Astroshots

struct AstroshotWatcherTests {
    @Test @MainActor
    func appStateStreamsAndReconfiguresTwoUnrelatedWatchRoots() async throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-multiple-roots-\(UUID().uuidString)",
                isDirectory: true
            )
        let firstRoot = temporary.appendingPathComponent(
            "first-workspace",
            isDirectory: true
        )
        let secondRoot = temporary.appendingPathComponent(
            "second-workspace",
            isDirectory: true
        )
        let firstFeature = firstRoot.appendingPathComponent(
            ".astroshot/first-feature",
            isDirectory: true
        )
        let secondFeature = secondRoot.appendingPathComponent(
            ".astroshot/second-feature",
            isDirectory: true
        )
        try FileManager.default.createDirectory(
            at: firstFeature,
            withIntermediateDirectories: true
        )
        try FileManager.default.createDirectory(
            at: secondFeature,
            withIntermediateDirectories: true
        )
        try Data("first screenshot".utf8).write(
            to: firstFeature.appendingPathComponent("0001-first.png")
        )
        try Data("second screenshot".utf8).write(
            to: secondFeature.appendingPathComponent("0001-second.png")
        )
        defer { try? FileManager.default.removeItem(at: temporary) }

        let suiteName = "astroshots-multiple-roots-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)
        preferences.watchRootPaths = [firstRoot.path]
        let cacheURL = temporary.appendingPathComponent("shot-index.json")
        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [firstRoot],
                cacheFileURL: cacheURL
            )
        )
        defer { watcher.stop() }

        // Cross the persisted configuration and real recursive watcher
        // boundaries with the initial top-level folder.
        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: false
        )
        state.startWatching()
        for _ in 0..<100 where state.shots.map(\.worktree) != ["first-workspace"] {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(state.shots.map(\.worktree) == ["first-workspace"])

        // Add an unrelated root through the same AppState action used by
        // Settings, then observe both filesystems merged into one stream.
        state.addWatchRoots([secondRoot.path])
        for _ in 0..<100 where state.shots.count != 2 {
            try await Task.sleep(for: .milliseconds(20))
        }

        #expect(Set(state.shots.map(\.worktree)) == [
            "first-workspace",
            "second-workspace",
        ])
        #expect(state.watchRootPaths == [firstRoot.path, secondRoot.path])

        // Cross the live-event and partial-write-settling boundary after both
        // roots are active, proving either tree can update the unified stream.
        let firstLiveImage = firstFeature.appendingPathComponent("0002-first-live.png")
        let secondLiveImage = secondFeature.appendingPathComponent("0002-second-live.png")
        try Data("first live screenshot".utf8).write(to: firstLiveImage)
        try Data("second live screenshot".utf8).write(to: secondLiveImage)
        watcher.handleFSEvents(paths: [firstLiveImage.path, secondLiveImage.path])
        for _ in 0..<100 where state.shots.count != 4 {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(
            Dictionary(grouping: state.shots, by: \.worktree)
                .mapValues(\.count)
                == [
                    "first-workspace": 2,
                    "second-workspace": 2,
                ]
        )

        // Removing a top-level root must immediately evict its existing frames,
        // then remain correct after the replacement full scan completes.
        state.removeWatchRoot(firstRoot.path)
        #expect(Set(state.shots.map(\.worktree)) == ["second-workspace"])
        #expect(state.shots.count == 2)
        for _ in 0..<100 where state.isScanning {
            try await Task.sleep(for: .milliseconds(20))
        }

        #expect(Set(state.shots.map(\.worktree)) == ["second-workspace"])
        #expect(state.shots.count == 2)
        #expect(preferences.watchRootPaths == [secondRoot.path])
    }

    @Test @MainActor
    func reviewAndImageEventsRefreshTheOpenReviewState() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-live-review-\(UUID().uuidString)", isDirectory: true)
        let feature = root.appendingPathComponent(".astroshot/review", isDirectory: true)
        let image = feature.appendingPathComponent("0001-settings.png")
        try FileManager.default.createDirectory(
            at: feature,
            withIntermediateDirectories: true
        )
        try Data("image revision one".utf8).write(to: image)
        defer { try? FileManager.default.removeItem(at: root) }

        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [root],
                settleNanos: 20_000_000,
                cacheFileURL: root.appendingPathComponent("shot-index.json")
            )
        )
        defer { watcher.stop() }
        let store = ReviewStore()
        _ = try await store.markSeen(
            forImage: image,
            featureDirectory: feature,
            runID: "review-run-1"
        )

        // Sidecar boundary: the same targeted refresh used by FSEvents must
        // rehydrate the feature with its persisted Seen state.
        var refreshedShots: [Shot]?
        watcher.onFeatureShotsChanged = { _, shots in
            refreshedShots = shots
        }
        watcher.handleFSEvents(
            paths: [feature.appendingPathComponent(ReviewStore.sidecarFileName).path]
        )
        for _ in 0..<40 where refreshedShots == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(refreshedShots?.first?.review?.state == .seen)

        // Image boundary: replacing bytes at the same path must emit a new
        // snapshot whose acknowledgement is stale while feedback remains.
        var replacement: Shot?
        watcher.onNewShot = { shot in replacement = shot }
        try Data("image revision two".utf8).write(to: image, options: .atomic)
        watcher.handleFSEvents(paths: [image.path])
        for _ in 0..<40 where replacement == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(replacement?.review?.state == .pending)
        #expect(replacement?.review?.isStale == true)
    }

    @Test @MainActor
    func rootReplacementRejectsAnAlreadyQueuedOldGenerationDelivery() async throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-delivery-\(UUID().uuidString)", isDirectory: true)
        let newRoot = temporary.appendingPathComponent("new", isDirectory: true)
        try FileManager.default.createDirectory(
            at: newRoot,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: temporary) }

        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [temporary],
                cacheFileURL: temporary.appendingPathComponent("shot-index.json")
            )
        )
        var emitted = false
        watcher.onNewShot = { _ in emitted = true }
        let shot = Shot(
            path: temporary.appendingPathComponent(
                ".astroshot/feature/0001-old.png"
            ).path,
            worktree: "old",
            worktreePath: temporary.path,
            feature: "feature",
            fileName: "0001-old.png",
            sequence: "0001",
            slug: "old",
            title: "Old",
            description: "",
            url: nil,
            runID: nil,
            status: nil,
            capturedAt: Date()
        )

        // Queue delivery for generation zero while this MainActor test holds
        // the queue, then invalidate it before allowing delivery.
        watcher.emitNew(shot, generation: 0)
        watcher.updateRoots([newRoot])
        await Task.yield()
        try await Task.sleep(for: .milliseconds(50))
        watcher.stop()

        #expect(!emitted)
    }

    @Test @MainActor
    func replacingRootsCancelsSettledIngestFromOldRoot() async throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-watcher-\(UUID().uuidString)", isDirectory: true)
        let oldRoot = temporary.appendingPathComponent("old", isDirectory: true)
        let newRoot = temporary.appendingPathComponent("new", isDirectory: true)
        let oldFeature = oldRoot.appendingPathComponent(".astroshot/feature", isDirectory: true)
        let oldImage = oldFeature.appendingPathComponent("0001-old.png")
        try FileManager.default.createDirectory(
            at: oldFeature,
            withIntermediateDirectories: true
        )
        try FileManager.default.createDirectory(
            at: newRoot,
            withIntermediateDirectories: true
        )
        try Data("synthetic image bytes".utf8).write(to: oldImage)
        defer { try? FileManager.default.removeItem(at: temporary) }

        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [oldRoot],
                settleNanos: 200_000_000,
                cacheFileURL: temporary.appendingPathComponent("shot-index.json")
            )
        )
        var emittedPaths: [String] = []
        watcher.onNewShot = { shot in
            emittedPaths.append(shot.path)
        }

        // Cross the event boundary, then replace the watched root before the
        // partial-write settling delay expires.
        watcher.handleFSEvents(paths: [oldImage.path])
        watcher.updateRoots([newRoot])
        try await Task.sleep(for: .milliseconds(450))
        watcher.stop()

        // Work from the prior generation must not repopulate AppState.
        #expect(!emittedPaths.contains(oldImage.path))
    }

    @Test @MainActor
    func recoveryScanEmitsDiscoveredFramesAsNewShots() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-recovery-overlay-\(UUID().uuidString)",
                isDirectory: true
            )
        let feature = root.appendingPathComponent(
            ".astroshot/recovered",
            isDirectory: true
        )
        let image = feature.appendingPathComponent("0001-recovered.png")
        try FileManager.default.createDirectory(
            at: feature,
            withIntermediateDirectories: true
        )
        try Data("recovered image".utf8).write(to: image)
        defer { try? FileManager.default.removeItem(at: root) }

        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [root],
                settleNanos: 10_000_000,
                cacheFileURL: root.appendingPathComponent("shot-index.json")
            )
        )
        defer { watcher.stop() }
        var newPaths: [String] = []
        var fullPaths: [String] = []
        watcher.onNewShot = { shot in
            newPaths.append(shot.path)
        }
        watcher.onShotsChanged = { shots in
            fullPaths = shots.map(\.path)
        }

        // A dropped event batch may contain no usable paths. Reconciliation
        // must still surface captures as arrivals, not just bulk-list rows.
        watcher.handleFSEvents(
            paths: [],
            latestEventID: 42,
            requiresFullScan: true
        )
        for _ in 0..<100 where
            !newPaths.contains(image.path) || !fullPaths.contains(image.path)
        {
            try await Task.sleep(for: .milliseconds(10))
        }

        #expect(newPaths.contains(image.path))
        #expect(fullPaths.contains(image.path))
    }

    @Test @MainActor
    func manifestRefreshEmitsFramesMissedByImageEvents() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-manifest-arrival-\(UUID().uuidString)",
                isDirectory: true
            )
        let feature = root.appendingPathComponent(
            ".astroshot/atomic-capture",
            isDirectory: true
        )
        let image = feature.appendingPathComponent("0001-atomic.png")
        let manifest = feature.appendingPathComponent("manifest.json")
        try FileManager.default.createDirectory(
            at: feature,
            withIntermediateDirectories: true
        )
        try Data("atomic image".utf8).write(to: image)
        try Data(
            """
            {"version":1,"feature":"atomic-capture","shots":[{"file":"0001-atomic.png"}]}
            """.utf8
        ).write(to: manifest)
        defer { try? FileManager.default.removeItem(at: root) }

        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [root],
                settleNanos: 10_000_000,
                cacheFileURL: root.appendingPathComponent("shot-index.json")
            )
        )
        defer { watcher.stop() }
        let suiteName = "astroshots-manifest-arrival-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)
        preferences.watchRootPaths = [root.path]
        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: false
        )

        // Atomic producers always replace the manifest after publishing a
        // frame. That targeted refresh is the fallback if an image event was
        // coalesced or dropped.
        watcher.handleFSEvents(paths: [manifest.path])
        for _ in 0..<100 where !state.shots.contains(where: { $0.path == image.path }) {
            try await Task.sleep(for: .milliseconds(10))
        }

        #expect(state.shots.map(\.path) == [image.path])
        #expect(state.unreadCount == 1)

        // If the coalesced image event arrives after the manifest refresh, it
        // must not increment unread or flash a second overlay for the same file.
        try Data("atomic image revision".utf8).write(to: image, options: .atomic)
        watcher.handleFSEvents(paths: [image.path])
        try await Task.sleep(for: .milliseconds(50))

        #expect(state.shots.map(\.path) == [image.path])
        #expect(state.unreadCount == 1)
    }
}
