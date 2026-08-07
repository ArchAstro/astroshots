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

    private func runPicker(log: FrictionLog, selected: FrictionLogRun) -> some View {
        Group {
            if log.runs.count > 1 {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(log.runs) { run in
                            Button {
                                appState.selectFrictionRun(run)
                            } label: {
                                Text(run.runID)
                                    .font(.system(size: 10, weight: .semibold, design: .monospaced))
                                    .foregroundStyle(
                                        run.id == selected.id ? Theme.purple : Theme.ink2
                                    )
                                    .padding(.horizontal, 9)
                                    .padding(.vertical, 5)
                                    .background(
                                        run.id == selected.id ? Theme.purpleSoft : Theme.surface,
                                        in: Capsule()
                                    )
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            } else {
                HStack(spacing: 6) {
                    Image(systemName: "play.circle.fill")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.purple)
                    Text(selected.runID)
                        .font(.system(size: 11, weight: .semibold, design: .monospaced))
                        .foregroundStyle(Theme.ink2)
                    Spacer()
                    Text("\(selected.steps.count) steps")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(Theme.muted)
                }
            }
        }
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
