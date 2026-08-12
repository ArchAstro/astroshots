import AVFoundation
import AppKit
import CoreGraphics
import Foundation

/// Builds a step-by-step narrated MP4: still + TTS audio per step.
///
/// Each step is a short MP4 segment (image frames + audio). Segments are then
/// concatenated with `AVMutableComposition`.
enum NarrationVideoComposer {
    struct StepMedia: Sendable {
        var imagePath: String?
        var audioPath: String?
        var duration: Double
        var title: String = ""
        var transcript: String = ""
        var good: [String] = []
        var improve: [String] = []
        var isBrandCard = false
    }

    /// AVAssetWriter is designed for independently fed inputs but its Objective-C
    /// types do not declare Sendable conformance. Access is split by track; only
    /// status/cancellation is shared across the two child tasks.
    private final class SegmentWriterContext: @unchecked Sendable {
        let writer: AVAssetWriter
        let videoInput: AVAssetWriterInput
        let adaptor: AVAssetWriterInputPixelBufferAdaptor
        let audioInput: AVAssetWriterInput
        let image: NSImage
        let step: StepMedia

        init(
            writer: AVAssetWriter,
            videoInput: AVAssetWriterInput,
            adaptor: AVAssetWriterInputPixelBufferAdaptor,
            audioInput: AVAssetWriterInput,
            image: NSImage,
            step: StepMedia
        ) {
            self.writer = writer
            self.videoInput = videoInput
            self.adaptor = adaptor
            self.audioInput = audioInput
            self.image = image
            self.step = step
        }
    }

    static func compose(
        steps: [StepMedia],
        outputURL: URL,
        logTitle: String = "Astroshots",
        size: CGSize = CGSize(width: 1280, height: 720)
    ) async throws {
        guard !steps.isEmpty else {
            throw NarrationError.processFailed("No steps to encode.")
        }
        if FileManager.default.fileExists(atPath: outputURL.path) {
            try FileManager.default.removeItem(at: outputURL)
        }
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )

        let workRoot = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-narration-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workRoot, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: workRoot) }

        let brandCard = StepMedia(
            imagePath: nil,
            audioPath: nil,
            duration: NarrationDefaults.titleCardSeconds,
            title: logTitle,
            isBrandCard: true
        )
        let timeline = [brandCard] + steps + [brandCard]

        var segmentURLs: [URL] = []
        for (index, step) in timeline.enumerated() {
            let segment = workRoot.appendingPathComponent(String(format: "seg-%02d.mp4", index))
            try await writeSegment(step: step, outputURL: segment, size: size)
            segmentURLs.append(segment)
        }

        if segmentURLs.count == 1 {
            try FileManager.default.copyItem(at: segmentURLs[0], to: outputURL)
            return
        }
        try await concatenate(segments: segmentURLs, outputURL: outputURL)
    }

    // MARK: - Segment

    private static func writeSegment(
        step: StepMedia,
        outputURL: URL,
        size: CGSize
    ) async throws {
        let duration = max(step.duration, NarrationDefaults.minimumStepSeconds)
        let writer = try AVAssetWriter(outputURL: outputURL, fileType: .mp4)

        let videoSettings: [String: Any] = [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: Int(size.width),
            AVVideoHeightKey: Int(size.height),
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: 2_000_000,
            ],
        ]
        let videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
        videoInput.expectsMediaDataInRealTime = false

        let adaptor = AVAssetWriterInputPixelBufferAdaptor(
            assetWriterInput: videoInput,
            sourcePixelBufferAttributes: [
                kCVPixelBufferPixelFormatTypeKey as String: Int(kCVPixelFormatType_32BGRA),
                kCVPixelBufferWidthKey as String: Int(size.width),
                kCVPixelBufferHeightKey as String: Int(size.height),
            ]
        )

        let audioSettings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVNumberOfChannelsKey: 1,
            AVSampleRateKey: 44_100,
            AVEncoderBitRateKey: 128_000,
        ]
        let audioInput = AVAssetWriterInput(mediaType: .audio, outputSettings: audioSettings)
        audioInput.expectsMediaDataInRealTime = false

        guard writer.canAdd(videoInput), writer.canAdd(audioInput) else {
            throw NarrationError.processFailed("Cannot configure segment writer inputs.")
        }
        writer.add(videoInput)
        writer.add(audioInput)
        guard writer.startWriting() else {
            throw NarrationError.processFailed(
                writer.error?.localizedDescription ?? "Failed to start segment writer."
            )
        }
        writer.startSession(atSourceTime: .zero)

        let image = loadImage(path: step.imagePath) ?? placeholderImage(size: size)
        // Still-image segments stay inexpensive while fades remain visibly smooth.
        let fps = NarrationDefaults.framesPerSecond
        let frameCount = max(2, Int(ceil(duration * Double(fps))))
        let frameDuration = CMTime(value: 1, timescale: fps)
        let context = SegmentWriterContext(
            writer: writer,
            videoInput: videoInput,
            adaptor: adaptor,
            audioInput: audioInput,
            image: image,
            step: step
        )

        do {
            // AVAssetWriter interleaves its inputs and may backpressure one
            // until the other advances. Feed both tracks concurrently; awaiting
            // all video before starting audio deadlocks on longer narration.
            try await withThrowingTaskGroup(of: Void.self) { group in
                group.addTask {
                    try await appendVideoFrames(
                        adaptor: context.adaptor,
                        input: context.videoInput,
                        writer: context.writer,
                        image: context.image,
                        step: context.step,
                        size: size,
                        frameCount: frameCount,
                        frameDuration: frameDuration
                    )
                    context.videoInput.markAsFinished()
                }
                group.addTask {
                    try await appendAudioFile(
                        path: step.audioPath,
                        to: context.audioInput,
                        writer: context.writer,
                        maxDuration: CMTime(seconds: duration, preferredTimescale: 600)
                    )
                    context.audioInput.markAsFinished()
                }
                try await group.waitForAll()
            }

            try Task.checkCancellation()
            await withTaskCancellationHandler {
                await writer.finishWriting()
            } onCancel: {
                context.writer.cancelWriting()
            }
        } catch {
            writer.cancelWriting()
            throw error
        }
        guard writer.status == .completed else {
            throw NarrationError.processFailed(
                writer.error?.localizedDescription ?? "Segment encode failed."
            )
        }
    }

    private static func appendVideoFrames(
        adaptor: AVAssetWriterInputPixelBufferAdaptor,
        input: AVAssetWriterInput,
        writer: AVAssetWriter,
        image: NSImage,
        step: StepMedia,
        size: CGSize,
        frameCount: Int,
        frameDuration: CMTime
    ) async throws {
        var index = 0
        while index < frameCount {
            try Task.checkCancellation()
            try checkWriter(writer)
            guard input.isReadyForMoreMediaData else {
                try await Task.sleep(for: .milliseconds(2))
                continue
            }

            let elapsed = Double(index) / Double(frameDuration.timescale)
            guard let buffer = makePixelBuffer(
                from: image,
                step: step,
                elapsed: elapsed,
                duration: Double(frameCount) / Double(frameDuration.timescale),
                size: size
            ) else {
                throw NarrationError.processFailed("Could not create video frame.")
            }
            let time = CMTimeMultiply(frameDuration, multiplier: Int32(index))
            guard adaptor.append(buffer, withPresentationTime: time) else {
                throw NarrationError.processFailed(
                    writer.error?.localizedDescription ?? "Failed appending video frame."
                )
            }
            index += 1
        }
    }

    private static func appendAudioFile(
        path: String?,
        to input: AVAssetWriterInput,
        writer: AVAssetWriter,
        maxDuration: CMTime
    ) async throws {
        guard let path else { return }
        let url = URL(fileURLWithPath: path)
        guard FileManager.default.fileExists(atPath: path) else { return }
        let asset = AVURLAsset(url: url)
        let tracks = try await asset.loadTracks(withMediaType: .audio)
        guard let track = tracks.first else { return }

        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(
            track: track,
            outputSettings: [
                AVFormatIDKey: kAudioFormatLinearPCM,
                AVLinearPCMIsFloatKey: false,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsBigEndianKey: false,
                AVLinearPCMIsNonInterleaved: false,
            ]
        )
        output.alwaysCopiesSampleData = false
        guard reader.canAdd(output) else {
            throw NarrationError.processFailed("Cannot add audio reader output.")
        }
        reader.add(output)
        guard reader.startReading() else {
            throw NarrationError.processFailed(
                reader.error?.localizedDescription ?? "Audio reader failed to start."
            )
        }
        defer { reader.cancelReading() }

        while true {
            try Task.checkCancellation()
            try checkWriter(writer)
            guard input.isReadyForMoreMediaData else {
                try await Task.sleep(for: .milliseconds(2))
                continue
            }

            guard let sample = output.copyNextSampleBuffer() else {
                if reader.status == .failed {
                    throw NarrationError.processFailed(
                        reader.error?.localizedDescription ?? "Audio reader failed."
                    )
                }
                break
            }
            let pts = CMSampleBufferGetPresentationTimeStamp(sample)
            if CMTimeCompare(pts, maxDuration) > 0 {
                break
            }
            guard input.append(sample) else {
                throw NarrationError.processFailed(
                    writer.error?.localizedDescription ?? "Failed appending audio sample."
                )
            }
        }
    }

    private static func checkWriter(_ writer: AVAssetWriter) throws {
        switch writer.status {
        case .failed:
            throw NarrationError.processFailed(
                writer.error?.localizedDescription ?? "Segment encode failed."
            )
        case .cancelled:
            throw CancellationError()
        default:
            return
        }
    }

    private static func concatenate(segments: [URL], outputURL: URL) async throws {
        let composition = AVMutableComposition()
        guard
            let videoTrack = composition.addMutableTrack(
                withMediaType: .video,
                preferredTrackID: kCMPersistentTrackID_Invalid
            ),
            let audioTrack = composition.addMutableTrack(
                withMediaType: .audio,
                preferredTrackID: kCMPersistentTrackID_Invalid
            )
        else {
            throw NarrationError.processFailed("Could not build composition tracks.")
        }

        var cursor = CMTime.zero
        for url in segments {
            let asset = AVURLAsset(url: url)
            let duration = try await asset.load(.duration)
            let vTracks = try await asset.loadTracks(withMediaType: .video)
            let aTracks = try await asset.loadTracks(withMediaType: .audio)
            let timeRange = CMTimeRange(start: .zero, duration: duration)
            if let v = vTracks.first {
                try videoTrack.insertTimeRange(timeRange, of: v, at: cursor)
            }
            if let a = aTracks.first {
                try audioTrack.insertTimeRange(timeRange, of: a, at: cursor)
            }
            cursor = CMTimeAdd(cursor, duration)
        }

        guard let export = AVAssetExportSession(
            asset: composition,
            presetName: AVAssetExportPresetHighestQuality
        ) else {
            throw NarrationError.processFailed("Could not create export session.")
        }
        export.outputURL = outputURL
        export.outputFileType = .mp4
        await export.export()
        guard export.status == .completed else {
            throw NarrationError.processFailed(
                export.error?.localizedDescription ?? "Concat export failed."
            )
        }
    }

    // MARK: - Images

    private static func loadImage(path: String?) -> NSImage? {
        guard let path, FileManager.default.fileExists(atPath: path) else { return nil }
        return NSImage(contentsOfFile: path)
    }

    private static func placeholderImage(size: CGSize) -> NSImage {
        let image = NSImage(size: size)
        image.lockFocus()
        NSColor(calibratedWhite: 0.12, alpha: 1).setFill()
        NSBezierPath(rect: NSRect(origin: .zero, size: size)).fill()
        let text = "No screenshot" as NSString
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 28, weight: .medium),
            .foregroundColor: NSColor.white.withAlphaComponent(0.55),
        ]
        let textSize = text.size(withAttributes: attrs)
        text.draw(
            at: NSPoint(
                x: (size.width - textSize.width) / 2,
                y: (size.height - textSize.height) / 2
            ),
            withAttributes: attrs
        )
        image.unlockFocus()
        return image
    }

    private static func makePixelBuffer(
        from image: NSImage,
        step: StepMedia,
        elapsed: Double,
        duration: Double,
        size: CGSize
    ) -> CVPixelBuffer? {
        var buffer: CVPixelBuffer?
        let attrs: [CFString: Any] = [
            kCVPixelBufferCGImageCompatibilityKey: true,
            kCVPixelBufferCGBitmapContextCompatibilityKey: true,
        ]
        let status = CVPixelBufferCreate(
            kCFAllocatorDefault,
            Int(size.width),
            Int(size.height),
            kCVPixelFormatType_32BGRA,
            attrs as CFDictionary,
            &buffer
        )
        guard status == kCVReturnSuccess, let buffer else { return nil }

        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }

        guard let context = CGContext(
            data: CVPixelBufferGetBaseAddress(buffer),
            width: Int(size.width),
            height: Int(size.height),
            bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(buffer),
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                | CGBitmapInfo.byteOrder32Little.rawValue
        ) else { return nil }

        let backgroundColor = step.isBrandCard
            ? NSColor.black
            : NSColor(calibratedWhite: 0.035, alpha: 1)
        context.setFillColor(backgroundColor.cgColor)
        context.fill(CGRect(origin: .zero, size: size))

        if step.isBrandCard {
            drawBrandOverlay(title: step.title, size: size, context: context)
            return buffer
        }

        let feedbackAlpha = feedbackOpacity(
            elapsed: elapsed,
            duration: duration,
            hasFeedback: !step.good.isEmpty || !step.improve.isEmpty
        )
        let inset: CGFloat = 28 * feedbackAlpha
        let railWidth = size.width * 0.31
        let imageRight = size.width - inset - railWidth * feedbackAlpha
        let imageFrame = CGRect(
            x: inset,
            y: inset,
            width: max(1, imageRight - inset),
            height: size.height - inset * 2
        )
        drawAspectFit(image, in: imageFrame, context: context)

        if feedbackAlpha > 0.001 {
            drawFeedbackRail(
                step: step,
                alpha: feedbackAlpha,
                frame: CGRect(
                    x: size.width - railWidth + 10,
                    y: 34,
                    width: railWidth - 34,
                    height: size.height - 68
                ),
                context: context
            )
        }
        drawCaptions(
            captionText(step.transcript, elapsed: elapsed, duration: duration),
            feedbackAlpha: feedbackAlpha,
            size: size,
            context: context
        )
        return buffer
    }

    static func feedbackOpacity(
        elapsed: Double,
        duration: Double,
        hasFeedback: Bool
    ) -> CGFloat {
        guard hasFeedback else { return 0 }
        let fadeStart = min(
            NarrationDefaults.feedbackVisibleSeconds,
            max(
                0,
                duration
                    - NarrationDefaults.feedbackFadeSeconds
                    - NarrationDefaults.fullScreenHoldSeconds
            )
        )
        guard elapsed > fadeStart else { return 1 }
        let progress = (elapsed - fadeStart) / NarrationDefaults.feedbackFadeSeconds
        return CGFloat(max(0, 1 - min(progress, 1)))
    }

    static func captionText(_ transcript: String, elapsed: Double, duration: Double) -> String {
        let words = transcript.split(whereSeparator: { $0.isWhitespace })
        guard !words.isEmpty else { return "" }
        let wordsPerCaption = 11
        let chunks = stride(from: 0, to: words.count, by: wordsPerCaption).map {
            words[$0 ..< min($0 + wordsPerCaption, words.count)].joined(separator: " ")
        }
        let progress = duration > 0 ? max(0, min(elapsed / duration, 0.999_999)) : 0
        return chunks[min(Int(progress * Double(chunks.count)), chunks.count - 1)]
    }

    private static func brandIcon() -> NSImage? {
        guard let url = Bundle.main.url(
            forResource: "astroshots-app-icon-master",
            withExtension: "png"
        ) else { return nil }
        return NSImage(contentsOf: url)
    }

    private static func drawAspectFit(
        _ image: NSImage,
        in frame: CGRect,
        context: CGContext
    ) {
        guard let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
            return
        }
        let imageSize = CGSize(width: cgImage.width, height: cgImage.height)
        let scale = min(frame.width / imageSize.width, frame.height / imageSize.height)
        let drawSize = CGSize(width: imageSize.width * scale, height: imageSize.height * scale)
        let rect = CGRect(
            x: frame.midX - drawSize.width / 2,
            y: frame.midY - drawSize.height / 2,
            width: drawSize.width,
            height: drawSize.height
        )
        context.interpolationQuality = .high
        context.draw(cgImage, in: rect)
    }

    private static func withFlippedAppKitContext(
        _ context: CGContext,
        draw: () -> Void
    ) {
        context.saveGState()
        context.translateBy(x: 0, y: CGFloat(context.height))
        context.scaleBy(x: 1, y: -1)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        draw()
        NSGraphicsContext.restoreGraphicsState()
        context.restoreGState()
    }

    private static func drawBrandOverlay(title: String, size: CGSize, context: CGContext) {
        withFlippedAppKitContext(context) {
            NSColor.black.withAlphaComponent(0.42).setFill()
            NSBezierPath(rect: CGRect(origin: .zero, size: size)).fill()

            let card = NSBezierPath(
                roundedRect: CGRect(
                    x: size.width / 2 - 310,
                    y: 112,
                    width: 620,
                    height: 430
                ),
                xRadius: 30,
                yRadius: 30
            )
            NSColor.black.withAlphaComponent(0.92).setFill()
            card.fill()
            NSColor.white.withAlphaComponent(0.14).setStroke()
            card.lineWidth = 1
            card.stroke()

            if let icon = brandIcon() {
                icon.draw(
                    in: CGRect(x: size.width / 2 - 58, y: 150, width: 116, height: 116),
                    from: .zero,
                    operation: .sourceOver,
                    fraction: 1
                )
            }

            let paragraph = NSMutableParagraphStyle()
            paragraph.alignment = .center
            paragraph.lineBreakMode = .byWordWrapping
            (title as NSString).draw(
                in: CGRect(x: 150, y: 292, width: size.width - 300, height: 150),
                withAttributes: [
                    .font: NSFont.systemFont(ofSize: 50, weight: .bold),
                    .foregroundColor: NSColor.white,
                    .paragraphStyle: paragraph,
                ]
            )
            ("Astroshots by ArchAstro" as NSString).draw(
                in: CGRect(x: 150, y: 458, width: size.width - 300, height: 54),
                withAttributes: [
                    .font: NSFont.systemFont(ofSize: 24, weight: .medium),
                    .foregroundColor: NSColor.white.withAlphaComponent(0.82),
                    .paragraphStyle: paragraph,
                ]
            )
        }
    }

    private static func drawFeedbackRail(
        step: StepMedia,
        alpha: CGFloat,
        frame: CGRect,
        context: CGContext
    ) {
        withFlippedAppKitContext(context) {
            let rail = NSBezierPath(roundedRect: frame, xRadius: 24, yRadius: 24)
            NSColor(calibratedWhite: 0.075, alpha: 0.96 * alpha).setFill()
            rail.fill()
            NSColor.white.withAlphaComponent(0.12 * alpha).setStroke()
            rail.lineWidth = 1
            rail.stroke()

            var y = frame.minY + 28
            (step.title as NSString).draw(
                in: CGRect(x: frame.minX + 24, y: y, width: frame.width - 48, height: 70),
                withAttributes: [
                    .font: NSFont.systemFont(ofSize: 25, weight: .bold),
                    .foregroundColor: NSColor.white.withAlphaComponent(alpha),
                ]
            )
            y += 82
            y = drawFeedbackSection(
                label: "WHAT WORKED",
                bullets: step.good,
                color: NSColor(calibratedRed: 0.25, green: 0.88, blue: 0.65, alpha: alpha),
                x: frame.minX + 24,
                y: y,
                width: frame.width - 48
            )
            if !step.good.isEmpty, !step.improve.isEmpty { y += 22 }
            _ = drawFeedbackSection(
                label: "IMPROVE",
                bullets: step.improve,
                color: NSColor(calibratedRed: 1.0, green: 0.68, blue: 0.28, alpha: alpha),
                x: frame.minX + 24,
                y: y,
                width: frame.width - 48
            )
        }
    }

    private static func drawFeedbackSection(
        label: String,
        bullets: [String],
        color: NSColor,
        x: CGFloat,
        y: CGFloat,
        width: CGFloat
    ) -> CGFloat {
        guard !bullets.isEmpty else { return y }
        (label as NSString).draw(
            in: CGRect(x: x, y: y, width: width, height: 26),
            withAttributes: [
                .font: NSFont.systemFont(ofSize: 13, weight: .bold),
                .foregroundColor: color,
            ]
        )
        var cursor = y + 31
        for bullet in bullets.prefix(4) {
            let text = "•  \(bullet)"
            (text as NSString).draw(
                in: CGRect(x: x, y: cursor, width: width, height: 68),
                withAttributes: [
                    .font: NSFont.systemFont(ofSize: 18, weight: .medium),
                    .foregroundColor: NSColor.white.withAlphaComponent(color.alphaComponent),
                ]
            )
            cursor += 62
        }
        return cursor
    }

    private static func drawCaptions(
        _ text: String,
        feedbackAlpha: CGFloat,
        size: CGSize,
        context: CGContext
    ) {
        guard !text.isEmpty else { return }
        withFlippedAppKitContext(context) {
            let railWidth = feedbackAlpha > 0.001 ? size.width * 0.31 : 0
            let horizontalInset: CGFloat = 90
            let frame = CGRect(
                x: horizontalInset,
                y: size.height - 112,
                width: size.width - horizontalInset * 2 - railWidth,
                height: 76
            )
            NSColor.black.withAlphaComponent(0.78).setFill()
            NSBezierPath(roundedRect: frame.insetBy(dx: -18, dy: -10), xRadius: 14, yRadius: 14)
                .fill()
            let paragraph = NSMutableParagraphStyle()
            paragraph.alignment = .center
            paragraph.lineBreakMode = .byWordWrapping
            (text as NSString).draw(
                in: frame,
                withAttributes: [
                    .font: NSFont.systemFont(ofSize: 23, weight: .semibold),
                    .foregroundColor: NSColor.white,
                    .paragraphStyle: paragraph,
                ]
            )
        }
    }
}
