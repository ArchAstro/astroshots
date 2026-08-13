import AppKit
import SwiftUI

struct DetailView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.reviewChrome) private var reviewChrome
    @State private var reviewNote = ""
    @State private var submissionError: String?
    @State private var isSubmitting = false
    @State private var activeSubmissionID: UUID?
    @State private var showInlinePlayer = false
    @FocusState private var feedbackFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Button {
                    appState.backToStream()
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 10, weight: .semibold))
                        Text("Stream")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    .foregroundStyle(Theme.ink2)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .background(Color.clear, in: RoundedRectangle(cornerRadius: 7))

                Spacer()

                if let position = selectedDetailPosition {
                    Text("\(position.index) / \(position.count)")
                        .font(.system(size: 10, weight: .semibold, design: .monospaced))
                        .foregroundStyle(Theme.muted2)
                        .accessibilityIdentifier("detail.position")
                        .accessibilityLabel(
                            "Screenshot \(position.index) of \(position.count)"
                        )
                }

                Button {
                    appState.stepDetail(1)
                } label: {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.ink2)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .opacity(canStepOlder ? 1 : 0.32)
                .disabled(!canStepOlder)
                .help("Older screenshot (←)")
                .accessibilityLabel("Older screenshot")
                .accessibilityIdentifier("detail.navigate.older")
                .keyboardShortcut(.leftArrow, modifiers: [])

                Button {
                    appState.stepDetail(-1)
                } label: {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.ink2)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .opacity(canStepNewer ? 1 : 0.32)
                .disabled(!canStepNewer)
                .help("Newer screenshot (→)")
                .accessibilityLabel("Newer screenshot")
                .accessibilityIdentifier("detail.navigate.newer")
                .keyboardShortcut(.rightArrow, modifiers: [])
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Theme.line).frame(height: 1)
            }

            if let shot = appState.selectedShot {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        Button {
                            reviewChrome.open(shot)
                        } label: {
                            FittedDetailPreview(path: shot.path, maxHeight: 240)
                                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                                        .stroke(Theme.line, lineWidth: 1)
                                )
                                .overlay(alignment: .bottomTrailing) {
                                    Image(systemName: "arrow.up.left.and.arrow.down.right")
                                        .font(.system(size: 10, weight: .bold))
                                        .foregroundStyle(.white)
                                        .frame(width: 28, height: 28)
                                        .background(.black.opacity(0.64), in: Circle())
                                        .padding(9)
                                }
                        }
                        .buttonStyle(.plain)
                        .help("Open full-screen review")
                        .accessibilityLabel("Review \(shot.title) full screen")
                        .accessibilityIdentifier("detail.preview")
                        .contextMenu {
                            Button("Copy Image") {
                                _ = appState.copyShotImage(shot)
                            }
                            .accessibilityIdentifier("detail.copy")
                        }
                        .padding(.horizontal, 12)
                        .padding(.top, 12)
                        .frame(maxWidth: .infinity)

                        VStack(alignment: .leading, spacing: 4) {
                            HStack(alignment: .top, spacing: 8) {
                                Text(heading(for: shot))
                                    .font(.system(size: 15, weight: .semibold))
                                    .foregroundStyle(Theme.ink)
                                    .tracking(-0.3)
                                    .lineLimit(2)
                                    .fixedSize(horizontal: false, vertical: true)
                                    .layoutPriority(1)
                                Spacer(minLength: 4)
                                if shot.isMovie {
                                    MovieKindBadge(durationLabel: shot.durationLabel)
                                        .fixedSize()
                                }
                                ReviewBadge(state: shot.review?.state ?? .pending)
                                    .fixedSize()
                            }
                            Text(shot.description.isEmpty ? shot.fileName : shot.description)
                                .font(.system(size: 11))
                                .foregroundStyle(Theme.ink2)
                                .fixedSize(horizontal: false, vertical: true)
                                .padding(.bottom, shot.isMovie ? 8 : 12)

                            if shot.isMovie {
                                movieActions(for: shot)
                                    .padding(.bottom, 8)
                                if showInlinePlayer, let videoPath = shot.videoPath {
                                    MoviePlayerView(path: videoPath)
                                        .frame(height: 180)
                                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                                        .padding(.bottom, 12)
                                        .accessibilityIdentifier("detail.movie.player")
                                }
                                if !shot.chapters.isEmpty {
                                    chaptersCard(shot.chapters)
                                        .padding(.bottom, 12)
                                }
                            }

                            compactReviewControls(for: shot)
                                .padding(.bottom, 12)

                            VStack(alignment: .leading, spacing: 6) {
                                row("Tree", shot.worktree)
                                row("Feature", shot.feature)
                                row("File", shot.fileName)
                                if shot.isMovie {
                                    row("Kind", "Movie")
                                    if let video = shot.videoFileName, !video.isEmpty {
                                        row("Video", video)
                                    }
                                    if let duration = shot.durationLabel {
                                        row("Duration", duration)
                                    }
                                    if let source = shot.movieSource, !source.isEmpty {
                                        row("Source", source)
                                    }
                                    if !shot.chapters.isEmpty {
                                        row("Chapters", "\(shot.chapters.count)")
                                    }
                                }
                                row("Time", iso(shot.capturedAt))
                                if let url = shot.url, !url.isEmpty {
                                    row("URL", url)
                                }
                                if let run = shot.runID, !run.isEmpty {
                                    row("Run", run)
                                }
                                if let status = shot.status {
                                    row("Execution", status.rawValue.capitalized)
                                }
                                row("Review", reviewLabel(for: shot))
                                row("Path", shot.path)
                            }
                            .padding(11)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(
                                RoundedRectangle(cornerRadius: 11)
                                    .fill(Theme.elevated.opacity(0.7))
                                    .overlay(
                                        RoundedRectangle(cornerRadius: 11)
                                            .stroke(Theme.line, lineWidth: 1)
                                    )
                            )
                            .accessibilityElement(children: .contain)
                            .accessibilityIdentifier("detail.metadata")
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 14)
                        .padding(.top, 12)
                        .padding(.bottom, 16)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("detail.compact")
                }
                .scrollIndicators(.hidden)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("detail.viewport")
                .onChange(of: shot.id) {
                    reviewNote = ""
                    submissionError = nil
                    isSubmitting = false
                    activeSubmissionID = nil
                    feedbackFocused = false
                    showInlinePlayer = false
                }
            } else {
                EmptyStreamView()
            }
        }
    }

    private var canStepOlder: Bool {
        appState.canStepDetail(1)
    }

    private var canStepNewer: Bool {
        appState.canStepDetail(-1)
    }

    private var selectedDetailPosition: (index: Int, count: Int)? {
        guard let id = appState.selectedShotID ?? appState.selectedShot?.id else {
            return nil
        }
        return appState.detailPosition(for: id)
    }

    private func reviewLabel(for shot: Shot) -> String {
        switch shot.review?.state ?? .pending {
        case .pending: "Unseen"
        case .seen: "Seen"
        }
    }

    private func heading(for shot: Shot) -> String {
        if let sequence = shot.sequence {
            return "\(sequence) · \(shot.slug)"
        }
        return shot.slug
    }

    @ViewBuilder
    private func movieActions(for shot: Shot) -> some View {
        let videoURL = shot.videoPath.map { URL(fileURLWithPath: $0) }
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Button {
                    showInlinePlayer.toggle()
                } label: {
                    Label(
                        showInlinePlayer ? "Hide player" : "Play in tray",
                        systemImage: showInlinePlayer ? "rectangle.compress.vertical" : "play.rectangle.fill"
                    )
                }
                .buttonStyle(ReviewActionButtonStyle(tone: .primary))
                .disabled(videoURL == nil)
                .accessibilityIdentifier("detail.movie.play")

                Button {
                    if let videoURL {
                        NSWorkspace.shared.open(videoURL)
                    }
                } label: {
                    Label("Open movie", systemImage: "arrow.up.forward.app")
                }
                .buttonStyle(ReviewActionButtonStyle(tone: .quiet))
                .disabled(videoURL == nil)
                .help(
                    videoURL == nil
                        ? "Video file is missing next to this poster"
                        : "Open \(shot.videoFileName ?? "movie") in the default player"
                )
                .accessibilityIdentifier("detail.movie.open")
            }

            if videoURL == nil, shot.videoFileName != nil {
                Text("Video missing on disk")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(Theme.amber)
            } else if videoURL == nil {
                Text("Poster only — no video path in manifest")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(Theme.muted2)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("detail.movie.actions")
    }

    private func chaptersCard(_ chapters: [ManifestChapter]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Chapters")
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(Theme.muted)
                .textCase(.uppercase)
                .tracking(0.4)
            ForEach(Array(chapters.enumerated()), id: \.offset) { _, chapter in
                HStack(spacing: 8) {
                    Text(Self.chapterTimeLabel(chapter.tMs))
                        .font(.system(size: 10, weight: .semibold, design: .monospaced))
                        .foregroundStyle(Theme.purple)
                        .frame(width: 44, alignment: .leading)
                    Text(chapter.title ?? chapter.slug ?? "chapter")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(Theme.ink)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.purpleSoft.opacity(0.55), in: RoundedRectangle(cornerRadius: 10))
        .accessibilityIdentifier("detail.movie.chapters")
    }

    private static func chapterTimeLabel(_ tMs: Int?) -> String {
        guard let tMs, tMs >= 0 else { return "—" }
        let total = Double(tMs) / 1000
        if total < 60 {
            return String(format: "%.1fs", total)
        }
        let m = Int(total) / 60
        let s = Int(total) % 60
        return String(format: "%d:%02d", m, s)
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(label)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Theme.muted2)
                .frame(width: 56, alignment: .leading)
            Text(value)
                .font(.system(size: 9.5, design: .monospaced))
                .foregroundStyle(Theme.ink)
                .textSelection(.enabled)
                .lineLimit(nil)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func compactReviewControls(for shot: Shot) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField(
                "Share feedback…",
                text: $reviewNote,
                axis: .vertical
            )
            .textFieldStyle(.plain)
            .font(.system(size: 11))
            .foregroundStyle(Theme.ink)
            .lineLimit(1...3)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Theme.elevated.opacity(0.78), in: RoundedRectangle(cornerRadius: 9))
            .overlay(
                RoundedRectangle(cornerRadius: 9)
                    .stroke(Theme.lineStrong, lineWidth: 1)
            )
            .focused($feedbackFocused)
            .accessibilityLabel("Compact review feedback")
            .accessibilityHint("Share optional feedback about this screenshot.")
            .accessibilityIdentifier("detail.feedback.note")

            if let submissionError {
                Label(submissionError, systemImage: "exclamationmark.circle.fill")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(Theme.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("detail.review.error")
            }

            HStack(spacing: 8) {
                Button {
                    submitFeedback(for: shot)
                } label: {
                    Label("Send Feedback", systemImage: "arrow.up.circle")
                }
                .buttonStyle(ReviewActionButtonStyle(tone: .quiet))
                .disabled(isSubmitting || trimmedReviewNote.isEmpty)
                .accessibilityIdentifier("detail.feedback.send")

                Button {
                    markSeen(for: shot)
                } label: {
                    Label("Seen", systemImage: "checkmark.circle.fill")
                }
                .buttonStyle(ReviewActionButtonStyle(tone: .primary))
                .disabled(isSubmitting)
                .accessibilityHint("Marks this screenshot as seen and returns to the stream.")
                .accessibilityIdentifier("detail.seen")
            }

            if isSubmitting {
                HStack(spacing: 7) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Saving review…")
                        .font(.system(size: 10))
                        .foregroundStyle(Theme.muted2)
                }
                .frame(maxWidth: .infinity, alignment: .center)
                .accessibilityIdentifier("detail.review.saving")
            }
        }
        .padding(10)
        .background(Theme.surface.opacity(0.64), in: RoundedRectangle(cornerRadius: 11))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("detail.review.controls")
    }

    private var trimmedReviewNote: String {
        reviewNote.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func submitFeedback(for shot: Shot) {
        let note = trimmedReviewNote
        guard !note.isEmpty else { return }
        performSubmission(for: shot) {
            try await appState.addReviewComment(note, to: shot)
            reviewNote = ""
        }
    }

    private func markSeen(for shot: Shot) {
        let note = trimmedReviewNote
        performSubmission(for: shot) {
            try await appState.markSeen(
                note: note.isEmpty ? nil : note,
                for: shot
            )
            reviewNote = ""
            appState.backToStream()
        }
    }

    private func performSubmission(
        for shot: Shot,
        operation: @escaping @MainActor () async throws -> Void
    ) {
        guard !isSubmitting else { return }
        let submissionID = UUID()
        activeSubmissionID = submissionID
        isSubmitting = true
        submissionError = nil

        Task { @MainActor in
            defer {
                if activeSubmissionID == submissionID {
                    isSubmitting = false
                    activeSubmissionID = nil
                }
            }
            do {
                try await operation()
            } catch {
                if activeSubmissionID == submissionID,
                   appState.selectedShotID == shot.id
                {
                    submissionError = error.localizedDescription
                }
            }
        }
    }

    private static let isoFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    private func iso(_ date: Date) -> String {
        Self.isoFormatter.string(from: date)
    }
}

private struct FittedDetailPreview: View {
    let path: String
    let maxHeight: CGFloat

    var body: some View {
        AspectFitPreviewLayout(
            aspectRatio: imageAspectRatio,
            maxHeight: maxHeight
        ) {
            ShotThumbnail(path: path, contentMode: .fit)
        }
    }

    private var imageAspectRatio: CGFloat {
        guard let image = NSImage(contentsOfFile: path) else {
            return 16 / 10
        }
        if let representation = image.representations.first(where: {
            $0.pixelsWide > 0 && $0.pixelsHigh > 0
        }) {
            return CGFloat(representation.pixelsWide) / CGFloat(representation.pixelsHigh)
        }
        guard image.size.width > 0, image.size.height > 0 else {
            return 16 / 10
        }
        return image.size.width / image.size.height
    }
}

private struct AspectFitPreviewLayout: Layout {
    let aspectRatio: CGFloat
    let maxHeight: CGFloat

    func sizeThatFits(
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) -> CGSize {
        let safeAspectRatio = max(aspectRatio, 0.01)
        let proposedWidth = proposal.width.flatMap { $0.isFinite ? $0 : nil }
        let availableWidth = max(0, proposedWidth ?? maxHeight * safeAspectRatio)
        let width = min(availableWidth, maxHeight * safeAspectRatio)
        let height = width / safeAspectRatio
        return CGSize(width: width, height: height)
    }

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        guard let preview = subviews.first else { return }
        preview.place(
            at: bounds.origin,
            anchor: .topLeading,
            proposal: ProposedViewSize(width: bounds.width, height: bounds.height)
        )
    }
}
