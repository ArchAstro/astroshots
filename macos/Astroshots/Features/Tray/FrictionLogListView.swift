import SwiftUI

/// Catalog of authored friction-log scenarios across watched worktrees.
struct FrictionLogListView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        if appState.isFrictionLogsEmpty {
            EmptyFrictionLogsView()
        } else {
            ScrollView {
                LazyVStack(spacing: 8) {
                    ForEach(appState.frictionLogs) { log in
                        FrictionLogRow(
                            log: log,
                            action: { appState.selectFrictionLog(log) },
                            hideAction: { appState.hideFrictionLog(log) }
                        )
                    }
                }
                .padding(.horizontal, 10)
                .padding(.top, 10)
                .padding(.bottom, 8)
            }
            .scrollIndicators(.hidden)
        }
    }
}

private struct FrictionLogRow: View {
    let log: FrictionLog
    let action: () -> Void
    let hideAction: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            Button(action: action) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .top, spacing: 8) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(log.title)
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(Theme.ink)
                                .lineLimit(2)
                                .multilineTextAlignment(.leading)
                            if !log.description.isEmpty {
                                Text(log.description)
                                    .font(.system(size: 11))
                                    .foregroundStyle(Theme.ink2)
                                    .lineLimit(2)
                                    .multilineTextAlignment(.leading)
                            }
                        }
                        Spacer(minLength: 4)
                        if let status = log.status {
                            FrictionStatusBadge(status: status)
                        }
                    }

                    // Quiet meta row — worktree + slug only; counts live as plain text.
                    HStack(spacing: 6) {
                        WorktreeChip(label: log.worktreeShort)
                        Text(log.slug)
                            .font(.system(size: 10, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.muted2)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Text(metaSummary(for: log))
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(Theme.muted)
                            .lineLimit(1)
                    }

                    if let run = log.latestRun {
                        HStack(spacing: 6) {
                            Text(runFooter(log: log, run: run))
                                .font(.system(size: 10, weight: .medium))
                                .foregroundStyle(Theme.ink2)
                                .lineLimit(1)
                            Spacer(minLength: 4)
                            Text(relativeTime(run.capturedAt))
                                .font(.system(size: 10, weight: .medium))
                                .foregroundStyle(Theme.muted)
                        }
                        .help(run.runID)
                    } else {
                        Text("Prompt only · no runs yet")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(Theme.muted)
                    }
                }
                .padding(11)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("friction.row.\(log.slug)")

            Button(action: hideAction) {
                Image(systemName: "eye.slash")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.muted)
                    .frame(width: 30, height: 30)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Hide friction log")
            .accessibilityLabel("Hide \(log.title)")
            .accessibilityIdentifier("friction.hide.\(log.slug)")
            .padding(.trailing, 4)
        }
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Theme.elevated.opacity(0.72))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Theme.line, lineWidth: 1)
        )
    }

    private func metaSummary(for log: FrictionLog) -> String {
        var parts: [String] = []
        if log.stepCount > 0 {
            parts.append(log.stepCount == 1 ? "1 step" : "\(log.stepCount) steps")
        }
        if log.improveCount > 0 {
            parts.append("\(log.improveCount) improve")
        }
        return parts.joined(separator: " · ")
    }

    private func runFooter(log: FrictionLog, run: FrictionLogRun) -> String {
        if log.runCount > 1 {
            return "\(log.runCount) runs · \(run.displayTitle)"
        }
        return run.displayTitle
    }

    private func relativeTime(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: Date())
    }
}

struct FrictionStatusBadge: View {
    let status: FrictionLogStatus

    var body: some View {
        Text(status.label)
            .font(.system(size: 9, weight: .bold))
            .foregroundStyle(fg)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(bg, in: Capsule())
    }

    private var fg: Color {
        switch status {
        case .draft: return Theme.muted
        case .ready: return Theme.blue
        case .running: return Theme.amber
        case .complete: return Theme.green
        case .failed: return Theme.red
        }
    }

    private var bg: Color {
        switch status {
        case .draft: return Theme.surface
        case .ready: return Theme.blueSoft
        case .running: return Theme.amberSoft
        case .complete: return Theme.greenSoft
        case .failed: return Theme.redSoft
        }
    }
}

struct EmptyFrictionLogsView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        VStack(spacing: 14) {
            Spacer(minLength: 24)
            ZStack {
                Circle()
                    .fill(Theme.purpleSoft)
                    .frame(width: 56, height: 56)
                Image(systemName: "list.bullet.rectangle")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(Theme.purple)
            }
            VStack(spacing: 6) {
                Text(
                    appState.hiddenFrictionLogIDs.isEmpty
                        ? "No friction logs yet"
                        : "All friction logs are hidden"
                )
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Theme.ink)
                Text(
                    appState.hiddenFrictionLogIDs.isEmpty
                        ? "Author a scenario with the friction-log skill, then run it. Results land under .astroshot/friction-logs/."
                        : "Their files are still on disk. Restore them from Settings whenever you need them."
                )
                .font(.system(size: 11))
                .foregroundStyle(Theme.ink2)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 28)
            }
            if !appState.hiddenFrictionLogIDs.isEmpty {
                Button("Manage hidden logs") {
                    appState.openSettings()
                }
                .buttonStyle(.plain)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Theme.purple)
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(Theme.purpleSoft, in: RoundedRectangle(cornerRadius: 8))
                .accessibilityIdentifier("friction.hidden.manage")
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("How to author")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(Theme.muted)
                    .textCase(.uppercase)
                    .tracking(0.3)
                codeLine(".astroshot/friction-logs/<slug>/prompt.md")
                codeLine("runs/<run-id>/log.jsonl + screenshots")
                Text("In a coding agent: use the friction-log skill → Author, then Run.")
                    .font(.system(size: 10.5))
                    .foregroundStyle(Theme.ink2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 10))
            .padding(.horizontal, 28)
            .accessibilityIdentifier("friction.empty.cta")
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityIdentifier("friction.empty")
    }

    private func codeLine(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10, weight: .medium, design: .monospaced))
            .foregroundStyle(Theme.ink2)
    }
}
