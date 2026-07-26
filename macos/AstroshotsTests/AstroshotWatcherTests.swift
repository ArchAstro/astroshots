import Foundation
import Testing
@testable import Astroshots

struct AstroshotWatcherTests {
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
            configuration: .init(roots: [root], settleNanos: 20_000_000)
        )
        defer { watcher.stop() }
        let store = ReviewStore()
        _ = try await store.setDecision(
            .approved,
            forImage: image,
            featureDirectory: feature,
            runID: "review-run-1"
        )

        // Sidecar boundary: the same targeted refresh used by FSEvents must
        // rehydrate the feature with its persisted approval.
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
        #expect(refreshedShots?.first?.review?.state == .approved)

        // Image boundary: replacing bytes at the same path must emit a new
        // snapshot whose approval is stale while the feedback record remains.
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
            configuration: .init(roots: [temporary])
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
                settleNanos: 200_000_000
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
}
