import AppKit
import Foundation
import Testing
@testable import Astroshots

struct ShotThumbnailCacheTests {
    private func writePNG(width: Int, height: Int, to url: URL) throws {
        let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        )!
        try rep.representation(using: .png, properties: [:])!.write(to: url)
    }

    private func tempPNG(width: Int, height: Int) throws -> String {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("thumb-\(UUID().uuidString).png")
        try writePNG(width: width, height: height, to: url)
        return url.path
    }

    private let rowSize = CGSize(width: 88, height: 56)

    @Test
    func downsamplesLargeImageToRowSize() throws {
        let path = try tempPNG(width: 3000, height: 2000)
        defer { try? FileManager.default.removeItem(atPath: path) }
        let cache = ShotThumbnailCache(countLimit: 10)
        let image = try #require(cache.thumbnail(path: path, fitting: rowSize, scale: 2))
        let rep = try #require(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        // Aspect-fill for a 176x112 pixel box, far below the 3000px source.
        #expect(rep.width < 400)
        #expect(rep.width >= 176)
        #expect(rep.height >= 112)
    }

    @Test
    func neverUpscalesSmallImages() throws {
        let path = try tempPNG(width: 40, height: 30)
        defer { try? FileManager.default.removeItem(atPath: path) }
        let cache = ShotThumbnailCache(countLimit: 10)
        let image = try #require(cache.thumbnail(path: path, fitting: rowSize, scale: 2))
        #expect(image.cgImage(forProposedRect: nil, context: nil, hints: nil)?.width == 40)
    }

    @Test
    func secondLookupHitsTheCache() throws {
        let path = try tempPNG(width: 800, height: 600)
        defer { try? FileManager.default.removeItem(atPath: path) }
        let cache = ShotThumbnailCache(countLimit: 10)
        #expect(cache.cached(path: path, fitting: rowSize, scale: 2) == nil)
        let first = try #require(cache.thumbnail(path: path, fitting: rowSize, scale: 2))
        let hit = try #require(cache.cached(path: path, fitting: rowSize, scale: 2))
        #expect(hit === first)
        #expect(cache.thumbnail(path: path, fitting: rowSize, scale: 2) === first)
    }

    @Test
    func rewrittenFileInvalidatesTheEntry() throws {
        let path = try tempPNG(width: 800, height: 600)
        defer { try? FileManager.default.removeItem(atPath: path) }
        let cache = ShotThumbnailCache(countLimit: 10)
        _ = cache.thumbnail(path: path, fitting: rowSize, scale: 2)
        try writePNG(width: 801, height: 600, to: URL(fileURLWithPath: path))
        try FileManager.default.setAttributes(
            [.modificationDate: Date().addingTimeInterval(5)], ofItemAtPath: path)
        #expect(cache.cached(path: path, fitting: rowSize, scale: 2) == nil)
    }

    @Test
    func missingOrUndecodableFileReturnsNil() throws {
        let cache = ShotThumbnailCache(countLimit: 10)
        #expect(cache.thumbnail(path: "/nonexistent/x.png", fitting: rowSize, scale: 2) == nil)
        let junk = FileManager.default.temporaryDirectory
            .appendingPathComponent("junk-\(UUID().uuidString).png")
        try Data("nope".utf8).write(to: junk)
        defer { try? FileManager.default.removeItem(at: junk) }
        #expect(cache.thumbnail(path: junk.path, fitting: rowSize, scale: 2) == nil)
    }
}
