import AppKit
import SwiftUI

/// Compact tray page for one friction-log step — mirrors shot DetailView.
/// Screenshot expands into the full-screen friction step viewer.
struct FrictionStepDetailView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.reviewChrome) private var reviewChrome
    @State private var selectedScreenshotIndex = 0

    var body: some View {
        VStack(spacing: 0) {
            chrome
            if let step = appState.selectedFrictionStep {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        preview(for: step)
                        bodyContent(for: step)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .scrollIndicators(.hidden)
                .accessibilityIdentifier("friction.step.detail")
                .onChange(of: step.id) {
                    selectedScreenshotIndex = 0
                }
            } else {
                EmptyFrictionLogsView()
            }
        }
    }

    private var chrome: some View {
        HStack(spacing: 6) {
            Button {
                appState.backToFrictionLogDetail()
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 10, weight: .semibold))
                    Text("Steps")
                        .font(.system(size: 11, weight: .semibold))
                }
                .foregroundStyle(Theme.ink2)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("friction.step.back")

            Spacer()

            if let position = appState.frictionStepPosition() {
                Text("\(position.index) / \(position.count)")
                    .font(.system(size: 10, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Theme.muted2)
                    .accessibilityIdentifier("friction.step.position")
            }

            Button {
                appState.stepFrictionStep(-1)
            } label: {
                Image(systemName: "chevron.left")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.ink2)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .opacity(appState.canStepFrictionStep(-1) ? 1 : 0.32)
            .disabled(!appState.canStepFrictionStep(-1))
            .help("Previous step (←)")
            .keyboardShortcut(.leftArrow, modifiers: [])

            Button {
                appState.stepFrictionStep(1)
            } label: {
                Image(systemName: "chevron.right")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.ink2)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .opacity(appState.canStepFrictionStep(1) ? 1 : 0.32)
            .disabled(!appState.canStepFrictionStep(1))
            .help("Next step (→)")
            .keyboardShortcut(.rightArrow, modifiers: [])
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.line).frame(height: 1)
        }
    }

    @ViewBuilder
    private func preview(for step: FrictionLogStep) -> some View {
        let paths = step.screenshotPaths
        if paths.isEmpty {
            HStack(spacing: 8) {
                Image(systemName: "photo")
                    .foregroundStyle(Theme.muted)
                Text("No screenshot for this step")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(Theme.muted)
            }
            .frame(maxWidth: .infinity)
            .frame(height: 140)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .padding(.horizontal, 12)
            .padding(.top, 12)
        } else {
            let index = min(selectedScreenshotIndex, paths.count - 1)
            let path = paths[index]

            VStack(spacing: 8) {
                Button {
                    reviewChrome.openFrictionStep(step)
                } label: {
                    ShotThumbnail(path: path, contentMode: .fit)
                        .frame(maxHeight: 240)
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
                .help("Open full-screen step review")
                .accessibilityLabel("Review \(step.title) full screen")
                .accessibilityIdentifier("friction.step.preview")
                .contextMenu {
                    Button("Copy Image") {
                        _ = ImageClipboard.copyImage(atPath: path)
                        appState.showToast("Copied image")
                    }
                    Button("Reveal in Finder") {
                        NSWorkspace.shared.activateFileViewerSelecting(
                            [URL(fileURLWithPath: path)]
                        )
                    }
                }

                if paths.count > 1 {
                    HStack(spacing: 6) {
                        ForEach(Array(paths.enumerated()), id: \.offset) { offset, _ in
                            Button {
                                selectedScreenshotIndex = offset
                            } label: {
                                Circle()
                                    .fill(offset == index ? Theme.purple : Theme.lineStrong)
                                    .frame(width: 7, height: 7)
                            }
                            .buttonStyle(.plain)
                        }
                        Spacer()
                        Text("\(index + 1) of \(paths.count)")
                            .font(.system(size: 10, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.muted)
                    }
                }
            }
            .padding(.horizontal, 12)
            .padding(.top, 12)
        }
    }

    private func bodyContent(for step: FrictionLogStep) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(String(format: "%02d", step.step))
                        .font(.system(size: 12, weight: .bold, design: .monospaced))
                        .foregroundStyle(Theme.purple)
                    Text(step.title)
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(Theme.ink)
                        .tracking(-0.3)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if !step.description.isEmpty {
                    Text(step.description)
                        .font(.system(size: 11))
                        .foregroundStyle(Theme.ink2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let url = step.url, !url.isEmpty {
                    Text(url)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundStyle(Theme.blue)
                        .textSelection(.enabled)
                }
            }

            notesSection(
                title: "Looks good",
                icon: "hand.thumbsup.fill",
                items: step.good,
                accent: Theme.green,
                soft: Theme.greenSoft,
                empty: "No positives noted for this step"
            )

            notesSection(
                title: "Can improve",
                icon: "wrench.and.screwdriver.fill",
                items: step.improve,
                accent: Theme.amber,
                soft: Theme.amberSoft,
                empty: "No friction found for this step"
            )

            if let log = appState.selectedFrictionLog,
               let run = appState.selectedFrictionRun
            {
                VStack(alignment: .leading, spacing: 6) {
                    metaRow("Log", log.slug)
                    metaRow("Run", run.runID)
                    metaRow("Tree", log.worktree)
                    if let path = step.primaryScreenshotPath {
                        metaRow("File", URL(fileURLWithPath: path).lastPathComponent)
                    }
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
        }
        .padding(.horizontal, 14)
        .padding(.top, 12)
        .padding(.bottom, 16)
    }

    private func notesSection(
        title: String,
        icon: String,
        items: [String],
        accent: Color,
        soft: Color,
        empty: String
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(accent)
                Text(title)
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(Theme.ink)
                Spacer()
                Text("\(items.count)")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(accent)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(soft, in: Capsule())
            }

            if items.isEmpty {
                Text(empty)
                    .font(.system(size: 11))
                    .foregroundStyle(Theme.muted)
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                        HStack(alignment: .top, spacing: 8) {
                            Circle()
                                .fill(accent)
                                .frame(width: 5, height: 5)
                                .padding(.top, 5)
                            Text(item)
                                .font(.system(size: 11))
                                .foregroundStyle(Theme.ink2)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(soft.opacity(0.45), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(accent.opacity(0.18), lineWidth: 1)
        )
    }

    private func metaRow(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text(label)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Theme.muted)
                .frame(width: 48, alignment: .leading)
            Text(value)
                .font(.system(size: 10, weight: .medium, design: .monospaced))
                .foregroundStyle(Theme.ink2)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
