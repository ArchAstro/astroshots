import Foundation

/// Pure speech-prep helpers for Qwen3-TTS: chunk long transcripts, trim the
/// silence the model leaves after it stops talking, and attach a pace prompt.
enum NarrationSpeech {
    static let targetChunkWords = 24
    static let maxChunkWords = 42
    static let silenceRMSThreshold: Float = 0.01
    static let analysisWindowSeconds = 0.05
    static let leadingPadSeconds = 0.08
    static let trailingPadSeconds = 0.2
    static let interChunkSilenceSeconds = 0.12
    static let paceInstruction =
        "speaking at a natural conversational pace, like a colleague walking through a product"
    /// ~3s CustomVoice line, then cloned via ICL so every later chunk shares one timbre.
    static let referenceText = "This is the narrator for this Astroshots friction log."

    static func voicePrompt(speaker: String) -> String {
        let speaker = NarrationVoice.normalized(speaker)
        return "\(speaker), \(paceInstruction)"
    }

    static func chunks(for text: String) -> [String] {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return [] }

        var result: [String] = []
        var current: [String] = []
        var currentWords = 0

        func flush() {
            let joined = current.joined(separator: " ")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !joined.isEmpty { result.append(joined) }
            current = []
            currentWords = 0
        }

        for piece in clauses(in: trimmed) {
            let words = wordCount(piece)
            guard words > 0 else { continue }
            if !current.isEmpty {
                let combined = currentWords + words
                if combined > maxChunkWords || currentWords >= targetChunkWords {
                    flush()
                }
            }
            current.append(piece)
            currentWords += words
        }
        if !current.isEmpty { flush() }
        return result
    }

    /// Drops leading/trailing near-silence so captions and the step hold follow
    /// speech, not the leftover WAV the model fills up to its token cap.
    static func trimSilence(_ samples: [Float], sampleRate: Int) -> [Float] {
        guard !samples.isEmpty, sampleRate > 0 else { return samples }
        let window = max(1, Int(Double(sampleRate) * analysisWindowSeconds))

        func rms(at start: Int) -> Float {
            let end = min(samples.count, start + window)
            guard end > start else { return 0 }
            var sum: Float = 0
            for index in start ..< end {
                let sample = samples[index]
                sum += sample * sample
            }
            return sqrt(sum / Float(end - start))
        }

        var first = 0
        var foundSpeech = false
        var index = 0
        while index < samples.count {
            if rms(at: index) > silenceRMSThreshold {
                first = index
                foundSpeech = true
                break
            }
            index += window
        }
        guard foundSpeech else { return samples }

        var last = samples.count
        index = ((samples.count - 1) / window) * window
        while index >= 0 {
            if rms(at: index) > silenceRMSThreshold {
                last = min(samples.count, index + window)
                break
            }
            index -= window
        }
        guard last > first else { return samples }

        let lead = Int(leadingPadSeconds * Double(sampleRate))
        let tail = Int(trailingPadSeconds * Double(sampleRate))
        let start = max(0, first - lead)
        let end = min(samples.count, last + tail)
        return Array(samples[start ..< end])
    }

    static func interChunkSilence(sampleRate: Int) -> [Float] {
        let count = max(0, Int(interChunkSilenceSeconds * Double(max(sampleRate, 1))))
        return Array(repeating: 0, count: count)
    }

    static func wordCount(_ text: String) -> Int {
        text.split(whereSeparator: { $0.isWhitespace }).count
    }

    /// Sentences first, then long sentences on commas / semicolons / dashes.
    static func clauses(in text: String) -> [String] {
        var sentences: [String] = []
        var current = ""
        let scalars = Array(text)
        var index = 0
        while index < scalars.count {
            let character = scalars[index]
            current.append(character)
            let isEnd = character == "." || character == "!" || character == "?"
            let nextIsSpaceOrEnd = index + 1 >= scalars.count
                || scalars[index + 1].isWhitespace
            if isEnd, nextIsSpaceOrEnd {
                let piece = current.trimmingCharacters(in: .whitespacesAndNewlines)
                if !piece.isEmpty { sentences.append(piece) }
                current = ""
            }
            index += 1
        }
        let tail = current.trimmingCharacters(in: .whitespacesAndNewlines)
        if !tail.isEmpty { sentences.append(tail) }

        return sentences.flatMap { sentence -> [String] in
            guard wordCount(sentence) > maxChunkWords else { return [sentence] }
            return splitLongSentence(sentence)
        }
    }

    private static func splitLongSentence(_ sentence: String) -> [String] {
        let separators = CharacterSet(charactersIn: ",;—–")
        let raw = sentence.components(separatedBy: separators)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard raw.count > 1 else { return [sentence] }

        var parts: [String] = []
        var current: [String] = []
        var currentWords = 0
        for piece in raw {
            let words = wordCount(piece)
            if !current.isEmpty, currentWords + words > maxChunkWords {
                parts.append(current.joined(separator: ", ") + ".")
                current = []
                currentWords = 0
            }
            current.append(piece)
            currentWords += words
        }
        if !current.isEmpty {
            var joined = current.joined(separator: ", ")
            if !joined.hasSuffix(".") && !joined.hasSuffix("!") && !joined.hasSuffix("?") {
                joined += "."
            }
            parts.append(joined)
        }
        return parts
    }
}
