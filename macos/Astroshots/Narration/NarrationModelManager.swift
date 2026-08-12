import AVFoundation
import Foundation
import HuggingFace
import MLX
import MLXAudioCore
import MLXAudioTTS
import Observation

/// Owns opt-in narration readiness using **mlx-audio-swift** (MLX Swift + Qwen3-TTS).
@MainActor
@Observable
final class NarrationModelManager {
    static let shared = NarrationModelManager()

    private(set) var readiness: NarrationReadiness = .disabled
    private(set) var capability: NarrationCapability = .unknown
    private(set) var lastError: String?
    private(set) var previewingVoice: String?

    /// Warm model kept after successful setup so render jobs skip reload cost.
    private(set) var loadedModel: (any SpeechGenerationModel)?

    private let preferences: Preferences
    private var bootstrapTask: Task<Void, Never>?
    private var previewTask: Task<Void, Never>?
    private var previewPlayer: AVAudioPlayer?
    private var synthesisBusy = false
    private var synthesisWaiters: [CheckedContinuation<Void, Never>] = []

    init(preferences: Preferences = .shared) {
        self.preferences = preferences
        #if DEBUG
        if ProcessInfo.processInfo.environment["ASTROSHOTS_UI_TEST_NARRATION_READY"] == "1" {
            readiness = .ready
            capability = NarrationCapability(
                isAppleSilicon: true,
                memoryGB: Self.physicalMemoryGB(),
                readyForTTS: true,
                unsupportedReason: nil
            )
            return
        }
        #endif
        if preferences.narrationEnabled {
            if NarrationPaths.isModelOnDisk() {
                readiness = .ready
            } else {
                readiness = .checking
            }
            Task { await refreshAndBootstrapIfNeeded() }
        } else {
            readiness = .disabled
        }
    }

    func setEnabled(_ enabled: Bool) {
        preferences.narrationEnabled = enabled
        if !enabled {
            bootstrapTask?.cancel()
            bootstrapTask = nil
            loadedModel = nil
            stopPreview()
            readiness = .disabled
            lastError = nil
            return
        }
        bootstrapTask?.cancel()
        bootstrapTask = Task { await refreshAndBootstrapIfNeeded() }
    }

    func retry() {
        guard preferences.narrationEnabled else { return }
        bootstrapTask?.cancel()
        bootstrapTask = Task { await refreshAndBootstrapIfNeeded(forceDownload: true) }
    }

    /// Ensures weights are on disk and the MLX model is loadable.
    func refreshAndBootstrapIfNeeded(forceDownload: Bool = false) async {
        readiness = .checking
        lastError = nil
        NarrationPaths.ensureDirectories()

        let silicon = Self.isAppleSilicon()
        let memory = Self.physicalMemoryGB()
        if !silicon {
            capability = NarrationCapability(
                isAppleSilicon: false,
                memoryGB: memory,
                readyForTTS: false,
                unsupportedReason: "Apple Silicon (M-series) is required for MLX."
            )
            readiness = .unsupported(capability.unsupportedReason ?? "Unsupported Mac")
            return
        }
        if memory > 0, memory < NarrationDefaults.minMemoryGB {
            capability = NarrationCapability(
                isAppleSilicon: true,
                memoryGB: memory,
                readyForTTS: false,
                unsupportedReason:
                    "At least \(Int(NarrationDefaults.minMemoryGB)) GB of memory is recommended for Qwen3-TTS."
            )
            readiness = .unsupported(capability.unsupportedReason ?? "Not enough memory")
            return
        }

        capability = NarrationCapability(
            isAppleSilicon: true,
            memoryGB: memory,
            readyForTTS: true,
            unsupportedReason: nil
        )

        do {
            if forceDownload || !NarrationPaths.isModelOnDisk() {
                readiness = .downloadingModel(0.02)
                try await downloadModel(force: forceDownload)
            }

            readiness = .loadingModel
            let model = try await TTS.loadModel(modelRepo: NarrationDefaults.modelID)
            loadedModel = model
            readiness = .ready
            preferences.narrationModelReady = true
        } catch is CancellationError {
            // leave state
        } catch {
            loadedModel = nil
            let message = (error as? LocalizedError)?.errorDescription
                ?? error.localizedDescription
            lastError = message
            preferences.narrationModelReady = false
            readiness = .failed(message)
        }
    }

    /// Returns a warm model, loading if needed.
    func modelForSynthesis() async throws -> any SpeechGenerationModel {
        if let loadedModel { return loadedModel }
        guard readiness.isReady || preferences.narrationEnabled else {
            throw NarrationError.notReady
        }
        readiness = .loadingModel
        let model = try await TTS.loadModel(modelRepo: NarrationDefaults.modelID)
        loadedModel = model
        readiness = .ready
        return model
    }

    /// One-shot TTS for smoke tests / debugging (writes a mono WAV).
    func synthesizePhrase(
        _ text: String,
        voice: String = NarrationDefaults.voice,
        to output: URL
    ) async throws -> Double {
        let model = try await modelForSynthesis()
        let audio = try await synthesizeAudio(text, voice: voice, using: model)
        let samples = audio.samples
        guard !samples.isEmpty else {
            throw NarrationError.processFailed("TTS produced empty audio.")
        }
        try FileManager.default.createDirectory(
            at: output.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try AudioUtils.writeWavFile(
            samples: samples,
            sampleRate: Double(audio.sampleRate),
            fileURL: output
        )
        return Double(samples.count) / Double(max(audio.sampleRate, 1))
    }

    func synthesizeAudio(
        _ text: String,
        voice: String,
        using model: any SpeechGenerationModel
    ) async throws -> (samples: [Float], sampleRate: Int) {
        await acquireSynthesisSlot()
        defer { releaseSynthesisSlot() }
        try Task.checkCancellation()

        let box = NarrationSpeechBox(model)
        let samples = try await box.speak(text, voice: NarrationVoice.normalized(voice))
        return (samples, box.sampleRate)
    }

    private func acquireSynthesisSlot() async {
        if !synthesisBusy {
            synthesisBusy = true
            return
        }
        await withCheckedContinuation { continuation in
            synthesisWaiters.append(continuation)
        }
    }

    private func releaseSynthesisSlot() {
        if synthesisWaiters.isEmpty {
            synthesisBusy = false
        } else {
            synthesisWaiters.removeFirst().resume()
        }
    }

    func previewVoice(_ voice: String) {
        let voice = NarrationVoice.normalized(voice)
        if previewingVoice == voice {
            stopPreview()
            return
        }
        stopPreview()
        previewingVoice = voice
        lastError = nil
        previewTask = Task { [weak self] in
            guard let self else { return }
            let output = FileManager.default.temporaryDirectory
                .appendingPathComponent("astroshots-voice-preview-\(voice).wav")
            do {
                try? FileManager.default.removeItem(at: output)
                _ = try await synthesizePhrase(
                    NarrationDefaults.voicePreviewText,
                    voice: voice,
                    to: output
                )
                try Task.checkCancellation()
                let player = try AVAudioPlayer(contentsOf: output)
                player.prepareToPlay()
                previewPlayer = player
                player.play()
                let nanoseconds = UInt64((player.duration + 0.15) * 1_000_000_000)
                try await Task.sleep(nanoseconds: nanoseconds)
                if previewingVoice == voice { previewingVoice = nil }
            } catch is CancellationError {
                // A second tap or a new voice selection stops the current sample.
            } catch {
                lastError = error.localizedDescription
                previewingVoice = nil
            }
        }
    }

    func stopPreview() {
        previewTask?.cancel()
        previewTask = nil
        previewPlayer?.stop()
        previewPlayer = nil
        previewingVoice = nil
    }

    private func downloadModel(force: Bool) async throws {
        guard let repoID = Repo.ID(rawValue: NarrationDefaults.modelID) else {
            throw NarrationError.invalidModelRepo
        }
        if force {
            let dir = NarrationPaths.mlxAudioModelDirectory()
            try? FileManager.default.removeItem(at: dir)
        }

        _ = try await ModelUtils.resolveOrDownloadModel(
            client: HubClient(cache: .default),
            cache: .default,
            repoID: repoID,
            requiredExtension: "safetensors",
            progressHandler: { [weak self] progress in
                let fraction: Double
                if progress.totalUnitCount > 0 {
                    fraction = min(
                        0.99,
                        max(0.02, Double(progress.completedUnitCount) / Double(progress.totalUnitCount))
                    )
                } else {
                    fraction = 0.15
                }
                self?.readiness = .downloadingModel(fraction)
            }
        )
    }

    nonisolated static func isAppleSilicon() -> Bool {
        #if arch(arm64)
        return true
        #else
        var value: Int32 = 0
        var size = MemoryLayout<Int32>.size
        let result = sysctlbyname("hw.optional.arm64", &value, &size, nil, 0)
        return result == 0 && value == 1
        #endif
    }

    nonisolated static func physicalMemoryGB() -> Double {
        Double(ProcessInfo.processInfo.physicalMemory) / (1024.0 * 1024.0 * 1024.0)
    }
}

/// Serial TTS access for non-Sendable `SpeechGenerationModel` instances.
final class NarrationSpeechBox: @unchecked Sendable {
    private let model: any SpeechGenerationModel

    init(_ model: any SpeechGenerationModel) {
        self.model = model
    }

    var sampleRate: Int { model.sampleRate }

    func speak(_ text: String, voice: String = NarrationDefaults.voice) async throws -> [Float] {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let audio: MLXArray = try await model.generate(
            text: trimmed,
            voice: NarrationVoice.normalized(voice),
            refAudio: nil,
            refText: nil,
            language: NarrationDefaults.language
        )
        return audio.asArray(Float.self)
    }
}
