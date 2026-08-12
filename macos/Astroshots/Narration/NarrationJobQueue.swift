import AppKit
import Foundation
import MLXAudioCore
import MLXAudioTTS
import Observation

/// Serializes narrated-video jobs so only one MLX TTS + encode runs at a time.
@MainActor
@Observable
final class NarrationJobQueue {
    static let shared = NarrationJobQueue()

    private(set) var jobs: [NarrationJob] = []
    private(set) var activeJobID: UUID?

    private let modelManager: NarrationModelManager
    private var worker: Task<Void, Never>?

    init(modelManager: NarrationModelManager = .shared) {
        self.modelManager = modelManager
    }

    var activeJob: NarrationJob? {
        guard let activeJobID else { return nil }
        return jobs.first { $0.id == activeJobID }
    }

    func job(forRunDirectory path: String) -> NarrationJob? {
        jobs
            .filter { $0.runDirectoryPath == path }
            .sorted { $0.createdAt > $1.createdAt }
            .first
    }

    @discardableResult
    func enqueue(
        log: FrictionLog,
        run: FrictionLogRun,
        voice: String = NarrationDefaults.voice
    ) throws -> NarrationJob {
        guard modelManager.readiness.isReady else {
            throw NarrationError.notReady
        }
        guard !run.steps.isEmpty else {
            throw NarrationError.noSteps
        }
        if let existing = job(forRunDirectory: run.directoryPath),
           existing.phase == .queued || existing.phase == .synthesizing
               || existing.phase == .encoding
        {
            return existing
        }

        let job = NarrationJob.make(log: log, run: run, voice: voice)
        jobs.insert(job, at: 0)
        if jobs.count > 40 {
            jobs = Array(jobs.prefix(40))
        }
        kick()
        return job
    }

    func cancel(_ id: UUID) {
        guard let index = jobs.firstIndex(where: { $0.id == id }) else { return }
        if jobs[index].phase == .queued {
            jobs[index].phase = .cancelled
            jobs[index].message = "Cancelled"
        }
        if activeJobID == id {
            worker?.cancel()
            worker = nil
            jobs[index].phase = .cancelled
            jobs[index].message = "Cancelled"
            activeJobID = nil
            kick()
        }
    }

    func revealOutput(for id: UUID) {
        guard let path = jobs.first(where: { $0.id == id })?.outputPath else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    private func kick() {
        guard worker == nil else { return }
        worker = Task { [weak self] in
            await self?.drain()
        }
    }

    private func drain() async {
        while let next = nextQueued() {
            activeJobID = next.id
            update(id: next.id) {
                $0.phase = .synthesizing
                $0.progress = 0.02
                $0.message = "Loading Qwen3-TTS…"
            }
            do {
                try Task.checkCancellation()
                let output = try await render(job: next)
                update(id: next.id) {
                    $0.phase = .complete
                    $0.progress = 1
                    $0.message = "Narrated video ready"
                    $0.outputPath = output.path
                }
            } catch is CancellationError {
                update(id: next.id) {
                    $0.phase = .cancelled
                    $0.message = "Cancelled"
                }
            } catch {
                let message = (error as? LocalizedError)?.errorDescription
                    ?? error.localizedDescription
                update(id: next.id) {
                    $0.phase = .failed
                    $0.message = message
                }
            }
            activeJobID = nil
        }
        worker = nil
    }

    private func nextQueued() -> NarrationJob? {
        // Newest jobs are inserted at index 0; drain oldest queued first (FIFO).
        jobs.last(where: { $0.phase == .queued })
    }

    private func update(id: UUID, mutate: (inout NarrationJob) -> Void) {
        guard let index = jobs.firstIndex(where: { $0.id == id }) else { return }
        mutate(&jobs[index])
    }

    private func render(job: NarrationJob) async throws -> URL {
        NarrationPaths.ensureDirectories()
        let workDir = NarrationPaths.workRoot
            .appendingPathComponent(job.id.uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: workDir, withIntermediateDirectories: true)

        let model = try await modelManager.modelForSynthesis()
        let total = max(job.stepTranscripts.count, 1)
        var media: [NarrationVideoComposer.StepMedia] = []

        for (index, step) in job.stepTranscripts.enumerated() {
            try Task.checkCancellation()
            let base = Double(index) / Double(total)
            let span = 0.75 / Double(total)
            update(id: job.id) {
                $0.phase = .synthesizing
                $0.progress = base + span * 0.15
                $0.message = "Speaking step \(step.step): \(step.title)"
            }

            let wav = workDir.appendingPathComponent(
                String(format: "step-%02d.wav", step.step)
            )
            let duration = try await synthesize(
                model: model,
                text: step.transcript,
                voice: job.voice,
                output: wav
            )
            update(id: job.id) {
                $0.progress = base + span
                $0.message = "Step \(step.step) audio ready"
            }

            media.append(
                NarrationVideoComposer.StepMedia(
                    imagePath: step.imagePath,
                    audioPath: wav.path,
                    duration: duration,
                    title: step.title,
                    transcript: step.transcript,
                    good: step.good,
                    improve: step.improve
                )
            )
        }

        try Task.checkCancellation()
        update(id: job.id) {
            $0.phase = .encoding
            $0.progress = 0.8
            $0.message = "Encoding MP4…"
        }

        let fileTitle = Self.safeFilenameComponent(job.logTitle)
        let output = URL(fileURLWithPath: job.runDirectoryPath)
            .appendingPathComponent("Astroshots-\(fileTitle)-\(job.runID).mp4")
        try await NarrationVideoComposer.compose(
            steps: media,
            outputURL: output,
            logTitle: job.logTitle
        )

        update(id: job.id) {
            $0.progress = 0.98
            $0.message = "Finishing…"
        }
        return output
    }

    private func synthesize(
        model: any SpeechGenerationModel,
        text: String,
        voice: String,
        output: URL
    ) async throws -> Double {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw NarrationError.processFailed("Empty transcript for a step.")
        }

        let audio = try await modelManager.synthesizeAudio(
            trimmed,
            voice: voice,
            using: model
        )
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

        let duration = Double(samples.count) / Double(max(audio.sampleRate, 1))
        return max(duration, NarrationDefaults.minimumStepSeconds)
    }

    nonisolated static func safeFilenameComponent(_ title: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_"))
        let words = title
            .components(separatedBy: allowed.inverted)
            .filter { !$0.isEmpty }
        let value = words.joined(separator: "-")
        return value.isEmpty ? "Friction-Log" : String(value.prefix(80))
    }
}
