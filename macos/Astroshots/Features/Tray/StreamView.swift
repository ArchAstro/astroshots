import SwiftUI

struct StreamView: View {
    @Environment(AppState.self) private var appState
    @State private var filter: StreamFilter = .toReview
    @State private var moviesOnly = false
    @State private var collapsedGroupIDs: Set<String> = []

    var body: some View {
        if appState.isEmpty {
            EmptyStreamView()
        } else {
            VStack(spacing: 0) {
                HStack(spacing: 8) {
                    Text(
                        filter == .toReview
                            ? "Unseen (\(pendingCount))"
                            : "History (\(seenCount))"
                    )
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.ink2)

                    Spacer()

                    if movieCount > 0 {
                        StreamFilterChip(
                            title: "Movies",
                            systemImage: "film",
                            isActive: moviesOnly
                        ) {
                            moviesOnly.toggle()
                        }
                        .accessibilityIdentifier("stream.filter.movies")
                    }

                    if filter == .toReview {
                        SeenAllButton(
                            count: pendingCount,
                            isWorking: appState.isMarkingSeen,
                            accessibilityIdentifier: "stream.seen.all"
                        ) {
                            Task {
                                await appState.markAllSeen(
                                    moviesOnly
                                        ? appState.shots.filter(\.isMovie)
                                        : appState.shots
                                )
                            }
                        }
                    }

                    StreamFilterChip(
                        title: filter == .history ? "Unseen" : "History",
                        systemImage: filter == .history
                            ? "eye"
                            : "clock.arrow.circlepath",
                        isActive: filter == .history
                    ) {
                        filter = filter == .history ? .toReview : .history
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 9)

                if displayedGroups.isEmpty {
                    if filter == .history {
                        EmptyHistoryView {
                            filter = .toReview
                        }
                    } else {
                        ReviewedStreamView {
                            filter = .history
                        }
                    }
                } else {
                    ScrollView {
                        LazyVStack(spacing: 8) {
                            ForEach(displayedGroups) { group in
                                WorktreeStreamGroup(
                                    group: group,
                                    isCollapsed: collapsedGroupIDs.contains(group.id),
                                    isMarkingSeen: appState.isMarkingSeen,
                                    showsSeenAction: filter == .toReview,
                                    toggleCollapsed: {
                                        if collapsedGroupIDs.contains(group.id) {
                                            collapsedGroupIDs.remove(group.id)
                                        } else {
                                            collapsedGroupIDs.insert(group.id)
                                        }
                                    },
                                    markAllSeen: {
                                        Task {
                                            await appState.markAllSeen(group.allShots)
                                        }
                                    },
                                    selectShot: appState.selectShot
                                )
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

    private var displayedGroups: [StreamShotGroup] {
        let base = moviesOnly ? appState.shots.filter(\.isMovie) : appState.shots
        return StreamGrouping.contiguousGroups(base).compactMap { group in
            let visibleShots: [Shot]
            switch filter {
            case .toReview:
                visibleShots = group.allShots.filter {
                    ($0.review?.state ?? .pending) != .seen
                }
            case .history:
                visibleShots = group.allShots.filter {
                    ($0.review?.state ?? .pending) == .seen
                }
            }
            guard !visibleShots.isEmpty else { return nil }
            return group.withVisibleShots(visibleShots)
        }
    }

    private var pendingCount: Int {
        let pool = moviesOnly ? appState.shots.filter(\.isMovie) : appState.shots
        return pool.filter { ($0.review?.state ?? .pending) != .seen }.count
    }

    private var seenCount: Int {
        let pool = moviesOnly ? appState.shots.filter(\.isMovie) : appState.shots
        return pool.filter { ($0.review?.state ?? .pending) == .seen }.count
    }

    private var movieCount: Int {
        appState.shots.filter(\.isMovie).count
    }
}

struct StreamShotGroup: Identifiable {
    let id: String
    let worktree: String
    let worktreePath: String
    let allShots: [Shot]
    let visibleShots: [Shot]

    func withVisibleShots(_ shots: [Shot]) -> StreamShotGroup {
        StreamShotGroup(
            id: id,
            worktree: worktree,
            worktreePath: worktreePath,
            allShots: allShots,
            visibleShots: shots
        )
    }

    var worktreeShort: String {
        allShots[0].worktreeShort
    }

    var unseenCount: Int {
        allShots.filter { ($0.review?.state ?? .pending) != .seen }.count
    }
}

enum StreamGrouping {
    static func contiguousGroups(_ shots: [Shot]) -> [StreamShotGroup] {
        var groups: [StreamShotGroup] = []
        var current: [Shot] = []

        func appendCurrent() {
            guard let first = current.first, let stableAnchor = current.last else {
                return
            }
            groups.append(
                StreamShotGroup(
                    id: stableAnchor.path,
                    worktree: first.worktree,
                    worktreePath: first.worktreePath,
                    allShots: current,
                    visibleShots: current
                )
            )
        }

        for shot in shots {
            if let first = current.first,
               first.worktreePath != shot.worktreePath
            {
                appendCurrent()
                current = []
            }
            current.append(shot)
        }
        appendCurrent()
        return groups
    }
}

private struct SeenAllButton: View {
    let count: Int
    let isWorking: Bool
    let accessibilityIdentifier: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Group {
                if isWorking {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Image(systemName: "checkmark.circle")
                        .font(.system(size: 12, weight: .semibold))
                }
            }
            .frame(width: 28, height: 28)
            .foregroundStyle(count > 0 ? Theme.purple : Theme.muted2)
            .background(
                count > 0 ? Theme.purpleSoft : Theme.surface,
                in: RoundedRectangle(cornerRadius: 6, style: .continuous)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(count == 0 || isWorking)
        .help(count == 1 ? "Mark 1 frame seen" : "Mark all \(count) unseen frames seen")
        .accessibilityLabel("Mark all unseen frames as seen")
        .accessibilityIdentifier(accessibilityIdentifier)
    }
}

private struct StreamFilterChip: View {
    let title: String
    let systemImage: String
    let isActive: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Image(systemName: systemImage)
                    .font(.system(size: 11, weight: .semibold))
                Text(title)
                    .font(.system(size: 10.5, weight: .semibold))
            }
            .padding(.horizontal, 8)
            .frame(height: 28)
            .foregroundStyle(isActive ? Theme.purple : Theme.ink2)
            .background(
                isActive ? Theme.purpleSoft : Theme.surface,
                in: RoundedRectangle(cornerRadius: 6, style: .continuous)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(isActive ? "Return to unseen frames" : "Show seen frame history")
        .accessibilityLabel(
            isActive ? "Return to unseen frames" : "Show seen frame history"
        )
        .accessibilityAddTraits(isActive ? .isSelected : [])
        .accessibilityIdentifier("stream.history")
    }
}

private struct WorktreeStreamGroup: View {
    let group: StreamShotGroup
    let isCollapsed: Bool
    let isMarkingSeen: Bool
    let showsSeenAction: Bool
    let toggleCollapsed: () -> Void
    let markAllSeen: () -> Void
    let selectShot: (Shot) -> Void

    var body: some View {
        VStack(spacing: 1) {
            HStack(spacing: 7) {
                Button(action: toggleCollapsed) {
                    HStack(spacing: 7) {
                        Image(systemName: "chevron.down")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(Theme.muted)
                            .rotationEffect(.degrees(isCollapsed ? -90 : 0))
                        WorktreeChip(label: group.worktreeShort)
                        Text(group.worktree)
                            .font(.system(size: 10.5, weight: .semibold))
                            .foregroundStyle(Theme.ink2)
                            .lineLimit(1)
                        Text("\(group.visibleShots.count)")
                            .font(.system(size: 9, weight: .semibold, design: .rounded))
                            .foregroundStyle(Theme.muted2)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(isCollapsed ? "Expand \(group.worktree)" : "Collapse \(group.worktree)")
                .accessibilityLabel(
                    isCollapsed
                        ? "Expand \(group.worktree)"
                        : "Collapse \(group.worktree)"
                )
                .accessibilityIdentifier("stream.group.toggle")

                Spacer(minLength: 4)

                if showsSeenAction, group.unseenCount > 0 {
                    SeenAllButton(
                        count: group.unseenCount,
                        isWorking: isMarkingSeen,
                        accessibilityIdentifier: "stream.group.seen",
                        action: markAllSeen
                    )
                }
            }
            .padding(.leading, 7)
            .padding(.trailing, 4)
            .padding(.vertical, 5)
            .background(
                Theme.surface.opacity(0.76),
                in: RoundedRectangle(cornerRadius: 9, style: .continuous)
            )
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("stream.group")

            if !isCollapsed {
                ForEach(group.visibleShots) { shot in
                    ShotRow(shot: shot, showsWorktree: false) {
                        selectShot(shot)
                    }
                }
            }
        }
    }
}

struct ShotRow: View {
    @Environment(AppState.self) private var appState
    @Environment(\.reviewChrome) private var reviewChrome

    let shot: Shot
    var showsWorktree = true
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
                        if shot.isMovie {
                            Image(systemName: "play.fill")
                                .font(.system(size: 11, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(7)
                                .background(.black.opacity(0.55), in: Circle())
                        }
                    }
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
            .help(shot.isMovie ? "Open full-screen review of movie poster" : "Open full-screen review")
            .accessibilityLabel(
                shot.isMovie
                    ? "Review movie \(shot.title) full screen"
                    : "Review \(shot.title) full screen"
            )
            .accessibilityIdentifier("stream.review.\(shot.fileName)")

            Button(action: action) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        if showsWorktree {
                            WorktreeChip(label: shot.worktreeShort)
                        }
                        if shot.isFailure {
                            Circle()
                                .fill(Theme.red)
                                .frame(width: 6, height: 6)
                        }
                        Text("\(shot.feature) · \(shot.title)")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundStyle(Theme.ink)
                            .lineLimit(1)
                            .help("\(shot.feature) · \(shot.title)")
                        Spacer(minLength: 4)
                        Text(Self.timeString(shot.capturedAt))
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.muted2)
                    }
                    Text(shot.description.isEmpty ? shot.fileName : shot.description)
                        .font(.system(size: 10))
                        .foregroundStyle(Theme.muted)
                        .lineLimit(1)
                        .help(shot.description.isEmpty ? shot.fileName : shot.description)

                    HStack(spacing: 5) {
                        if shot.isMovie {
                            MovieKindBadge(durationLabel: shot.durationLabel)
                        }
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
            .accessibilityIdentifier("stream.detail.\(shot.fileName)")
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .fill(Color.clear)
        )
        .contentShape(Rectangle())
        .contextMenu {
            Button("Copy Image") {
                _ = appState.copyShotImage(shot)
            }
            .accessibilityIdentifier("stream.copy.\(shot.fileName)")
        }
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
        case .pending: "Unseen"
        case .seen: "Seen"
        }
    }

    private var reviewColor: Color {
        switch reviewState {
        case .pending: Theme.amber
        case .seen: Theme.blue
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
    case history
}

struct MovieKindBadge: View {
    var durationLabel: String?

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: "film")
                .font(.system(size: 8, weight: .bold))
            Text(durationLabel.map { "Movie · \($0)" } ?? "Movie")
                .font(.system(size: 9, weight: .bold))
        }
        .foregroundStyle(Theme.purple)
        .padding(.horizontal, 5)
        .padding(.vertical, 2)
        .background(Theme.purpleSoft, in: Capsule())
        .accessibilityLabel(durationLabel.map { "Movie, \($0)" } ?? "Movie")
        .accessibilityIdentifier("stream.movie.badge")
    }
}

private struct ReviewedStreamView: View {
    let showAll: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Spacer()
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 24))
                .foregroundStyle(Theme.green)
            Text("You’re all caught up")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.ink)
            Text("Every current frame has been seen.")
                .font(.system(size: 11))
                .foregroundStyle(Theme.muted)
            Button("View history", action: showAll)
                .buttonStyle(.plain)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Theme.purple)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("stream.seen.empty")
    }
}

private struct EmptyHistoryView: View {
    let showUnseen: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Spacer()
            Image(systemName: "clock.arrow.circlepath")
                .font(.system(size: 24))
                .foregroundStyle(Theme.muted2)
            Text("No history yet")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.ink)
            Text("Frames you mark Seen will appear here.")
                .font(.system(size: 11))
                .foregroundStyle(Theme.muted)
            Button("Back to unseen", action: showUnseen)
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
            Image(systemName: appState.needsWatchRootSetup ? "folder.badge.plus" : "camera")
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(Theme.muted)
                .frame(width: 40, height: 40)
                .background(Theme.surface, in: RoundedRectangle(cornerRadius: 12))
                .padding(.bottom, 12)
            Text(title)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.ink)
                .padding(.bottom, 5)
            Text(subtitle)
                .font(.system(size: 11))
                .foregroundStyle(Theme.muted)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 240)
                .padding(.bottom, 14)
            if appState.needsWatchRootSetup {
                Button("Choose folders…") {
                    _ = appState.chooseWatchRoots(forSetup: true)
                }
                .buttonStyle(.plain)
                .font(.system(size: 11, weight: .semibold))
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
                .background(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(Theme.lineStrong, lineWidth: 1)
                        .background(Theme.elevated, in: RoundedRectangle(cornerRadius: 8))
                )
                .accessibilityIdentifier("setup-choose-folders")
            } else {
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
                        .background(Theme.elevated, in: RoundedRectangle(cornerRadius: 8))
                )
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(.horizontal, 28)
    }

    private var title: String {
        if appState.needsWatchRootSetup {
            return "Choose folders to watch"
        }
        if appState.isScanning {
            return "Scanning watched folders…"
        }
        return "Waiting for frames"
    }

    private var subtitle: String {
        if appState.needsWatchRootSetup {
            return "Pick the directories that contain your projects. Astroshots watches them for .astroshot/ screenshots."
        }
        if appState.isScanning {
            return "First scan of a large folder can take a bit. The menu bar icon stays available."
        }
        return "When any project under a watched folder writes to .astroshot/, shots land here and on the desktop."
    }
}
