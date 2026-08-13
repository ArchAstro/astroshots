import SwiftUI

/// Catalog of authored friction-log scenarios across watched worktrees.
struct FrictionLogListView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        if appState.isFrictionLogsEmpty {
            EmptyFrictionLogsView()
        } else {
            VStack(spacing: 0) {
                HStack(spacing: 8) {
                    Text(
                        appState.frictionLogFilter == .toReview
                            ? "Unseen (\(pendingCount))"
                            : "History (\(seenCount))"
                    )
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.ink2)

                    Spacer()

                    if appState.frictionLogFilter == .toReview {
                        SeenAllButton(
                            count: pendingCount,
                            isWorking: appState.isMarkingSeen,
                            accessibilityIdentifier: "friction.seen.all"
                        ) {
                            Task {
                                await appState.markAllFrictionLogsSeen(appState.frictionLogs)
                            }
                        }
                    }

                    StreamFilterChip(
                        title: appState.frictionLogFilter == .history ? "Unseen" : "History",
                        systemImage: appState.frictionLogFilter == .history
                            ? "eye"
                            : "clock.arrow.circlepath",
                        isActive: appState.frictionLogFilter == .history,
                        unseenHelp: "Show seen friction-log history",
                        seenHelp: "Return to unseen friction logs",
                        accessibilityIdentifier: "friction.history"
                    ) {
                        appState.frictionLogFilter =
                            appState.frictionLogFilter == .history ? .toReview : .history
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 9)

                if displayedLogs.isEmpty {
                    if appState.frictionLogFilter == .history {
                        EmptyHistoryView(
                            detail: "Logs you mark Seen will appear here."
                        ) {
                            appState.frictionLogFilter = .toReview
                        }
                    } else {
                        ReviewedStreamView(
                            detail: "Every current friction log has been seen.",
                            accessibilityIdentifier: "friction.seen.empty"
                        ) {
                            appState.frictionLogFilter = .history
                        }
                    }
                } else {
                    ScrollView {
                        LazyVStack(spacing: 8) {
                            ForEach(displayedLogs) { log in
                                FrictionLogRow(
                                    log: log,
                                    showsSeenAction: appState.frictionLogFilter == .toReview,
                                    action: { appState.selectFrictionLog(log) },
                                    seenAction: {
                                        Task {
                                            try? await appState.markFrictionLogSeen(log)
                                        }
                                    }
                                )
                            }
                        }
                        .padding(.horizontal, 10)
                        .padding(.bottom, 8)
                    }
                    .scrollIndicators(.hidden)
                }
            }
        }
    }

    private var displayedLogs: [FrictionLog] {
        switch appState.frictionLogFilter {
        case .toReview:
            return appState.frictionLogs.filter { $0.reviewState != .seen }
        case .history:
            return appState.frictionLogs.filter { $0.reviewState == .seen }
        }
    }

    private var pendingCount: Int {
        appState.frictionLogs.filter { $0.reviewState != .seen }.count
    }

    private var seenCount: Int {
        appState.frictionLogs.filter { $0.reviewState == .seen }.count
    }
}

private struct FrictionLogRow: View {
    let log: FrictionLog
    var showsSeenAction = true
    let action: () -> Void
    let seenAction: () -> Void

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

            if showsSeenAction, log.reviewState != .seen, log.latestRun != nil {
                Button(action: seenAction) {
                    Image(systemName: "checkmark.circle")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.purple)
                        .frame(width: 30, height: 30)
                        .background(Theme.purpleSoft, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Mark \(log.title) seen")
                .accessibilityLabel("Mark \(log.title) seen")
                .accessibilityIdentifier("friction.seen.\(log.slug)")
                .padding(.trailing, 6)
            }
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
