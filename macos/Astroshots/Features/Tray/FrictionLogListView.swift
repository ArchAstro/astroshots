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
                        FrictionLogRow(log: log) {
                            appState.selectFrictionLog(log)
                        }
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

    var body: some View {
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

                HStack(spacing: 6) {
                    WorktreeChip(label: log.worktreeShort)
                    Text(log.slug)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundStyle(Theme.muted2)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    metric(icon: "list.number", value: "\(log.stepCount)")
                    metric(icon: "hand.thumbsup", value: "\(log.goodCount)", tint: Theme.green)
                    metric(icon: "exclamationmark.bubble", value: "\(log.improveCount)", tint: Theme.amber)
                }

                if let run = log.latestRun {
                    HStack(spacing: 6) {
                        Image(systemName: "play.circle.fill")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(Theme.purple)
                        Text(run.runID)
                            .font(.system(size: 10, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.ink2)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Text(relativeTime(run.capturedAt))
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(Theme.muted)
                    }
                } else {
                    Text("Prompt only · no runs yet")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(Theme.muted)
                }
            }
            .padding(11)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Color.white.opacity(0.72))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(Theme.line, lineWidth: 1)
            )
            .contentShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("friction.row.\(log.slug)")
    }

    private func metric(icon: String, value: String, tint: Color = Theme.muted2) -> some View {
        HStack(spacing: 3) {
            Image(systemName: icon)
                .font(.system(size: 9, weight: .semibold))
            Text(value)
                .font(.system(size: 10, weight: .semibold, design: .monospaced))
        }
        .foregroundStyle(tint)
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
                Text("No friction logs yet")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Theme.ink)
                Text(
                    "Author a scenario with the friction-log skill, then run it. Results land under .astroshot/friction-logs/."
                )
                .font(.system(size: 11))
                .foregroundStyle(Theme.ink2)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 28)
            }
            VStack(alignment: .leading, spacing: 4) {
                codeLine(".astroshot/friction-logs/<slug>/prompt.md")
                codeLine("runs/<run-id>/log.jsonl + screenshots")
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 10))
            .padding(.horizontal, 28)
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
