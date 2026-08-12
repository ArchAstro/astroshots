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
        var audioPath: String
        var duration: Double
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

        init(
            writer: AVAssetWriter,
            videoInput: AVAssetWriterInput,
            adaptor: AVAssetWriterInputPixelBufferAdaptor,
            audioInput: AVAssetWriterInput,
            image: NSImage
        ) {
            self.writer = writer
            self.videoInput = videoInput
            self.adaptor = adaptor
            self.audioInput = audioInput
            self.image = image
        }
    }

    static func compose(
        steps: [StepMedia],
        outputURL: URL,
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

        var segmentURLs: [URL] = []
        for (index, step) in steps.enumerated() {
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
        // Sparse still frames — 2 fps is enough for a held screenshot.
        let fps: Int32 = 2
        let frameCount = max(2, Int(ceil(duration * Double(fps))))
        let frameDuration = CMTime(value: 1, timescale: fps)
        let context = SegmentWriterContext(
            writer: writer,
            videoInput: videoInput,
            adaptor: adaptor,
            audioInput: audioInput,
            image: image
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

            guard let buffer = makePixelBuffer(from: image, size: size) else {
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
        path: String,
        to input: AVAssetWriterInput,
        writer: AVAssetWriter,
        maxDuration: CMTime
    ) async throws {
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

    private static func makePixelBuffer(from image: NSImage, size: CGSize) -> CVPixelBuffer? {
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

        context.setFillColor(NSColor.black.cgColor)
        context.fill(CGRect(origin: .zero, size: size))

        guard let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
            return buffer
        }
        let imageSize = CGSize(width: cgImage.width, height: cgImage.height)
        let scale = min(size.width / imageSize.width, size.height / imageSize.height)
        let drawSize = CGSize(width: imageSize.width * scale, height: imageSize.height * scale)
        let origin = CGPoint(
            x: (size.width - drawSize.width) / 2,
            y: (size.height - drawSize.height) / 2
        )
        context.interpolationQuality = .high
        context.draw(cgImage, in: CGRect(origin: origin, size: drawSize))
        return buffer
    }
}
