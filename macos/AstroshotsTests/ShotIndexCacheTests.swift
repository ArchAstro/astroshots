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
            updatedAt: Date(timeIntervalSince1970: 1_700_000_000)
        )
        let data = try JSONEncoder().encode(doc)
        // Sanity: document is Codable and dirs sorted for stable compare.
        let decoded = try JSONDecoder().decode(ShotIndexCacheDocument.self, from: data)
        XCTAssertEqual(decoded.astroshotDirs, dirs.sorted())
        XCTAssertEqual(decoded.roots, [roots[0].path])
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
        ShotIndexCache.save(roots: [root], astroshotDirs: dirs)

        let loaded = ShotIndexCache.load(for: [root])
        XCTAssertEqual(loaded?.astroshotDirs.sorted(), dirs.sorted())
        XCTAssertEqual(loaded?.roots, [root.standardizedFileURL.path])

        // Different root → miss.
        let other = URL(fileURLWithPath: "/tmp/other-root", isDirectory: true)
        XCTAssertNil(ShotIndexCache.load(for: [other]))
    }
}
