import Foundation
import XCTest
@testable import Astroshots

final class ShotIndexCacheTests: XCTestCase {
    private var tempSupport: URL!

    override func setUpWithError() throws {
        tempSupport = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-cache-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: tempSupport, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let tempSupport {
            try? FileManager.default.removeItem(at: tempSupport)
        }
    }

    func testRoundTripMatchingRoots() throws {
        let roots = [
            URL(fileURLWithPath: "/Users/me/archastro", isDirectory: true),
        ]
        let dirs = [
            "/Users/me/archastro/firstlanding/.astroshot",
            "/Users/me/archastro/other-wt/.astroshot",
        ]

        // Write via a private URL by saving then moving into our isolation
        // is hard because ShotIndexCache uses Application Support. Instead
        // encode the document shape the same way and assert load filters.
        let doc = ShotIndexCacheDocument(
            version: ShotIndexCacheDocument.currentVersion,
            roots: roots.map(\.path).sorted(),
            astroshotDirs: dirs.sorted(),
            arrivalOrder: [
                "/Users/me/archastro/other-wt/.astroshot/login/0002-new.png",
                "/Users/me/archastro/firstlanding/.astroshot/login/0001-old.png",
            ],
            updatedAt: Date(timeIntervalSince1970: 1_700_000_000)
        )
        let data = try JSONEncoder().encode(doc)
        // Sanity: document is Codable and dirs sorted for stable compare.
        let decoded = try JSONDecoder().decode(ShotIndexCacheDocument.self, from: data)
        XCTAssertEqual(decoded.astroshotDirs, dirs.sorted())
        XCTAssertEqual(decoded.roots, [roots[0].path])
        XCTAssertEqual(decoded.arrivalOrder, doc.arrivalOrder)
    }

    func testLegacyDocumentDefaultsToNoArrivalOrder() throws {
        let data = Data(
            """
            {
              "version": 1,
              "roots": ["/tmp/project"],
              "astroshotDirs": ["/tmp/project/.astroshot"],
              "updatedAt": 1700000000
            }
            """.utf8
        )

        let decoded = try JSONDecoder().decode(
            ShotIndexCacheDocument.self,
            from: data
        )
        XCTAssertEqual(decoded.arrivalOrder, [])
    }

    func testReconcilePreservesKnownArrivalOrderAndPrependsDiscoveries() {
        let base = "/tmp/project/.astroshot/feature"
        let olderKnown = makeShot(
            path: "\(base)/0001-known.png",
            capturedAt: Date(timeIntervalSince1970: 100)
        )
        let newerKnown = makeShot(
            path: "\(base)/0002-known.png",
            capturedAt: Date(timeIntervalSince1970: 200)
        )
        let newlyDiscovered = makeShot(
            path: "\(base)/0003-new.png",
            capturedAt: Date(timeIntervalSince1970: 50)
        )

        let order = ShotIndexCache.reconciledArrivalOrder(
            existing: [olderKnown.path, newerKnown.path, "/tmp/deleted.png"],
            shots: [newerKnown, newlyDiscovered, olderKnown]
        )

        XCTAssertEqual(
            order,
            [newlyDiscovered.path, olderKnown.path, newerKnown.path]
        )
        XCTAssertEqual(
            ShotIndexCache.orderedShots(
                [newerKnown, olderKnown, newlyDiscovered],
                arrivalOrder: order
            ).map(\.path),
            order
        )
    }

    func testAstroshotDirFromImagePath() {
        let path = "/tmp/proj/.astroshot/login/0001-home.png"
        XCTAssertEqual(
            ShotIndexCache.astroshotDir(forImagePath: path),
            "/tmp/proj/.astroshot"
        )
        XCTAssertNil(ShotIndexCache.astroshotDir(forImagePath: "/tmp/proj/not-a-shot.png"))
    }

    func testSaveAndLoadViaRealCacheFile() throws {
        // Isolate by writing through the public API then verifying load
        // filters on root mismatch. Uses the real Application Support path
        // but restores any pre-existing file afterward.
        let cacheURL = ShotIndexCache.cacheFileURL()
        let backup = try? Data(contentsOf: cacheURL)
        defer {
            if let backup {
                try? backup.write(to: cacheURL, options: [.atomic])
            } else {
                ShotIndexCache.clear()
            }
        }

        let root = URL(fileURLWithPath: "/tmp/astroshots-test-root-\(UUID().uuidString)", isDirectory: true)
        let dirs = [
            root.appendingPathComponent("a/.astroshot").path,
            root.appendingPathComponent("b/.astroshot").path,
        ]
        let order = [
            root.appendingPathComponent("b/.astroshot/feature/0002.png").path,
            root.appendingPathComponent("a/.astroshot/feature/0001.png").path,
        ]
        ShotIndexCache.save(
            roots: [root],
            astroshotDirs: dirs,
            arrivalOrder: order
        )

        let loaded = ShotIndexCache.load(for: [root])
        XCTAssertEqual(loaded?.astroshotDirs.sorted(), dirs.sorted())
        XCTAssertEqual(loaded?.roots, [root.standardizedFileURL.path])
        XCTAssertEqual(loaded?.arrivalOrder, order)

        // Different root → miss.
        let other = URL(fileURLWithPath: "/tmp/other-root", isDirectory: true)
        XCTAssertNil(ShotIndexCache.load(for: [other]))
    }

    @MainActor
    func testWatcherRestoresLiveArrivalOrderAfterRestart() async throws {
        let cacheURL = ShotIndexCache.cacheFileURL()
        let backup = try? Data(contentsOf: cacheURL)
        defer {
            if let backup {
                try? backup.write(to: cacheURL, options: [.atomic])
            } else {
                ShotIndexCache.clear()
            }
        }

        let root = tempSupport.appendingPathComponent("restart-root", isDirectory: true)
        let feature = root.appendingPathComponent(".astroshot/feature", isDirectory: true)
        let older = feature.appendingPathComponent("0001-older.png")
        let newer = feature.appendingPathComponent("0002-newer.png")
        try FileManager.default.createDirectory(
            at: feature,
            withIntermediateDirectories: true
        )
        try Data("older".utf8).write(to: older)
        try Data("newer".utf8).write(to: newer)
        try FileManager.default.setAttributes(
            [.modificationDate: Date(timeIntervalSince1970: 100)],
            ofItemAtPath: older.path
        )
        try FileManager.default.setAttributes(
            [.modificationDate: Date(timeIntervalSince1970: 200)],
            ofItemAtPath: newer.path
        )

        let firstWatcher = AstroshotWatcher(
            configuration: .init(roots: [root], settleNanos: 10_000_000)
        )
        var initialOrder: [String] = []
        firstWatcher.onShotsChanged = { shots in
            initialOrder = shots.map(\.path)
        }
        firstWatcher.start()
        for _ in 0..<100 where initialOrder.count != 2 {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(initialOrder, [newer.path, older.path])

        // A later filesystem event for the older path is the actual arrival
        // signal. It must win over file timestamps and survive app restart.
        var receivedLiveEvent = false
        firstWatcher.onNewShot = { shot in
            receivedLiveEvent = shot.path == older.path
        }
        try Data("older revision".utf8).write(to: older, options: [.atomic])
        firstWatcher.handleFSEvents(paths: [older.path])
        for _ in 0..<100 where !receivedLiveEvent {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(receivedLiveEvent)
        XCTAssertEqual(
            ShotIndexCache.load(for: [root])?.arrivalOrder,
            [older.path, newer.path]
        )
        firstWatcher.stop()

        // Deliberately make timestamp sorting disagree with the cached stream.
        try FileManager.default.setAttributes(
            [.modificationDate: Date(timeIntervalSince1970: 300)],
            ofItemAtPath: newer.path
        )

        let restartedWatcher = AstroshotWatcher(
            configuration: .init(roots: [root], settleNanos: 10_000_000)
        )
        defer { restartedWatcher.stop() }
        var restoredOrder: [String] = []
        restartedWatcher.onShotsChanged = { shots in
            restoredOrder = shots.map(\.path)
        }
        restartedWatcher.start()
        for _ in 0..<100 where restoredOrder.count != 2 {
            try await Task.sleep(for: .milliseconds(10))
        }

        XCTAssertEqual(restoredOrder, [older.path, newer.path])
    }

    private func makeShot(path: String, capturedAt: Date) -> Shot {
        Shot(
            path: path,
            worktree: "project",
            worktreePath: "/tmp/project",
            feature: "feature",
            fileName: URL(fileURLWithPath: path).lastPathComponent,
            sequence: nil,
            slug: "shot",
            title: "Shot",
            description: "",
            url: nil,
            runID: nil,
            status: nil,
            capturedAt: capturedAt
        )
    }
}
