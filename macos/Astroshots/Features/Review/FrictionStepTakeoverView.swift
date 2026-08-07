import AppKit
import SwiftUI

/// Chromeless full-screen friction-step viewer — same shell as shot review.
///
/// Large image stage + notes rail (Looks good / Can improve). Step paging
/// walks the current run; multi-screenshot steps page with secondary controls.
@MainActor
struct FrictionStepTakeoverView: View {
    let step: FrictionLogStep
    let appState: AppState
    let onClose: () -> Void
    let onNavigateStep: (Int) -> Void

    @State private var screenshotIndex = 0

    var body: some View {
        ZStack {
            Color(hex: 0x242423)
                .ignoresSafeArea()

            VStack(spacing: 0) {
                takeoverHeader

                HStack(spacing: 0) {
                    imageStage
                    notesRail
                        .frame(width: 372)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("friction.takeover")
        .onExitCommand(perform: onClose)
        .onChange(of: step.id) {
            screenshotIndex = 0
        }
    }

    private var currentStep: FrictionLogStep {
        appState.selectedFrictionStep ?? step
    }

    private var takeoverHeader: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(String(format: "%02d", currentStep.step))
                        .font(.system(size: 11, weight: .bold, design: .monospaced))
                        .foregroundStyle(Color.white.opacity(0.62))
                        .padding(.horizontal, 7)
                        .padding(.vertical, 3)
                        .background(Color.white.opacity(0.1), in: Capsule())

                    Text(currentStep.title)
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(Color.white.opacity(0.96))
                        .lineLimit(1)
                }

                Text(metadataLine)
                    .font(.system(size: 10.5))
                    .foregroundStyle(Color.white.opacity(0.52))
                    .lineLimit(1)
            }

            Spacer(minLength: 24)

            if let position = appState.frictionStepPosition() {
                Text("\(position.index) / \(position.count)")
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    .foregroundStyle(Color.white.opacity(0.58))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(Color.white.opacity(0.08), in: Capsule())
                    .accessibilityIdentifier("friction.takeover.position")
            }

            navigationButton(
                symbol: "chevron.left",
                help: "Previous step",
                identifier: "friction.takeover.older",
                isDisabled: !appState.canStepFrictionStep(-1)
            ) {
                onNavigateStep(-1)
            }

            navigationButton(
                symbol: "chevron.right",
                help: "Next step",
                identifier: "friction.takeover.newer",
                isDisabled: !appState.canStepFrictionStep(1)
            ) {
                onNavigateStep(1)
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
            .help("Close")
            .accessibilityLabel("Close friction step review")
            .accessibilityIdentifier("friction.takeover.close")
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

    private var metadataLine: String {
        var parts: [String] = []
        if let log = appState.selectedFrictionLog {
            parts.append(log.title)
            parts.append(log.worktreeShort)
        }
        if let run = appState.selectedFrictionRun {
            parts.append(run.runID)
        }
        if let url = currentStep.url, !url.isEmpty {
            parts.append(url)
        }
        return parts.joined(separator: " · ")
    }

    private var imageStage: some View {
        GeometryReader { proxy in
            ZStack {
                Color(hex: 0x2A2A29)

                stageImage
                    .frame(
                        maxWidth: max(240, proxy.size.width - 72),
                        maxHeight: max(180, proxy.size.height - 72)
                    )
                    .padding(36)

                HStack {
                    stageNavigateButton(
                        symbol: "chevron.left",
                        help: "Previous step",
                        identifier: "friction.stage.older",
                        isDisabled: !appState.canStepFrictionStep(-1)
                    ) {
                        onNavigateStep(-1)
                    }

                    Spacer()

                    stageNavigateButton(
                        symbol: "chevron.right",
                        help: "Next step",
                        identifier: "friction.stage.newer",
                        isDisabled: !appState.canStepFrictionStep(1)
                    ) {
                        onNavigateStep(1)
                    }
                }
                .padding(.horizontal, 18)

                if currentStep.screenshotPaths.count > 1 {
                    VStack {
                        Spacer()
                        screenshotPager
                            .padding(.bottom, 20)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var stageImage: some View {
        let paths = currentStep.screenshotPaths
        if paths.isEmpty {
            VStack(spacing: 10) {
                Image(systemName: "photo.badge.exclamationmark")
                    .font(.system(size: 32, weight: .light))
                Text("No screenshot for this step")
                    .font(.system(size: 13, weight: .medium))
            }
            .foregroundStyle(Color.white.opacity(0.52))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityIdentifier("friction.takeover.image")
        } else {
            let index = min(screenshotIndex, paths.count - 1)
            let path = paths[index]
            if let image = NSImage(contentsOfFile: path) {
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
                            _ = ImageClipboard.copyImage(atPath: path)
                            appState.showToast("Copied image")
                        }
                        Button("Reveal in Finder") {
                            NSWorkspace.shared.activateFileViewerSelecting(
                                [URL(fileURLWithPath: path)]
                            )
                        }
                    }
                    .accessibilityLabel("Screenshot \(currentStep.title)")
                    .accessibilityIdentifier("friction.takeover.image")
            } else {
                VStack(spacing: 10) {
                    Image(systemName: "photo.badge.exclamationmark")
                        .font(.system(size: 32, weight: .light))
                    Text("Screenshot unavailable")
                        .font(.system(size: 13, weight: .medium))
                }
                .foregroundStyle(Color.white.opacity(0.52))
                .accessibilityIdentifier("friction.takeover.image")
            }
        }
    }

    private var screenshotPager: some View {
        let paths = currentStep.screenshotPaths
        let index = min(screenshotIndex, max(paths.count - 1, 0))
        return HStack(spacing: 10) {
            Button {
                screenshotIndex = max(0, index - 1)
            } label: {
                Image(systemName: "chevron.left")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(.white.opacity(0.9))
                    .frame(width: 28, height: 28)
                    .background(Color.black.opacity(0.45), in: Circle())
            }
            .buttonStyle(.plain)
            .disabled(index <= 0)
            .opacity(index <= 0 ? 0.3 : 1)

            Text("Image \(index + 1) / \(paths.count)")
                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                .foregroundStyle(Color.white.opacity(0.78))
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Color.black.opacity(0.45), in: Capsule())

            Button {
                screenshotIndex = min(paths.count - 1, index + 1)
            } label: {
                Image(systemName: "chevron.right")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(.white.opacity(0.9))
                    .frame(width: 28, height: 28)
                    .background(Color.black.opacity(0.45), in: Circle())
            }
            .buttonStyle(.plain)
            .disabled(index >= paths.count - 1)
            .opacity(index >= paths.count - 1 ? 0.3 : 1)
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

    private func navigationButton(
        symbol: String,
        help: String,
        identifier: String,
        isDisabled: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Color.white.opacity(0.86))
                .frame(width: 32, height: 32)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 9))
        .opacity(isDisabled ? 0.35 : 1)
        .disabled(isDisabled)
        .help(help)
        .accessibilityLabel(help)
        .accessibilityIdentifier(identifier)
    }

    private var notesRail: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Step notes")
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundStyle(Theme.ink)

                    if !currentStep.description.isEmpty {
                        Text(currentStep.description)
                            .font(.system(size: 11.5))
                            .foregroundStyle(Theme.ink2)
                            .fixedSize(horizontal: false, vertical: true)
                    }

                    if let url = currentStep.url, !url.isEmpty {
                        Text(url)
                            .font(.system(size: 11, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.blue)
                            .textSelection(.enabled)
                    }
                }
                .padding(16)

                Rectangle().fill(Theme.line).frame(height: 1)

                railNotes(
                    title: "Looks good",
                    icon: "hand.thumbsup.fill",
                    items: currentStep.good,
                    accent: Theme.green,
                    soft: Theme.greenSoft,
                    empty: "No positives noted"
                )

                Rectangle().fill(Theme.line).frame(height: 1)

                railNotes(
                    title: "Can improve",
                    icon: "wrench.and.screwdriver.fill",
                    items: currentStep.improve,
                    accent: Theme.amber,
                    soft: Theme.amberSoft,
                    empty: "No friction found"
                )
            }
        }
        .background(Theme.paper)
        .overlay(alignment: .leading) {
            Rectangle()
                .fill(Color.black.opacity(0.2))
                .frame(width: 1)
        }
        .accessibilityIdentifier("friction.takeover.rail")
    }

    private func railNotes(
        title: String,
        icon: String,
        items: [String],
        accent: Color,
        soft: Color,
        empty: String
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(accent)
                Text(title)
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(Theme.ink)
                Spacer()
                Text("\(items.count)")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(accent)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(soft, in: Capsule())
            }

            if items.isEmpty {
                Text(empty)
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.muted)
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                        HStack(alignment: .top, spacing: 10) {
                            Circle()
                                .fill(accent)
                                .frame(width: 6, height: 6)
                                .padding(.top, 5)
                            Text(item)
                                .font(.system(size: 12.5))
                                .foregroundStyle(Theme.ink2)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(soft.opacity(0.55), in: RoundedRectangle(cornerRadius: 10))
                    }
                }
            }
        }
        .padding(16)
    }
}
