import Foundation

/// Setup card after the intro: who this person is and what they came to do.
struct NarrationBrief: Equatable, Sendable {
    var title: String
    var persona: String
    var goal: String
    var transcript: String

    var hasContent: Bool {
        !persona.isEmpty || !goal.isEmpty
    }

    static func make(log: FrictionLog) -> NarrationBrief {
        make(
            title: log.title,
            description: log.description,
            promptMarkdown: log.promptMarkdown
        )
    }

    static func make(
        title: String,
        description: String,
        promptMarkdown: String?
    ) -> NarrationBrief {
        let persona = clean(section("persona", in: promptMarkdown ?? ""))
        let goal = clean(section("goal", in: promptMarkdown ?? ""))
        let resolvedGoal = goal.isEmpty ? clean(description) : goal
        return NarrationBrief(
            title: title.trimmingCharacters(in: .whitespacesAndNewlines),
            persona: persona,
            goal: resolvedGoal,
            transcript: spokenTranscript(persona: persona, goal: resolvedGoal)
        )
    }

    static func spokenTranscript(persona: String, goal: String) -> String {
        var parts: [String] = []
        if !persona.isEmpty { parts.append(persona) }
        if !goal.isEmpty { parts.append(goal) }
        guard !parts.isEmpty else { return "" }
        parts.append("We follow them through it.")
        return parts.joined(separator: " ")
    }

    /// Body of `## Heading` until the next ATX heading. Skips `$ENV` bullets.
    static func section(_ name: String, in markdown: String) -> String {
        let wanted = name.lowercased()
        var capturing = false
        var lines: [String] = []
        for raw in markdown.components(separatedBy: .newlines) {
            let trimmed = raw.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("#") {
                let heading = trimmed
                    .trimmingCharacters(in: CharacterSet(charactersIn: "#").union(.whitespaces))
                    .lowercased()
                if capturing { break }
                capturing = heading == wanted || heading.hasPrefix(wanted + " ")
                continue
            }
            guard capturing, !trimmed.isEmpty else { continue }
            if trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") {
                let item = String(trimmed.dropFirst(2))
                    .trimmingCharacters(in: .whitespaces)
                if item.contains("$") { continue }
                lines.append(item)
            } else {
                lines.append(trimmed)
            }
        }
        return lines.joined(separator: " ")
    }

    private static func clean(_ text: String) -> String {
        text
            .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
