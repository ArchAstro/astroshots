import SwiftUI

struct SettingsPane: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        VStack(spacing: 0) {
            HStack {
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
                }
                .buttonStyle(.plain)
                Spacer()
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Theme.line).frame(height: 1)
            }

            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    card {
                        Text("Desktop flash")
                            .font(.system(size: 12, weight: .semibold))
                        Text("New frames from any worktree appear above all windows.")
                            .font(.system(size: 10))
                            .foregroundStyle(Theme.muted)
                            .padding(.bottom, 4)

                        toggleRow("Show overlay", isOn: appState.overlayEnabled) {
                            appState.setOverlayEnabled($0)
                        }
                        toggleRow("Auto-dismiss", isOn: appState.autoDismiss) {
                            appState.setAutoDismiss($0)
                        }
                    }

                    card {
                        Text("Watched folders")
                            .font(.system(size: 12, weight: .semibold))
                        Text(
                            appState.needsWatchRootSetup
                                ? "Pick the directories that contain your projects. Nothing is watched until you choose at least one."
                                : "Every worktree below any of these folders streams into one feed."
                        )
                            .font(.system(size: 10))
                            .foregroundStyle(Theme.muted)
                            .padding(.bottom, 4)

                        if appState.watchRootPaths.isEmpty {
                            Text("No folders yet")
                                .font(.system(size: 10))
                                .foregroundStyle(Theme.muted)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 6)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(
                                    Theme.surface,
                                    in: RoundedRectangle(cornerRadius: 8)
                                )
                        }

                        ForEach(
                            Array(appState.watchRootPaths.enumerated()),
                            id: \.element
                        ) { index, path in
                            HStack(spacing: 7) {
                                Circle()
                                    .fill(Color(hex: 0x20A37F))
                                    .frame(width: 6, height: 6)
                                Text(displayPath(path))
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundStyle(Theme.ink2)
                                    .lineLimit(2)
                                Spacer(minLength: 0)
                                Button {
                                    appState.removeWatchRoot(path)
                                } label: {
                                    Image(systemName: "xmark")
                                        .font(.system(size: 9, weight: .semibold))
                                        .foregroundStyle(Theme.muted)
                                        .frame(width: 18, height: 18)
                                }
                                .buttonStyle(.plain)
                                .disabled(appState.watchRootPaths.count == 1)
                                .help(
                                    appState.watchRootPaths.count == 1
                                        ? "Add another folder before removing this one"
                                        : "Stop watching this folder"
                                )
                                .accessibilityIdentifier("remove-watch-root-\(index)")
                                .accessibilityLabel(
                                    "Stop watching \(displayPath(path))"
                                )
                            }
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                            .background(
                                Theme.surface,
                                in: RoundedRectangle(cornerRadius: 8)
                            )
                            .accessibilityIdentifier("watch-root-\(index)")
                        }

                        HStack(spacing: 8) {
                            Button(appState.needsWatchRootSetup ? "Choose folders…" : "Add folders…") {
                                _ = appState.chooseWatchRoots(
                                    forSetup: appState.needsWatchRootSetup
                                )
                            }
                            .buttonStyle(.plain)
                            .font(.system(size: 11, weight: .semibold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))
                            .accessibilityIdentifier("add-watch-roots")

                            if !appState.needsWatchRootSetup {
                                Button("Rescan") {
                                    appState.rescan()
                                }
                                .buttonStyle(.plain)
                                .font(.system(size: 11, weight: .semibold))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))
                            }
                        }
                        .padding(.top, 6)
                    }

                    card {
                        Text("Harness layout")
                            .font(.system(size: 12, weight: .semibold))
                        Text("Write frames here so Astroshots can find them:")
                            .font(.system(size: 10))
                            .foregroundStyle(Theme.muted)
                        Text(".astroshot/<feature>/\n  manifest.json\n  0001-slug.png")
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(Theme.ink2)
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 9))
                    }

                    updatesCard

                    narrationCard
                }
                .padding(12)
            }
            .scrollIndicators(.hidden)
        }
    }

    /// Sparkle preferences UI — same shape as Sparkle’s documented SwiftUI sample:
    /// model seeded from `SPUUpdater`, write only on user change (via didSet).
    @ViewBuilder
    private var updatesCard: some View {
        if let settings = appState.updaterSettings {
            SoftwareUpdateSettingsCard(settings: settings)
        }
    }

    private var narrationCard: some View {
        let readiness = appState.narration.readiness
        return card {
            Text("Narration")
                .font(.system(size: 12, weight: .semibold))
            Text(
                "Opt-in. Uses on-device mlx-audio-swift (MLX Swift) with Qwen3-TTS to turn friction-log transcripts into narrated MP4s."
            )
            .font(.system(size: 10))
            .foregroundStyle(Theme.muted)
            .padding(.bottom, 4)

            toggleRow(
                "Enable narration",
                isOn: appState.narrationEnabled
            ) { enabled in
                appState.setNarrationEnabled(enabled)
            }

            if appState.narrationEnabled {
                HStack(spacing: 8) {
                    Text("Voice")
                        .font(.system(size: 11, weight: .semibold))
                    Spacer(minLength: 8)
                    Picker(
                        "Voice",
                        selection: Binding(
                            get: { appState.narrationVoice },
                            set: { voice in
                                appState.narration.stopPreview()
                                appState.setNarrationVoice(voice)
                            }
                        )
                    ) {
                        ForEach(NarrationVoice.available) { voice in
                            Text("\(voice.displayName) · \(voice.language)")
                                .tag(voice.id)
                        }
                    }
                    .labelsHidden()
                    .frame(width: 150)
                    .accessibilityIdentifier("settings.narration.voice")

                    Button {
                        appState.narration.previewVoice(appState.narrationVoice)
                    } label: {
                        Image(
                            systemName: appState.narration.previewingVoice == appState.narrationVoice
                                ? "stop.fill"
                                : "play.fill"
                        )
                        .font(.system(size: 9, weight: .semibold))
                        .frame(width: 24, height: 22)
                    }
                    .buttonStyle(.plain)
                    .background(Theme.purpleSoft, in: RoundedRectangle(cornerRadius: 7))
                    .disabled(!readiness.isReady)
                    .help(readiness.isReady ? "Hear this voice" : "Narration is not ready yet")
                    .accessibilityLabel(
                        appState.narration.previewingVoice == appState.narrationVoice
                            ? "Stop voice sample"
                            : "Play voice sample"
                    )
                    .accessibilityIdentifier("settings.narration.voice-preview")
                }
                .padding(.top, 4)
                .overlay(alignment: .top) {
                    Rectangle().fill(Theme.line).frame(height: 1)
                }
            }

            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    statusDot(for: readiness)
                    Text(readiness.statusLine)
                        .font(.system(size: 10.5, weight: .medium))
                        .foregroundStyle(Theme.ink2)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }

                if let fraction = readiness.progressFraction {
                    ProgressView(value: fraction)
                        .progressViewStyle(.linear)
                        .tint(Theme.purple)
                        .accessibilityIdentifier("settings.narration.progress")
                }

                if case .failed = readiness {
                    Button("Retry setup") {
                        appState.narration.retry()
                    }
                    .buttonStyle(.plain)
                    .font(.system(size: 11, weight: .semibold))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))
                    .accessibilityIdentifier("settings.narration.retry")
                }

                if let error = appState.narration.lastError, readiness.isReady {
                    Text(error)
                        .font(.system(size: 9.5))
                        .foregroundStyle(Theme.red)
                        .fixedSize(horizontal: false, vertical: true)
                }

                Text("Requires Apple Silicon. Qwen3-TTS downloads via Hugging Face into the MLX Hub cache; generation runs in-process.")
                    .font(.system(size: 9.5))
                    .foregroundStyle(Theme.muted)
            }
            .padding(.top, 6)
            .accessibilityIdentifier("settings.narration")
        }
    }

    private func statusDot(for readiness: NarrationReadiness) -> some View {
        let color: Color = {
            switch readiness {
            case .ready: return Theme.green
            case .failed, .unsupported: return Theme.red
            case .disabled: return Theme.muted2
            case .checking, .loadingModel, .downloadingModel:
                return Theme.amber
            }
        }()
        return Circle()
            .fill(color)
            .frame(width: 7, height: 7)
    }

    @ViewBuilder
    private func card<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            content()
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 12)
                .fill(Color.white.opacity(0.8))
                .overlay(
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(Theme.line, lineWidth: 1)
                )
        )
    }

    private func toggleRow(_ title: String, isOn: Bool, set: @escaping (Bool) -> Void) -> some View {
        HStack {
            Text(title)
                .font(.system(size: 11, weight: .semibold))
            Spacer()
            Toggle("", isOn: Binding(
                get: { isOn },
                set: set
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .controlSize(.small)
        }
        .padding(.top, 4)
        .overlay(alignment: .top) {
            Rectangle().fill(Theme.line).frame(height: 1)
        }
        .padding(.top, 6)
    }

    private func displayPath(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path.hasPrefix(home) {
            return "~" + path.dropFirst(home.count)
        }
        return path
    }
}

/// Software Update card: binds to Sparkle’s `SPUUpdater` through
/// `UpdaterSettingsModel` (idiomatic Sparkle + SwiftUI Observation).
private struct SoftwareUpdateSettingsCard: View {
    @Bindable var settings: UpdaterSettingsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Software Update")
                .font(.system(size: 12, weight: .semibold))
            Text("Checks GitHub Releases for new versions and installs them in place.")
                .font(.system(size: 10))
                .foregroundStyle(Theme.muted)
                .padding(.bottom, 4)

            HStack {
                Text("Automatically check for updates")
                    .font(.system(size: 11, weight: .semibold))
                Spacer()
                Toggle("", isOn: $settings.automaticallyChecksForUpdates)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .accessibilityIdentifier("settings.auto-check-updates")
            }
            .padding(.top, 4)

            HStack {
                Text("Automatically download updates")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(
                        settings.automaticallyChecksForUpdates
                            ? Theme.ink
                            : Theme.muted
                    )
                Spacer()
                Toggle("", isOn: $settings.automaticallyDownloadsUpdates)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .disabled(!settings.automaticallyChecksForUpdates)
                    .accessibilityIdentifier("settings.auto-download-updates")
            }
            .padding(.top, 4)

            Button {
                settings.checkForUpdates()
            } label: {
                HStack(spacing: 6) {
                    if settings.isCheckingForUpdates {
                        ProgressView()
                            .controlSize(.small)
                    }
                    Text(settings.isCheckingForUpdates ? "Checking…" : "Check for Updates…")
                }
            }
            .buttonStyle(.plain)
            .font(.system(size: 11, weight: .semibold))
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))
            .disabled(!settings.canCheckForUpdates)
            .padding(.top, 6)
            .accessibilityIdentifier("settings.check-for-updates")
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 12)
                .fill(Color.white.opacity(0.8))
                .overlay(
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(Theme.line, lineWidth: 1)
                )
        )
    }
}
