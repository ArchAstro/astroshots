import AppKit
import AVFoundation
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
        #expect(job.voice == NarrationDefaults.voice)
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
        #expect(NarrationVoice.available.map(\.id).contains("Aiden"))
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

    /// Regression for a circular AVAssetWriter wait: video was fed to
    /// completion before audio started, but long clips backpressure video until
    /// audio advances. This is synthetic and always runs in CI; no TTS model is
    /// loaded or downloaded.
    @Test(.timeLimit(.minutes(1)))
    func longNarrationFeedsAudioAndVideoConcurrently() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-narration-regression-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let wav = root.appendingPathComponent("long-silence.wav")
        let png = root.appendingPathComponent("frame.png")
        let mp4 = root.appendingPathComponent("narration.mp4")
        let duration = 33.0
        try writeSilentWAV(to: wav, duration: duration)
        try writeSolidPNG(to: png, width: 1280, height: 720)

        try await NarrationVideoComposer.compose(
            steps: [
                .init(
                    imagePath: png.path,
                    audioPath: wav.path,
                    duration: duration,
                    title: "Checkout",
                    transcript: "The checkout is clear and the primary action is easy to find.",
                    good: ["Clear primary action"],
                    improve: ["Move trust details closer"]
                ),
            ],
            outputURL: mp4,
            logTitle: "Checkout review"
        )

        let bytes = try FileManager.default.attributesOfItem(atPath: mp4.path)[.size]
            as? NSNumber
        #expect((bytes?.intValue ?? 0) > 5_000)
        let asset = AVURLAsset(url: mp4)
        let assetDuration = try await asset.load(.duration).seconds
        #expect(assetDuration >= duration - 0.6)

        let videoTrack = try #require(
            try await asset.loadTracks(withMediaType: .video).first
        )
        let audioTrack = try #require(
            try await asset.loadTracks(withMediaType: .audio).first
        )
        let videoSample = try decodeFirstSample(
            from: asset,
            track: videoTrack,
            outputSettings: [
                kCVPixelBufferPixelFormatTypeKey as String: Int(kCVPixelFormatType_32BGRA),
            ]
        )
        #expect(CMSampleBufferGetImageBuffer(videoSample) != nil)

        let audioSample = try decodeFirstSample(
            from: asset,
            track: audioTrack,
            outputSettings: [
                AVFormatIDKey: kAudioFormatLinearPCM,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsFloatKey: false,
            ]
        )
        #expect(CMSampleBufferGetTotalSampleSize(audioSample) > 0)

        let keepPreview = ProcessInfo.processInfo.environment["ASTROSHOTS_KEEP_NARRATION_PREVIEW"] == "1"
            || FileManager.default.fileExists(atPath: "/tmp/ASTROSHOTS_KEEP_NARRATION_PREVIEW")
        if keepPreview {
            let preview = FileManager.default.temporaryDirectory
                .appendingPathComponent("astroshots-narration-preview.mp4")
            try? FileManager.default.removeItem(at: preview)
            try FileManager.default.copyItem(at: mp4, to: preview)
            try await writePreviewFrames(from: asset)
            print("narration preview: \(preview.path)")
        }
    }

    @Test func captionsAdvanceAndFeedbackFadesAfterThirtySeconds() {
        let transcript = "one two three four five six seven eight nine ten eleven twelve thirteen"
        #expect(NarrationVideoComposer.captionText(transcript, elapsed: 0, duration: 40).contains("one"))
        #expect(NarrationVideoComposer.captionText(transcript, elapsed: 39, duration: 40).contains("thirteen"))
        #expect(NarrationVideoComposer.feedbackOpacity(elapsed: 29, duration: 40, hasFeedback: true) == 1)
        #expect(NarrationVideoComposer.feedbackOpacity(elapsed: 30.5, duration: 40, hasFeedback: true) == 0.5)
        #expect(NarrationVideoComposer.feedbackOpacity(elapsed: 32, duration: 40, hasFeedback: true) == 0)
        #expect(NarrationVideoComposer.feedbackOpacity(elapsed: 5, duration: 8, hasFeedback: true) == 1)
        #expect(NarrationVideoComposer.feedbackOpacity(elapsed: 6, duration: 8, hasFeedback: true) == 0.5)
        #expect(NarrationVideoComposer.feedbackOpacity(elapsed: 7, duration: 8, hasFeedback: true) == 0)
        #expect(NarrationVideoComposer.feedbackOpacity(elapsed: 5, duration: 40, hasFeedback: false) == 0)
    }

    @Test func narratedFilenameIncludesSanitizedLogTitle() {
        #expect(NarrationJobQueue.safeFilenameComponent("Checkout / mobile ✨") == "Checkout-mobile")
        #expect(NarrationJobQueue.safeFilenameComponent("✨") == "Friction-Log")
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

    private func writeSilentWAV(to url: URL, duration: Double) throws {
        let sampleRate = 44_100.0
        let frameCount = AVAudioFrameCount(duration * sampleRate)
        let format = try #require(
            AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1)
        )
        let buffer = try #require(
            AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frameCount)
        )
        buffer.frameLength = frameCount
        let file = try AVAudioFile(forWriting: url, settings: format.settings)
        try file.write(from: buffer)
    }

    private func writePreviewFrames(from asset: AVAsset) async throws {
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        generator.requestedTimeToleranceBefore = .zero
        generator.requestedTimeToleranceAfter = .zero

        for (name, seconds) in [
            ("intro", 1.0),
            ("feedback", 4.0),
            ("fullscreen", 34.5),
            ("outro", 37.0),
        ] {
            let (image, _) = try await generator.image(
                at: CMTime(seconds: seconds, preferredTimescale: 600)
            )
            let representation = NSBitmapImageRep(cgImage: image)
            let data = try #require(
                representation.representation(using: .png, properties: [:])
            )
            let output = FileManager.default.temporaryDirectory
                .appendingPathComponent("astroshots-narration-\(name).png")
            try data.write(to: output, options: .atomic)
            print("narration frame: \(output.path)")
        }
    }

    private func decodeFirstSample(
        from asset: AVAsset,
        track: AVAssetTrack,
        outputSettings: [String: Any]
    ) throws -> CMSampleBuffer {
        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: outputSettings)
        try #require(reader.canAdd(output))
        reader.add(output)
        guard reader.startReading() else {
            throw NarrationError.processFailed(
                reader.error?.localizedDescription ?? "Decode failed"
            )
        }
        guard let sample = output.copyNextSampleBuffer() else {
            throw NarrationError.processFailed(
                reader.error?.localizedDescription ?? "Track produced no decoded samples"
            )
        }
        return sample
    }
}
