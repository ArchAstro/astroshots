import SwiftUI

struct StreamView: View {
    @Environment(AppState.self) private var appState
    @State private var filter: StreamFilter = .toReview

    var body: some View {
        if appState.isEmpty {
            EmptyStreamView()
        } else {
            VStack(spacing: 0) {
                Picker("Review filter", selection: $filter) {
                    Text("To review (\(pendingCount))").tag(StreamFilter.toReview)
                    Text("All (\(appState.shots.count))").tag(StreamFilter.all)
                }
                .pickerStyle(.segmented)
                .controlSize(.small)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)

                if filteredShots.isEmpty {
                    ReviewedStreamView {
                        filter = .all
                    }
                } else {
                    ScrollView {
                        LazyVStack(spacing: 1) {
                            ForEach(filteredShots) { shot in
                                ShotRow(shot: shot) {
                                    appState.selectShot(shot)
                                }
                            }
                        }
                        .padding(.horizontal, 10)
                        .padding(.bottom, 6)
                    }
                    .scrollIndicators(.hidden)
                }
            }
        }
    }

    private var filteredShots: [Shot] {
        switch filter {
        case .toReview:
            appState.shots.filter { ($0.review?.state ?? .pending) != .approved }
        case .all:
            appState.shots
        }
    }

    private var pendingCount: Int {
        appState.shots.filter { ($0.review?.state ?? .pending) != .approved }.count
    }
}

struct ShotRow: View {
    @Environment(\.reviewChrome) private var reviewChrome

    let shot: Shot
    let action: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Button {
                reviewChrome.open(shot)
            } label: {
                ShotThumbnail(path: shot.path)
                    .frame(width: 88, height: 56)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(Theme.line, lineWidth: 1)
                    )
                    .overlay {
                        Image(systemName: "arrow.up.left.and.arrow.down.right")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(.white)
                            .padding(5)
                            .background(.black.opacity(0.56), in: Circle())
                            .padding(5)
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
                    }
            }
            .buttonStyle(.plain)
            .help("Open full-screen review")
            .accessibilityLabel("Review \(shot.title) full screen")
            .accessibilityIdentifier("stream.review.\(shot.fileName)")

            Button(action: action) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        WorktreeChip(label: shot.worktreeShort)
                        if shot.isFailure {
                            Circle()
                                .fill(Theme.red)
                                .frame(width: 6, height: 6)
                        }
                        Text("\(shot.feature) · \(shot.title)")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundStyle(Theme.ink)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Text(Self.timeString(shot.capturedAt))
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.muted2)
                    }
                    Text(shot.description.isEmpty ? shot.fileName : shot.description)
                        .font(.system(size: 10))
                        .foregroundStyle(Theme.muted)
                        .lineLimit(1)

                    HStack(spacing: 5) {
                        reviewDot
                        Text(reviewLabel)
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundStyle(reviewColor)
                        if let status = shot.status {
                            Text("· run \(status.rawValue)")
                                .font(.system(size: 9))
                                .foregroundStyle(Theme.muted2)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .fill(Color.clear)
        )
        #if os(macOS)
        .onHover { hovering in
            // Visual handled by plain button; could add highlight state later.
            _ = hovering
        }
        #endif
    }

    private var reviewState: ReviewState {
        shot.review?.state ?? .pending
    }

    private var reviewLabel: String {
        switch reviewState {
        case .pending: "To review"
        case .approved: "Approved"
        case .changesRequested: "Changes requested"
        }
    }

    private var reviewColor: Color {
        switch reviewState {
        case .pending: Theme.amber
        case .approved: Theme.green
        case .changesRequested: Theme.red
        }
    }

    private var reviewDot: some View {
        Circle()
            .fill(reviewColor)
            .frame(width: 6, height: 6)
    }

    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        return f
    }()

    static func timeString(_ date: Date) -> String {
        timeFormatter.string(from: date)
    }
}

private enum StreamFilter: Hashable {
    case toReview
    case all
}

private struct ReviewedStreamView: View {
    let showAll: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Spacer()
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 24))
                .foregroundStyle(Theme.green)
            Text("Review inbox is clear")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.ink)
            Text("Every current frame is approved.")
                .font(.system(size: 11))
                .foregroundStyle(Theme.muted)
            Button("Show all frames", action: showAll)
                .buttonStyle(.plain)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Theme.purple)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct EmptyStreamView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            Image(systemName: "camera")
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(Theme.muted)
                .frame(width: 40, height: 40)
                .background(Theme.surface, in: RoundedRectangle(cornerRadius: 12))
                .padding(.bottom, 12)
            Text(appState.isScanning ? "Scanning watch root…" : "Waiting for frames")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.ink)
                .padding(.bottom, 5)
            Text(
                appState.isScanning
                    ? "First scan of a large folder can take a bit. The menu bar icon stays available."
                    : "When any project under the watch root writes to .astroshot/, shots land here and on the desktop."
            )
                .font(.system(size: 11))
                .foregroundStyle(Theme.muted)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 240)
                .padding(.bottom, 14)
            Button("Rescan") {
                appState.rescan()
            }
            .buttonStyle(.plain)
            .font(.system(size: 11, weight: .semibold))
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Theme.lineStrong, lineWidth: 1)
                    .background(Color.white, in: RoundedRectangle(cornerRadius: 8))
            )
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(.horizontal, 28)
    }
}
