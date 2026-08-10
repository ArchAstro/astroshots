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

        try await appendVideoFrames(
            adaptor: adaptor,
            input: videoInput,
            image: image,
            size: size,
            frameCount: frameCount,
            frameDuration: frameDuration
        )
        videoInput.markAsFinished()

        try await appendAudioFile(
            path: step.audioPath,
            to: audioInput,
            maxDuration: CMTime(seconds: duration, preferredTimescale: 600)
        )
        audioInput.markAsFinished()

        await writer.finishWriting()
        guard writer.status == .completed else {
            throw NarrationError.processFailed(
                writer.error?.localizedDescription ?? "Segment encode failed."
            )
        }
    }

    private static func appendVideoFrames(
        adaptor: AVAssetWriterInputPixelBufferAdaptor,
        input: AVAssetWriterInput,
        image: NSImage,
        size: CGSize,
        frameCount: Int,
        frameDuration: CMTime
    ) async throws {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            let queue = DispatchQueue(label: "ai.archastro.astroshots.narration.video")
            var index = 0
            var finished = false

            input.requestMediaDataWhenReady(on: queue) {
                if finished { return }
                while input.isReadyForMoreMediaData {
                    if index >= frameCount {
                        finished = true
                        cont.resume()
                        return
                    }
                    guard let buffer = makePixelBuffer(from: image, size: size) else {
                        finished = true
                        cont.resume(
                            throwing: NarrationError.processFailed("Could not create video frame.")
                        )
                        return
                    }
                    let time = CMTimeMultiply(frameDuration, multiplier: Int32(index))
                    if !adaptor.append(buffer, withPresentationTime: time) {
                        finished = true
                        cont.resume(
                            throwing: NarrationError.processFailed("Failed appending video frame.")
                        )
                        return
                    }
                    index += 1
                }
            }
        }
    }

    private static func appendAudioFile(
        path: String,
        to input: AVAssetWriterInput,
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

        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            let queue = DispatchQueue(label: "ai.archastro.astroshots.narration.audio")
            var finished = false

            input.requestMediaDataWhenReady(on: queue) {
                if finished { return }
                while input.isReadyForMoreMediaData {
                    guard let sample = output.copyNextSampleBuffer() else {
                        finished = true
                        cont.resume()
                        return
                    }
                    let pts = CMSampleBufferGetPresentationTimeStamp(sample)
                    if CMTimeCompare(pts, maxDuration) > 0 {
                        finished = true
                        cont.resume()
                        return
                    }
                    if !input.append(sample) {
                        finished = true
                        cont.resume(
                            throwing: NarrationError.processFailed("Failed appending audio sample.")
                        )
                        return
                    }
                }
            }
        }
        reader.cancelReading()
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
