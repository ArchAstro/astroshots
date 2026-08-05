import AppKit
import Foundation
import Testing
@testable import Astroshots

struct ImageClipboardTests {
    @Test @MainActor
    func copyImageWritesReadableImageToPasteboard() throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-clipboard-\(UUID().uuidString)",
                isDirectory: true
            )
        try FileManager.default.createDirectory(
            at: temporary,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: temporary) }

        let imageURL = temporary.appendingPathComponent("sample.png")
        let image = NSImage(size: NSSize(width: 8, height: 8))
        image.lockFocus()
        NSColor.systemPurple.setFill()
        NSBezierPath(rect: NSRect(x: 0, y: 0, width: 8, height: 8)).fill()
        image.unlockFocus()
        let tiff = try #require(image.tiffRepresentation)
        let rep = try #require(NSBitmapImageRep(data: tiff))
        let png = try #require(rep.representation(using: .png, properties: [:]))
        try png.write(to: imageURL)

        NSPasteboard.general.clearContents()
        #expect(ImageClipboard.copyImage(atPath: imageURL.path))

        let pasted = NSPasteboard.general.readObjects(forClasses: [NSImage.self])
        let recovered = try #require(pasted?.first as? NSImage)
        #expect(recovered.size.width > 0)
        #expect(recovered.size.height > 0)
    }

    @Test @MainActor
    func copyImageFailsForMissingPath() {
        NSPasteboard.general.clearContents()
        #expect(
            ImageClipboard.copyImage(
                atPath: "/tmp/astroshots-missing-\(UUID().uuidString).png"
            ) == false
        )
    }

    @Test @MainActor
    func appStateCopyShotImageReportsToast() throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-clipboard-state-\(UUID().uuidString)",
                isDirectory: true
            )
        try FileManager.default.createDirectory(
            at: temporary,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: temporary) }

        let imageURL = temporary.appendingPathComponent("frame.png")
        let image = NSImage(size: NSSize(width: 4, height: 4))
        image.lockFocus()
        NSColor.black.setFill()
        NSBezierPath(rect: NSRect(x: 0, y: 0, width: 4, height: 4)).fill()
        image.unlockFocus()
        let tiff = try #require(image.tiffRepresentation)
        let rep = try #require(NSBitmapImageRep(data: tiff))
        let png = try #require(rep.representation(using: .png, properties: [:]))
        try png.write(to: imageURL)

        let suiteName = "astroshots-clipboard-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)
        preferences.watchRootPaths = [temporary.path]
        preferences.markFirstRunSetupComplete()

        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [temporary],
                cacheFileURL: temporary.appendingPathComponent("shot-index.json")
            )
        )
        defer { watcher.stop() }

        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: false
        )
        let shot = Shot(
            path: imageURL.path,
            worktree: "demo",
            worktreePath: temporary.path,
            feature: "clip",
            fileName: "frame.png",
            sequence: nil,
            slug: "frame",
            title: "Frame",
            description: "",
            url: nil,
            runID: nil,
            status: nil,
            capturedAt: Date()
        )

        #expect(state.copyShotImage(shot))
        #expect(state.toast == "Copied image")
    }
}
