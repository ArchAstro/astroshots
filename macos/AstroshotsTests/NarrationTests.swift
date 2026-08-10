import AppKit
import Foundation
import Testing
@testable import Astroshots

struct NarrationTests {
    @Test func capabilityDetectsAppleSiliconOnThisHost() {
        #if arch(arm64)
        #expect(NarrationModelManager.isAppleSilicon())
        #endif
        #expect(NarrationModelManager.physicalMemoryGB() > 0)
    }

    @Test func jobFactoryUsesTranscriptWhenPresent() throws {
        let step = FrictionLogStep(
            step: 1,
            stepID: "land",
            title: "Land",
            description: "desc",
            transcript: "I open the app and scan the hero.",
            screenshotPaths: [],
            good: [],
            improve: [],
            url: "/",
            capturedAt: nil
        )
        let run = FrictionLogRun(
            runID: "20260809T120000Z",
            directoryPath: "/tmp/run",
            steps: [step],
            capturedAt: Date(),
            status: .complete
        )
        let log = FrictionLog(
            worktree: "wt1",
            worktreePath: "/tmp/wt1",
            slug: "demo",
            title: "Demo",
            description: "",
            promptPath: nil,
            promptMarkdown: nil,
            status: .complete,
            runs: [run],
            updatedAt: Date()
        )
        let job = NarrationJob.make(log: log, run: run)
        #expect(job.stepTranscripts.count == 1)
        #expect(job.stepTranscripts[0].transcript.contains("open the app"))
        #expect(job.phase == .queued)
    }

    @Test func readinessStatusLinesAreStable() {
        #expect(NarrationReadiness.disabled.statusLine == "Off")
        #expect(NarrationReadiness.ready.isReady)
        #expect(NarrationReadiness.downloadingModel(0.5).isBusy)
        #expect(NarrationReadiness.downloadingModel(0.5).progressFraction == 0.5)
        #expect(NarrationReadiness.loadingModel.isBusy)
    }

    @Test func defaultModelIsQwen3TTS() {
        #expect(NarrationDefaults.modelID.contains("Qwen3-TTS"))
        #expect(NarrationDefaults.voice == "Ryan")
    }

    @Test func modelDirectoryUsesMlxAudioCacheLayout() {
        let dir = NarrationPaths.mlxAudioModelDirectory()
        #expect(dir.path.contains("mlx-audio"))
        #expect(
            dir.path.contains("Qwen3-TTS")
                || dir.path.contains("Qwen3_TTS")
                || dir.lastPathComponent.contains("Qwen3")
        )
    }

    /// Opt-in smoke: download Qwen3-TTS, synthesize one phrase, write a WAV.
    ///
    /// xcodebuild does not always forward shell env into the test host, so we
    /// also accept a sentinel file:
    ///
    /// ```bash
    /// touch /tmp/ASTROSHOTS_NARRATION_E2E
    /// xcodebuild test … -only-testing:AstroshotsTests/NarrationTests
    /// rm -f /tmp/ASTROSHOTS_NARRATION_E2E
    /// ```
    @MainActor
    @Test func ttsSmokeGeneratesWav() async throws {
        // Opt-in only — CI never enables this (model download is large).
        let envOn = ProcessInfo.processInfo.environment["ASTROSHOTS_NARRATION_E2E"] == "1"
        let fileOn = FileManager.default.fileExists(
            atPath: "/tmp/ASTROSHOTS_NARRATION_E2E"
        )
        guard envOn || fileOn else { return }
        #expect(NarrationModelManager.isAppleSilicon())

        NarrationPaths.ensureDirectories()
        let suite = "astroshots.narration.e2e.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defaults.removePersistentDomain(forName: suite)
        defaults.set(true, forKey: "narrationEnabled")
        let manager = NarrationModelManager(preferences: Preferences(defaults: defaults))
        await manager.refreshAndBootstrapIfNeeded()
        let status = manager.readiness.statusLine
        #expect(manager.readiness.isReady, "bootstrap failed: \(status)")

        let out = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-narration-smoke.wav")
        try? FileManager.default.removeItem(at: out)
        let duration = try await manager.synthesizePhrase(
            "Astroshots narration smoke test.",
            to: out
        )
        #expect(duration > 0.2)
        #expect(FileManager.default.fileExists(atPath: out.path))
        let size = try FileManager.default.attributesOfItem(atPath: out.path)[.size] as? NSNumber
        #expect((size?.intValue ?? 0) > 1000)
        print("narration smoke wav: \(out.path) bytes=\(size?.intValue ?? 0) duration=\(duration)")

        // Encode a 1-step narrated MP4 from the smoke WAV + a solid PNG.
        let png = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-narration-smoke.png")
        try writeSolidPNG(to: png, width: 640, height: 360)
        let mp4 = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-narration-smoke.mp4")
        try? FileManager.default.removeItem(at: mp4)
        try await NarrationVideoComposer.compose(
            steps: [
                .init(imagePath: png.path, audioPath: out.path, duration: duration),
            ],
            outputURL: mp4,
            size: CGSize(width: 640, height: 360)
        )
        #expect(FileManager.default.fileExists(atPath: mp4.path))
        let mp4Size = try FileManager.default.attributesOfItem(atPath: mp4.path)[.size] as? NSNumber
        #expect((mp4Size?.intValue ?? 0) > 5_000)
        print("narration smoke mp4: \(mp4.path) bytes=\(mp4Size?.intValue ?? 0)")
    }

    private func writeSolidPNG(to url: URL, width: Int, height: Int) throws {
        let image = NSImage(size: NSSize(width: width, height: height))
        image.lockFocus()
        NSColor(calibratedRed: 0.2, green: 0.25, blue: 0.45, alpha: 1).setFill()
        NSBezierPath(rect: NSRect(x: 0, y: 0, width: width, height: height)).fill()
        image.unlockFocus()
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let data = rep.representation(using: .png, properties: [:])
        else {
            throw NarrationError.processFailed("Could not write smoke PNG")
        }
        try data.write(to: url)
    }
}
