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
                        Text("Watch root")
                            .font(.system(size: 12, weight: .semibold))
                        Text("One recursive root. Every worktree under it streams in — no picker.")
                            .font(.system(size: 10))
                            .foregroundStyle(Theme.muted)
                            .padding(.bottom, 4)

                        HStack(spacing: 7) {
                            Circle()
                                .fill(Color(hex: 0x20A37F))
                                .frame(width: 6, height: 6)
                            Text(displayPath(appState.watchRootPath))
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundStyle(Theme.ink2)
                                .lineLimit(2)
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 6)
                        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 8))

                        HStack(spacing: 8) {
                            Button("Choose…") {
                                appState.chooseWatchRoot()
                            }
                            .buttonStyle(.plain)
                            .font(.system(size: 11, weight: .semibold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))

                            Button("Rescan") {
                                appState.rescan()
                            }
                            .buttonStyle(.plain)
                            .font(.system(size: 11, weight: .semibold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 7))
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
                }
                .padding(12)
            }
            .scrollIndicators(.hidden)
        }
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
