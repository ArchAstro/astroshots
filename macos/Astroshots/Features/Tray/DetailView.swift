import SwiftUI

struct DetailView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.reviewChrome) private var reviewChrome
    @State private var reviewNote = ""
    @State private var submissionError: String?
    @State private var isSubmitting = false
    @State private var activeSubmissionID: UUID?

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

                Button {
                    appState.stepDetail(1)
                } label: {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.ink2)
                        .frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .help("Older")

                Button {
                    appState.stepDetail(-1)
                } label: {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Theme.ink2)
                        .frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .help("Newer")
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
                            ShotThumbnail(path: shot.path)
                                .aspectRatio(16 / 10, contentMode: .fit)
                                .frame(maxWidth: .infinity)
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
                                ReviewBadge(state: shot.review?.state ?? .pending)
                                    .fixedSize()
                            }
                            Text(shot.description.isEmpty ? shot.fileName : shot.description)
                                .font(.system(size: 11))
                                .foregroundStyle(Theme.ink2)
                                .fixedSize(horizontal: false, vertical: true)
                                .padding(.bottom, 12)

                            compactReviewControls(for: shot)
                                .padding(.bottom, 12)

                            VStack(alignment: .leading, spacing: 6) {
                                row("Tree", shot.worktree)
                                row("Feature", shot.feature)
                                row("File", shot.fileName)
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
                                    .fill(Color.white.opacity(0.7))
                                    .overlay(
                                        RoundedRectangle(cornerRadius: 11)
                                            .stroke(Theme.line, lineWidth: 1)
                                    )
                            )
                            .accessibilityElement(children: .contain)
                            .accessibilityIdentifier("detail.metadata")
                        }
                        .padding(.horizontal, 14)
                        .padding(.top, 12)
                        .padding(.bottom, 16)
                        .frame(maxWidth: .infinity, alignment: .leading)
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
                }
            } else {
                EmptyStreamView()
            }
        }
    }

    private func reviewLabel(for shot: Shot) -> String {
        switch shot.review?.state ?? .pending {
        case .pending: "Pending"
        case .approved: "Approved"
        case .changesRequested: "Changes requested"
        }
    }

    private func heading(for shot: Shot) -> String {
        if let sequence = shot.sequence {
            return "\(sequence) · \(shot.slug)"
        }
        return shot.slug
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
                "Feedback — required when requesting changes",
                text: $reviewNote,
                axis: .vertical
            )
            .textFieldStyle(.plain)
            .font(.system(size: 11))
            .foregroundStyle(Theme.ink)
            .lineLimit(1...3)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Color.white.opacity(0.78), in: RoundedRectangle(cornerRadius: 9))
            .overlay(
                RoundedRectangle(cornerRadius: 9)
                    .stroke(Theme.lineStrong, lineWidth: 1)
            )
            .accessibilityLabel("Compact review feedback")
            .accessibilityHint("Feedback is required when requesting changes.")
            .accessibilityIdentifier("detail.review.note")

            if let submissionError {
                Label(submissionError, systemImage: "exclamationmark.circle.fill")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(Theme.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("detail.review.error")
            }

            HStack(spacing: 8) {
                Button {
                    submitDecision(.approved, for: shot)
                } label: {
                    Label("Approve", systemImage: "checkmark")
                }
                .buttonStyle(ReviewActionButtonStyle(tone: .primary))
                .disabled(isSubmitting)
                .accessibilityIdentifier("detail.review.approve")

                Button {
                    submitDecision(.changesRequested, for: shot)
                } label: {
                    Label("Request changes", systemImage: "exclamationmark.bubble")
                }
                .buttonStyle(ReviewActionButtonStyle(tone: .destructive))
                .disabled(isSubmitting || trimmedReviewNote.isEmpty)
                .accessibilityHint("Requires feedback in the field above.")
                .accessibilityIdentifier("detail.review.requestChanges")
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

    private func submitDecision(_ decision: ReviewDecision, for shot: Shot) {
        let note = trimmedReviewNote
        if case .changesRequested = decision, note.isEmpty {
            submissionError = "Explain what needs to change before requesting changes."
            return
        }

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
                try await appState.setReviewDecision(
                    decision,
                    note: note.isEmpty ? nil : note,
                    for: shot
                )
                if activeSubmissionID == submissionID,
                   appState.selectedShotID == shot.id
                {
                    reviewNote = ""
                }
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
