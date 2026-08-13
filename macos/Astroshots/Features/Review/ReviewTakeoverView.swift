import AppKit
import SwiftUI

/// Chromeless full-screen review surface.
///
/// Integration assumptions are intentionally narrow:
/// - `Shot.review` is an optional `ReviewSnapshot`.
/// - `ReviewSnapshot` exposes `state`, `comments`, and `isStale`.
/// - `AppState` performs the async review sidecar writes through
///   `addReviewComment(_:to:)` and `markSeen(note:for:)`.
@MainActor
struct ReviewTakeoverView: View {
    let shot: Shot
    let appState: AppState
    let onClose: () -> Void
    let onNavigate: (Int) -> Void

    @State private var feedback = ""
    @State private var isSubmitting = false
    @State private var submissionError: String?
    @FocusState private var composerFocused: Bool

    init(
        shot: Shot,
        appState: AppState,
        onClose: @escaping () -> Void,
        onNavigate: @escaping (Int) -> Void
    ) {
        self.shot = shot
        self.appState = appState
        self.onClose = onClose
        self.onNavigate = onNavigate
    }

    var body: some View {
        ZStack {
            Color(hex: 0x242423)
                .ignoresSafeArea()

            VStack(spacing: 0) {
                takeoverHeader

                HStack(spacing: 0) {
                    imageStage
                    feedbackRail
                        .frame(width: 372)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("review.takeover")
        .onExitCommand(perform: onClose)
    }

    private var takeoverHeader: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(currentShot.title)
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(Color.white.opacity(0.96))
                        .lineLimit(1)

                    if let sequence = currentShot.sequence {
                        Text(sequence)
                            .font(.system(size: 10, weight: .bold, design: .monospaced))
                            .foregroundStyle(Color.white.opacity(0.62))
                            .padding(.horizontal, 7)
                            .padding(.vertical, 3)
                            .background(Color.white.opacity(0.1), in: Capsule())
                    }
                }

                Text(metadataLine)
                    .font(.system(size: 10.5))
                    .foregroundStyle(Color.white.opacity(0.52))
                    .lineLimit(1)
            }

            Spacer(minLength: 24)

            if let position = reviewPosition {
                Text("\(position.index) / \(position.count)")
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Color.white.opacity(0.58))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(Color.white.opacity(0.08), in: Capsule())
                    .accessibilityIdentifier("review.position")
                    .accessibilityLabel(
                        "Screenshot \(position.index) of \(position.count)"
                    )
            }

            navigationButton(
                symbol: "chevron.left",
                help: "Older screenshot",
                identifier: "review.navigate.older",
                isDisabled: !canNavigate(-1)
            ) {
                onNavigate(-1)
            }

            navigationButton(
                symbol: "chevron.right",
                help: "Newer screenshot",
                identifier: "review.navigate.newer",
                isDisabled: !canNavigate(1)
            ) {
                onNavigate(1)
            }

            Rectangle()
                .fill(Color.white.opacity(0.14))
                .frame(width: 1, height: 22)
                .padding(.horizontal, 3)

            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(Color.white.opacity(0.78))
                    .frame(width: 32, height: 32)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 9))
            .keyboardShortcut(.cancelAction)
            .help("Close review")
            .accessibilityLabel("Close review")
            .accessibilityIdentifier("review.close")
        }
        .padding(.horizontal, 18)
        .frame(height: 64)
        .background(Color.black.opacity(0.18))
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(Color.white.opacity(0.1))
                .frame(height: 1)
        }
    }

    private var imageStage: some View {
        GeometryReader { proxy in
            ZStack {
                Color(hex: 0x2A2A29)

                reviewMedia
                    .frame(
                        maxWidth: max(240, proxy.size.width - 72),
                        maxHeight: max(180, proxy.size.height - 72)
                    )
                    .padding(36)

                HStack {
                    stageNavigateButton(
                        symbol: "chevron.left",
                        help: "Older screenshot",
                        identifier: "review.stage.older",
                        isDisabled: !canNavigate(-1)
                    ) {
                        onNavigate(-1)
                    }

                    Spacer()

                    stageNavigateButton(
                        symbol: "chevron.right",
                        help: "Newer screenshot",
                        identifier: "review.stage.newer",
                        isDisabled: !canNavigate(1)
                    ) {
                        onNavigate(1)
                    }
                }
                .padding(.horizontal, 18)
            }
        }
    }

    private func stageNavigateButton(
        symbol: String,
        help: String,
        identifier: String,
        isDisabled: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Color.white.opacity(0.92))
                .frame(width: 44, height: 44)
                .background(Color.black.opacity(0.42), in: Circle())
                .overlay(
                    Circle()
                        .stroke(Color.white.opacity(0.14), lineWidth: 1)
                )
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .opacity(isDisabled ? 0.22 : 1)
        .disabled(isDisabled)
        .help("\(help) (←/→)")
        .accessibilityLabel(help)
        .accessibilityIdentifier(identifier)
    }

    @ViewBuilder
    private var reviewMedia: some View {
        if currentShot.isMovie, let videoPath = currentShot.videoPath {
            MoviePlayerView(path: videoPath)
                .aspectRatio(movieAspectRatio, contentMode: .fit)
                .background(Color.black)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(Color.white.opacity(0.14), lineWidth: 1)
                )
                .shadow(color: .black.opacity(0.34), radius: 28, y: 12)
                .accessibilityLabel("Movie player for \(currentShot.title)")
                .accessibilityIdentifier("review.movie.player")
        } else if let image = NSImage(contentsOfFile: currentShot.path) {
            Image(nsImage: image)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(Color.white.opacity(0.14), lineWidth: 1)
                )
                .shadow(color: .black.opacity(0.34), radius: 28, y: 12)
                .contentShape(Rectangle())
                .contextMenu {
                    Button("Copy Image") {
                        _ = appState.copyShotImage(currentShot)
                    }
                    .accessibilityIdentifier("review.copy")
                }
                .accessibilityLabel("Screenshot \(currentShot.title)")
                .accessibilityIdentifier("review.image")
        } else {
            VStack(spacing: 10) {
                Image(systemName: "photo.badge.exclamationmark")
                    .font(.system(size: 32, weight: .light))
                Text("Screenshot unavailable")
                    .font(.system(size: 13, weight: .medium))
            }
            .foregroundStyle(Color.white.opacity(0.52))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityIdentifier("review.image")
        }
    }

    private var movieAspectRatio: CGFloat {
        guard let image = NSImage(contentsOfFile: currentShot.path),
              image.size.width > 0, image.size.height > 0
        else { return 16 / 9 }
        return image.size.width / image.size.height
    }

    private var feedbackRail: some View {
        VStack(alignment: .leading, spacing: 0) {
            railSummary

            Rectangle()
                .fill(Theme.line)
                .frame(height: 1)

            commentHistory

            Rectangle()
                .fill(Theme.line)
                .frame(height: 1)

            composer
        }
        .background(Theme.paper)
        .overlay(alignment: .leading) {
            Rectangle()
                .fill(Color.black.opacity(0.2))
                .frame(width: 1)
        }
    }

    private var railSummary: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text("Review")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(Theme.ink)

                Spacer()

                ReviewBadge(state: reviewState)
            }

            if currentShot.review?.isStale == true {
                Label("A newer image was captured since this was seen.", systemImage: "clock.arrow.circlepath")
                    .font(.system(size: 10.5, weight: .medium))
                    .foregroundStyle(Theme.amber)
                    .padding(.horizontal, 9)
                    .padding(.vertical, 7)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Theme.amberSoft, in: RoundedRectangle(cornerRadius: 8))
                    .accessibilityIdentifier("review.stale")
            }

            if !currentShot.description.isEmpty {
                Text(currentShot.description)
                    .font(.system(size: 11.5))
                    .foregroundStyle(Theme.ink2)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(16)
    }

    private var commentHistory: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Feedback")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(Theme.muted2)
                    .textCase(.uppercase)
                    .tracking(0.5)

                Spacer()

                Text("\(comments.count)")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(Theme.muted2)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Theme.surface, in: Capsule())
            }

            if comments.isEmpty {
                VStack(spacing: 9) {
                    Image(systemName: "text.bubble")
                        .font(.system(size: 22, weight: .light))
                        .foregroundStyle(Theme.muted)
                    Text("No comments yet")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.ink2)
                    Text("Leave concise, actionable feedback for this frame.")
                        .font(.system(size: 10.5))
                        .foregroundStyle(Theme.muted2)
                        .multilineTextAlignment(.center)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(.horizontal, 30)
                .accessibilityIdentifier("review.comments.empty")
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 9) {
                            ForEach(comments) { comment in
                                ReviewCommentView(comment: comment)
                                    .id(comment.id)
                            }
                        }
                    }
                    .scrollIndicators(.hidden)
                    .onChange(of: comments.count) {
                        guard let last = comments.last else { return }
                        withAnimation(.easeOut(duration: 0.2)) {
                            proxy.scrollTo(last.id, anchor: .bottom)
                        }
                    }
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 14)
        .padding(.bottom, 12)
        .frame(maxHeight: .infinity)
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 9) {
            ZStack(alignment: .topLeading) {
                if feedback.isEmpty {
                    Text("Share feedback…")
                        .font(.system(size: 11.5))
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, 9)
                        .padding(.vertical, 10)
                        .allowsHitTesting(false)
                }

                TextEditor(text: $feedback)
                    .font(.system(size: 11.5))
                    .foregroundStyle(Theme.ink)
                    .scrollContentBackground(.hidden)
                    .padding(5)
                    .focused($composerFocused)
                    .accessibilityLabel("Review feedback")
                    .accessibilityHint("Share optional feedback about this screenshot.")
                    .accessibilityIdentifier("review.comment.editor")
            }
            .frame(height: 88)
            .background(Theme.elevated.opacity(0.82), in: RoundedRectangle(cornerRadius: 9))
            .overlay(
                RoundedRectangle(cornerRadius: 9)
                    .stroke(composerFocused ? Theme.purple.opacity(0.7) : Theme.lineStrong, lineWidth: 1)
            )

            if let submissionError {
                Label(submissionError, systemImage: "exclamationmark.circle.fill")
                    .font(.system(size: 10.5, weight: .medium))
                    .foregroundStyle(Theme.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("review.error")
            }

            Button {
                submitComment()
            } label: {
                Label("Send Feedback", systemImage: "arrow.up.circle")
            }
            .buttonStyle(ReviewActionButtonStyle(tone: .quiet))
            .disabled(isSubmitting || trimmedFeedback.isEmpty)
            .keyboardShortcut(.return, modifiers: [.command])
            .accessibilityHint("Sends feedback and keeps this screenshot open.")
            .accessibilityIdentifier("review.feedback.send")

            Button {
                markSeen()
            } label: {
                Label("Seen", systemImage: "checkmark.circle.fill")
            }
            .buttonStyle(ReviewActionButtonStyle(tone: .primary))
            .disabled(isSubmitting)
            .keyboardShortcut(.return, modifiers: [.command, .option])
            .accessibilityHint("Marks this screenshot as seen and returns to the stream.")
            .accessibilityIdentifier("review.seen")

            if isSubmitting {
                HStack(spacing: 7) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Saving review…")
                        .font(.system(size: 10.5))
                        .foregroundStyle(Theme.muted2)
                }
                .frame(maxWidth: .infinity, alignment: .center)
                .accessibilityIdentifier("review.saving")
            } else {
                Text("← → page  ·  ⌘↩ send  ·  ⌘⌥↩ seen")
                    .font(.system(size: 9, design: .monospaced))
                    .foregroundStyle(Theme.muted)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .accessibilityHidden(true)
            }
        }
        .padding(14)
        .background(Theme.surface.opacity(0.64))
    }

    private var comments: [ReviewComment] {
        currentShot.review?.comments ?? []
    }

    private var reviewState: ReviewState {
        currentShot.review?.state ?? .pending
    }

    private var currentShot: Shot {
        appState.shots.first(where: { $0.id == shot.id }) ?? shot
    }

    private var trimmedFeedback: String {
        feedback.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var metadataLine: String {
        var parts = [currentShot.worktree, currentShot.feature]
        if let url = currentShot.url, !url.isEmpty {
            parts.append(url)
        }
        parts.append(currentShot.capturedAt.formatted(date: .abbreviated, time: .shortened))
        return parts.joined(separator: "  ·  ")
    }

    private func navigationButton(
        symbol: String,
        help: String,
        identifier: String,
        isDisabled: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Color.white.opacity(0.7))
                .frame(width: 30, height: 30)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
        .opacity(isDisabled ? 0.35 : 1)
        .disabled(isDisabled)
        .help("\(help) (←/→)")
        .accessibilityLabel(help)
        .accessibilityIdentifier(identifier)
    }

    private func canNavigate(_ delta: Int) -> Bool {
        appState.reviewSibling(from: currentShot.id, delta: delta) != nil
    }

    private var reviewPosition: (index: Int, count: Int)? {
        appState.reviewPosition(for: currentShot.id)
    }

    private func submitComment() {
        let body = trimmedFeedback
        guard !body.isEmpty else {
            focusFeedback(message: "Write feedback before sending it.")
            return
        }

        performSubmission {
            try await appState.addReviewComment(body, to: currentShot)
            feedback = ""
        }
    }

    private func markSeen() {
        let note = trimmedFeedback

        performSubmission {
            try await appState.markSeen(
                note: note.isEmpty ? nil : note,
                for: currentShot
            )
            feedback = ""
            onClose()
        }
    }

    private func focusFeedback(message: String) {
        submissionError = message
        composerFocused = true
    }

    private func performSubmission(_ operation: @escaping @MainActor () async throws -> Void) {
        guard !isSubmitting else { return }
        isSubmitting = true
        submissionError = nil

        Task { @MainActor in
            defer { isSubmitting = false }
            do {
                try await operation()
            } catch {
                submissionError = error.localizedDescription
            }
        }
    }
}
