import Foundation

/// Default Qwen3-TTS MLX checkpoint used by mlx-audio-swift.
enum NarrationDefaults {
    static let modelID = "mlx-community/Qwen3-TTS-12Hz-0.6B-Base-8bit"
    /// Base/CustomVoice English preset (see Qwen3-TTS README).
    static let voice = "Ryan"
    static let language = "English"
    /// Minimum step display duration when audio is shorter.
    static let minimumStepSeconds: Double = 1.6
    static let framesPerSecond: Int32 = 30
    static let minMemoryGB: Double = 8.0
}

enum NarrationReadiness: Equatable, Sendable {
    case disabled
    case unsupported(String)
    case checking
    case downloadingModel(Double)
    case loadingModel
    case ready
    case failed(String)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }

    var isBusy: Bool {
        switch self {
        case .checking, .downloadingModel, .loadingModel:
            return true
        default:
            return false
        }
    }

    var progressFraction: Double? {
        switch self {
        case let .downloadingModel(f):
            return f
        case .loadingModel:
            return nil
        default:
            return nil
        }
    }

    var statusLine: String {
        switch self {
        case .disabled:
            return "Off"
        case let .unsupported(reason):
            return reason
        case .checking:
            return "Checking this Mac…"
        case let .downloadingModel(f):
            return "Downloading Qwen3-TTS… \(Int(f * 100))%"
        case .loadingModel:
            return "Loading Qwen3-TTS into MLX…"
        case .ready:
            return "Ready"
        case let .failed(message):
            return message
        }
    }
}

struct NarrationCapability: Equatable, Sendable {
    var isAppleSilicon: Bool
    var memoryGB: Double
    var readyForTTS: Bool
    var unsupportedReason: String?

    static let unknown = NarrationCapability(
        isAppleSilicon: false,
        memoryGB: 0,
        readyForTTS: false,
        unsupportedReason: "Not checked yet"
    )
}

enum NarrationJobPhase: String, Equatable, Sendable {
    case queued
    case synthesizing
    case encoding
    case complete
    case failed
    case cancelled
}

struct NarrationStepInput: Equatable, Sendable {
    var step: Int
    var title: String
    var transcript: String
    var imagePath: String?
}

struct NarrationJob: Identifiable, Equatable, Sendable {
    let id: UUID
    let logTitle: String
    let logSlug: String
    let runID: String
    let runDirectoryPath: String
    let stepTranscripts: [NarrationStepInput]
    var phase: NarrationJobPhase
    var progress: Double
    var message: String
    var outputPath: String?
    var createdAt: Date

    var displayLabel: String {
        "\(logTitle) · \(runID)"
    }
}

extension NarrationJob {
    static func make(
        log: FrictionLog,
        run: FrictionLogRun
    ) -> NarrationJob {
        let steps = run.steps.map { step in
            NarrationStepInput(
                step: step.step,
                title: step.title,
                transcript: step.hasTranscript
                    ? step.transcript
                    : (step.description.isEmpty ? step.title : step.description),
                imagePath: step.primaryScreenshotPath
            )
        }
        return NarrationJob(
            id: UUID(),
            logTitle: log.title,
            logSlug: log.slug,
            runID: run.runID,
            runDirectoryPath: run.directoryPath,
            stepTranscripts: steps,
            phase: .queued,
            progress: 0,
            message: "Queued",
            outputPath: nil,
            createdAt: Date()
        )
    }
}

enum NarrationError: LocalizedError {
    case notReady
    case processFailed(String)
    case noSteps
    case cancelled
    case invalidModelRepo

    var errorDescription: String? {
        switch self {
        case .notReady:
            return "Narration is not ready. Enable it in Settings and wait for the model download."
        case let .processFailed(message):
            return message
        case .noSteps:
            return "This run has no steps to narrate."
        case .cancelled:
            return "Narration cancelled."
        case .invalidModelRepo:
            return "Invalid Hugging Face model id for Qwen3-TTS."
        }
    }
}
