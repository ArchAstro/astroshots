import SwiftUI

struct DetailView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.reviewChrome) private var reviewChrome

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
                                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                                        .stroke(Theme.line, lineWidth: 1)
                                )
                                .overlay(alignment: .bottomTrailing) {
                                    Label("Review full screen", systemImage: "arrow.up.left.and.arrow.down.right")
                                        .font(.system(size: 10, weight: .semibold))
                                        .foregroundStyle(.white)
                                        .padding(.horizontal, 9)
                                        .padding(.vertical, 6)
                                        .background(.black.opacity(0.68), in: Capsule())
                                        .padding(10)
                                }
                        }
                        .buttonStyle(.plain)
                        .help("Open full-screen review")
                        .accessibilityLabel("Review \(shot.title) full screen")
                        .padding(.horizontal, 12)
                        .padding(.top, 12)

                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text(heading(for: shot))
                                    .font(.system(size: 15, weight: .semibold))
                                    .foregroundStyle(Theme.ink)
                                    .tracking(-0.3)
                                Spacer()
                                ReviewBadge(state: shot.review?.state ?? .pending)
                            }
                            Text(shot.description.isEmpty ? shot.fileName : shot.description)
                                .font(.system(size: 11))
                                .foregroundStyle(Theme.ink2)
                                .fixedSize(horizontal: false, vertical: true)
                                .padding(.bottom, 12)

                            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 6) {
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
                        }
                        .padding(.horizontal, 14)
                        .padding(.top, 12)
                        .padding(.bottom, 16)
                    }
                }
                .scrollIndicators(.hidden)
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
        GridRow {
            Text(label)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Theme.muted2)
                .frame(width: 56, alignment: .leading)
            Text(value)
                .font(.system(size: 9.5, design: .monospaced))
                .foregroundStyle(Theme.ink)
                .textSelection(.enabled)
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
