import Foundation
import Testing
@testable import Astroshots

struct StreamGroupingTests {
    @Test
    func contiguousWorktreesRemainSeparateWhenTheSameWorktreeReturns() {
        let shots = [
            makeShot(path: "/tmp/alpha/.astroshot/run/0004.png", worktree: "alpha"),
            makeShot(path: "/tmp/alpha/.astroshot/run/0003.png", worktree: "alpha"),
            makeShot(path: "/tmp/beta/.astroshot/run/0002.png", worktree: "beta"),
            makeShot(path: "/tmp/alpha/.astroshot/run/0001.png", worktree: "alpha"),
        ]

        let groups = StreamGrouping.contiguousGroups(shots)

        #expect(groups.map(\.worktree) == ["alpha", "beta", "alpha"])
        #expect(groups.map { $0.allShots.count } == [2, 1, 1])
        #expect(groups[0].id == shots[1].path)
        #expect(groups[2].id == shots[3].path)
    }

    @Test @MainActor
    func bulkSeenPersistsEveryCandidateAndUpdatesTheStream() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-bulk-seen-\(UUID().uuidString)",
                isDirectory: true
            )
        let alphaFeature = root.appendingPathComponent(
            "alpha/.astroshot/run",
            isDirectory: true
        )
        let betaFeature = root.appendingPathComponent(
            "beta/.astroshot/run",
            isDirectory: true
        )
        try FileManager.default.createDirectory(
            at: alphaFeature,
            withIntermediateDirectories: true
        )
        try FileManager.default.createDirectory(
            at: betaFeature,
            withIntermediateDirectories: true
        )
        let alphaImage = alphaFeature.appendingPathComponent("0001-alpha.png")
        let betaImage = betaFeature.appendingPathComponent("0001-beta.png")
        try Data("alpha image".utf8).write(to: alphaImage)
        try Data("beta image".utf8).write(to: betaImage)
        defer { try? FileManager.default.removeItem(at: root) }

        let defaults = TestDefaults()
        let preferences = Preferences(defaults: defaults)
        preferences.watchRootPaths = [root.path]
        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [root],
                cacheFileURL: root.appendingPathComponent("shot-index.json")
            )
        )
        defer { watcher.stop() }
        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: false
        )
        let alpha = try #require(watcher.makeShot(at: alphaImage.path))
        let beta = try #require(watcher.makeShot(at: betaImage.path))
        state.shots = [alpha, beta]

        await state.markAllSeen([alpha, beta])

        #expect(state.shots.allSatisfy { $0.review?.state == .seen })
        let store = ReviewStore()
        let alphaReview = try await store.review(
            forImage: alphaImage,
            featureDirectory: alphaFeature
        )
        let betaReview = try await store.review(
            forImage: betaImage,
            featureDirectory: betaFeature
        )
        #expect(alphaReview.state == .seen)
        #expect(betaReview.state == .seen)
    }

    @Test @MainActor
    func bulkSeenContinuesAfterAnIndividualWriteFails() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-partial-bulk-seen-\(UUID().uuidString)",
                isDirectory: true
            )
        let feature = root.appendingPathComponent(
            "alpha/.astroshot/run",
            isDirectory: true
        )
        try FileManager.default.createDirectory(
            at: feature,
            withIntermediateDirectories: true
        )
        let missingImage = feature.appendingPathComponent("0002-missing.png")
        let validImage = feature.appendingPathComponent("0001-valid.png")
        try Data("valid image".utf8).write(to: validImage)
        defer { try? FileManager.default.removeItem(at: root) }

        let defaults = TestDefaults()
        let preferences = Preferences(defaults: defaults)
        preferences.watchRootPaths = [root.path]
        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [root],
                cacheFileURL: root.appendingPathComponent("shot-index.json")
            )
        )
        defer { watcher.stop() }
        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: false
        )
        let missing = makeShot(path: missingImage.path, worktree: "alpha")
        let valid = try #require(watcher.makeShot(at: validImage.path))
        state.shots = [missing, valid]

        await state.markAllSeen([missing, valid])

        #expect(state.shots[0].review == nil)
        #expect(state.shots[1].review?.state == .seen)
        let persisted = try await ReviewStore().review(
            forImage: validImage,
            featureDirectory: feature
        )
        #expect(persisted.state == .seen)
    }

    private func makeShot(path: String, worktree: String) -> Shot {
        Shot(
            path: path,
            worktree: worktree,
            worktreePath: "/tmp/\(worktree)",
            feature: "run",
            fileName: URL(fileURLWithPath: path).lastPathComponent,
            sequence: nil,
            slug: "shot",
            title: "Shot",
            description: "",
            url: nil,
            runID: nil,
            status: nil,
            capturedAt: Date()
        )
    }
}
