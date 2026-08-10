import SwiftUI

/// Friction-log scenario page: header, prompt, run picker, and step table.
/// Clicking a step opens `FrictionStepDetailView` (not an inline panel).
struct FrictionLogDetailView: View {
    @Environment(AppState.self) private var appState
    @State private var showPrompt = false

    var body: some View {
        VStack(spacing: 0) {
            chrome
            if let log = appState.selectedFrictionLog {
                content(for: log)
            } else {
                EmptyFrictionLogsView()
            }
        }
    }

    private var chrome: some View {
        HStack(spacing: 6) {
            Button {
                appState.backToFrictionLogs()
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 10, weight: .semibold))
                    Text("Logs")
                        .font(.system(size: 11, weight: .semibold))
                }
                .foregroundStyle(Theme.ink2)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Spacer()

            if let run = appState.selectedFrictionRun {
                Text("\(run.steps.count) steps")
                    .font(.system(size: 10, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Theme.muted2)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.line).frame(height: 1)
        }
    }

    @ViewBuilder
    private func content(for log: FrictionLog) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                header(for: log)
                if showPrompt, let markdown = log.promptMarkdown, !markdown.isEmpty {
                    promptCard(markdown)
                }
                if let run = appState.selectedFrictionRun {
                    runPicker(log: log, selected: run)
                    narrationCard(log: log, run: run)
                    improveRollup(run: run)
                    stepsTable(run: run)
                } else {
                    noRunsCard
                }
            }
            .padding(.horizontal, 12)
            .padding(.top, 12)
            .padding(.bottom, 16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.hidden)
        .accessibilityIdentifier("friction.detail")
    }

    private func header(for log: FrictionLog) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 8) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(log.title)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(Theme.ink)
                        .tracking(-0.3)
                        .fixedSize(horizontal: false, vertical: true)
                    if !log.description.isEmpty {
                        Text(log.description)
                            .font(.system(size: 11))
                            .foregroundStyle(Theme.ink2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 4)
                if let status = log.status {
                    FrictionStatusBadge(status: status)
                }
            }

            HStack(spacing: 6) {
                WorktreeChip(label: log.worktreeShort)
                Text(log.slug)
                    .font(.system(size: 10, weight: .medium, design: .monospaced))
                    .foregroundStyle(Theme.muted2)
                Spacer(minLength: 4)
                if log.promptMarkdown != nil {
                    Button {
                        withAnimation(.easeOut(duration: 0.15)) {
                            showPrompt.toggle()
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "doc.plaintext")
                                .font(.system(size: 10, weight: .semibold))
                            Text(showPrompt ? "Hide prompt" : "Prompt")
                                .font(.system(size: 10, weight: .semibold))
                        }
                        .foregroundStyle(showPrompt ? Theme.purple : Theme.ink2)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(
                            showPrompt ? Theme.purpleSoft : Theme.surface,
                            in: Capsule()
                        )
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("friction.prompt.toggle")
                }
            }
        }
    }

    private func promptCard(_ markdown: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Scenario prompt")
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(Theme.muted)
                .textCase(.uppercase)
                .tracking(0.4)
            Text(markdown)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Theme.ink2)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(11)
        .background(
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .fill(Color.white.opacity(0.7))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .stroke(Theme.line, lineWidth: 1)
        )
        .accessibilityIdentifier("friction.prompt.body")
    }

    @ViewBuilder
    private func improveRollup(run: FrictionLogRun) -> some View {
        let improves = run.steps.flatMap { step in
            step.improve.map { note in (step: step, note: note) }
        }
        if !improves.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.bubble.fill")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(Theme.amber)
                    Text("Improve rollup · \(improves.count)")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.ink)
                    Spacer()
                }
                ForEach(Array(improves.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .top, spacing: 6) {
                        Text(String(format: "%02d", item.step.step))
                            .font(.system(size: 9, weight: .bold, design: .monospaced))
                            .foregroundStyle(Theme.amber)
                        Text(item.note)
                            .font(.system(size: 10.5))
                            .foregroundStyle(Theme.ink2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.amberSoft.opacity(0.65), in: RoundedRectangle(cornerRadius: 10))
            .accessibilityIdentifier("friction.improve.rollup")
        }
    }

    /// Run history control. Matches `StreamFilterChip`: 28pt pills, radius 6,
    /// purpleSoft selection — no nested card (chips sit on paper like stream filters).
    @ViewBuilder
    private func narrationCard(log: FrictionLog, run: FrictionLogRun) -> some View {
        let readiness = appState.narration.readiness
        let job = appState.narrationQueue.job(forRunDirectory: run.directoryPath)

        // Only surface once the user has opted in (or a job already exists).
        if readiness == .disabled, job == nil {
            EmptyView()
        } else {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 6) {
                    Image(systemName: "film.stack")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.purple)
                    Text("Narrated video")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.ink)
                    Spacer(minLength: 4)
                    if let job, job.phase == .complete, job.outputPath != nil {
                        Text("Ready")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(Theme.green)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Theme.greenSoft, in: Capsule())
                    }
                }

                if readiness.isReady {
                    Text(
                        "Builds an MP4 that walks each step’s screenshot with the spoken transcript."
                    )
                    .font(.system(size: 10))
                    .foregroundStyle(Theme.ink2)
                    .fixedSize(horizontal: false, vertical: true)

                    if let job, job.phase == .queued || job.phase == .synthesizing
                        || job.phase == .encoding
                    {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(job.message)
                                .font(.system(size: 10.5, weight: .medium))
                                .foregroundStyle(Theme.ink2)
                            ProgressView(value: job.progress)
                                .progressViewStyle(.linear)
                                .tint(Theme.purple)
                            HStack {
                                Spacer()
                                Button("Cancel") {
                                    appState.narrationQueue.cancel(job.id)
                                }
                                .buttonStyle(.plain)
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundStyle(Theme.muted)
                            }
                        }
                    } else if let job, job.phase == .complete, let path = job.outputPath {
                        HStack(spacing: 8) {
                            Button("Show in Finder") {
                                appState.narrationQueue.revealOutput(for: job.id)
                            }
                            .buttonStyle(.plain)
                            .font(.system(size: 11, weight: .semibold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(Theme.purpleSoft, in: RoundedRectangle(cornerRadius: 7))
                            .foregroundStyle(Theme.purple)

                            Button("Render again") {
                                _ = appState.enqueueNarration(for: log, run: run)
                            }
                            .buttonStyle(.plain)
                            .font(.system(size: 11, weight: .semibold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))

                            Spacer(minLength: 0)
                        }
                        Text((path as NSString).lastPathComponent)
                            .font(.system(size: 9.5, design: .monospaced))
                            .foregroundStyle(Theme.muted2)
                            .lineLimit(1)
                    } else if let job, job.phase == .failed {
                        Text(job.message)
                            .font(.system(size: 10.5))
                            .foregroundStyle(Theme.red)
                            .fixedSize(horizontal: false, vertical: true)
                        Button("Retry narration") {
                            _ = appState.enqueueNarration(for: log, run: run)
                        }
                        .buttonStyle(.plain)
                        .font(.system(size: 11, weight: .semibold))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))
                    } else {
                        Button {
                            _ = appState.enqueueNarration(for: log, run: run)
                        } label: {
                            HStack(spacing: 6) {
                                Image(systemName: "waveform")
                                    .font(.system(size: 11, weight: .semibold))
                                Text("Make narrated video")
                                    .font(.system(size: 11, weight: .semibold))
                            }
                            .foregroundStyle(Theme.purple)
                            .padding(.horizontal, 11)
                            .padding(.vertical, 7)
                            .background(Theme.purpleSoft, in: RoundedRectangle(cornerRadius: 8))
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("friction.narration.make")
                    }
                } else if readiness == .disabled {
                    Text("Turn on Narration in Settings to generate step voiceovers with Qwen3-TTS.")
                        .font(.system(size: 10))
                        .foregroundStyle(Theme.muted)
                } else if readiness.isBusy {
                    Text(readiness.statusLine)
                        .font(.system(size: 10.5, weight: .medium))
                        .foregroundStyle(Theme.ink2)
                    if let fraction = readiness.progressFraction {
                        ProgressView(value: fraction)
                            .progressViewStyle(.linear)
                            .tint(Theme.purple)
                    }
                } else {
                    Text(readiness.statusLine)
                        .font(.system(size: 10.5))
                        .foregroundStyle(Theme.muted)
                    Button("Open Settings") {
                        appState.openSettings()
                    }
                    .buttonStyle(.plain)
                    .font(.system(size: 11, weight: .semibold))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))
                }
            }
            .padding(11)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                Color.white.opacity(0.72),
                in: RoundedRectangle(cornerRadius: 12, style: .continuous)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(Theme.line, lineWidth: 1)
            )
            .accessibilityIdentifier("friction.narration.card")
        }
    }

    /// Run history: single-line menu selector (scales to many runs; no h-scroll pills).
    private func runPicker(log: FrictionLog, selected: FrictionLogRun) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(log.runCount == 1 ? "Run" : "Runs")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(Theme.muted)
                    .textCase(.uppercase)
                    .tracking(0.4)
                if log.runCount > 1 {
                    Text("\(log.runCount)")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .foregroundStyle(Theme.muted2)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Theme.surface, in: Capsule())
                }
                Spacer(minLength: 4)
                Text(selected.stepCountLabel)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(Theme.muted2)
                    .help(selected.runID)
            }

            if log.runCount == 1 {
                HStack(spacing: 8) {
                    Text(selected.displayTitle)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.ink)
                    Text(selected.runID)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundStyle(Theme.muted2)
                        .lineLimit(1)
                        .textSelection(.enabled)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 9)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    Color.white.opacity(0.72),
                    in: RoundedRectangle(cornerRadius: 12, style: .continuous)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(Theme.line, lineWidth: 1)
                )
            } else {
                // Full-width menu selector — closed label stays compact; menu lists all runs.
                Menu {
                    ForEach(Array(log.runs.enumerated()), id: \.element.id) { index, run in
                        let isLatest = index == 0
                        Button {
                            appState.selectFrictionRun(run)
                        } label: {
                            if run.id == selected.id {
                                Label(runMenuLabel(run: run, isLatest: isLatest), systemImage: "checkmark")
                            } else {
                                Text(runMenuLabel(run: run, isLatest: isLatest))
                            }
                        }
                        .accessibilityIdentifier("friction.run.\(run.runID)")
                    }
                } label: {
                    HStack(spacing: 8) {
                        Text(selected.displayTitle)
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(Theme.ink)
                            .lineLimit(1)
                        if selected.runID == log.latestRun?.runID {
                            Text("Latest")
                                .font(.system(size: 9, weight: .bold))
                                .foregroundStyle(Theme.purple)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Theme.purpleSoft, in: Capsule())
                        }
                        Spacer(minLength: 4)
                        Image(systemName: "chevron.up.chevron.down")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(Theme.muted2)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 9)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .menuStyle(.borderlessButton)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    Color.white.opacity(0.72),
                    in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(Theme.line, lineWidth: 1)
                )
                .help(selected.runID)
                .accessibilityIdentifier("friction.runs.picker")
                .accessibilityLabel(
                    "Run \(selected.displayTitle), \(log.runCount) runs available"
                )
            }
        }
    }

    private func runMenuLabel(run: FrictionLogRun, isLatest: Bool) -> String {
        var parts = [run.displayTitle]
        if isLatest { parts.append("Latest") }
        parts.append(run.stepCountLabel)
        // Full id in the open menu for disambiguation.
        return "\(parts.joined(separator: " · "))  ·  \(run.runID)"
    }

    private func stepsTable(run: FrictionLogRun) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Steps")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(Theme.muted)
                    .textCase(.uppercase)
                    .tracking(0.4)
                Spacer()
                Text("Tap a step for notes")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(Theme.muted2)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)

            Rectangle().fill(Theme.line).frame(height: 1)

            if run.steps.isEmpty {
                Text("No JSONL steps in this run")
                    .font(.system(size: 11))
                    .foregroundStyle(Theme.muted)
                    .padding(12)
            } else {
                ForEach(Array(run.steps.enumerated()), id: \.element.id) { index, step in
                    if index > 0 {
                        Rectangle().fill(Theme.line).frame(height: 1)
                    }
                    stepRow(step)
                }
            }
        }
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Color.white.opacity(0.72))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Theme.line, lineWidth: 1)
        )
        .accessibilityIdentifier("friction.steps.table")
    }

    private func stepRow(_ step: FrictionLogStep) -> some View {
        Button {
            appState.selectFrictionStep(step)
        } label: {
            HStack(alignment: .top, spacing: 10) {
                Text(String(format: "%02d", step.step))
                    .font(.system(size: 11, weight: .bold, design: .monospaced))
                    .foregroundStyle(Theme.muted2)
                    .frame(width: 22, alignment: .leading)

                VStack(alignment: .leading, spacing: 3) {
                    Text(step.title)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.ink)
                        .multilineTextAlignment(.leading)
                        .lineLimit(2)
                    if !step.description.isEmpty {
                        Text(step.description)
                            .font(.system(size: 10))
                            .foregroundStyle(Theme.ink2)
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                    }
                }

                Spacer(minLength: 4)

                VStack(alignment: .trailing, spacing: 4) {
                    HStack(spacing: 4) {
                        if step.hasTranscript {
                            Image(systemName: "text.bubble.fill")
                                .font(.system(size: 9, weight: .semibold))
                                .foregroundStyle(Theme.purple.opacity(0.85))
                        }
                        if !step.screenshotPaths.isEmpty {
                            Image(systemName: "camera.fill")
                                .font(.system(size: 9, weight: .semibold))
                                .foregroundStyle(Theme.muted2)
                        }
                        Image(systemName: "chevron.right")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(Theme.muted2)
                    }
                    HStack(spacing: 4) {
                        if !step.good.isEmpty {
                            chip("\(step.good.count)", color: Theme.green, bg: Theme.greenSoft)
                        }
                        if !step.improve.isEmpty {
                            chip("\(step.improve.count)", color: Theme.amber, bg: Theme.amberSoft)
                        }
                    }
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("friction.step.\(step.step)")
    }

    private func chip(_ text: String, color: Color, bg: Color) -> some View {
        Text(text)
            .font(.system(size: 9, weight: .bold, design: .monospaced))
            .foregroundStyle(color)
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(bg, in: Capsule())
    }

    private var noRunsCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("No runs yet")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.ink)
            Text(
                "This scenario has a prompt but no log.jsonl run. Use the friction-log skill to execute it; steps will appear here as the agent writes them."
            )
            .font(.system(size: 11))
            .foregroundStyle(Theme.ink2)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.amberSoft.opacity(0.55), in: RoundedRectangle(cornerRadius: 12))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Theme.amber.opacity(0.2), lineWidth: 1)
        )
    }
}
