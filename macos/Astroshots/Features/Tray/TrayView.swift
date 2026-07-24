import SwiftUI

/// Menu-bar tray: stream home, detail drill-in, settings via gear.
struct TrayView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openWindow) private var openWindow

    var isPinned = false

    var body: some View {
        VStack(spacing: 0) {
            header
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(width: Theme.trayWidth, height: Theme.trayHeight)
        .background(Theme.paper)
        .overlay(alignment: .bottom) { toastOverlay }
        .onAppear {
            appState.markTrayOpened()
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            HStack(spacing: 8) {
                Text("◆")
                    .font(.system(size: 10, weight: .heavy))
                    .foregroundStyle(.white)
                    .frame(width: 24, height: 24)
                    .background(
                        LinearGradient(
                            colors: [Color(hex: 0x1F6A55), Color(hex: 0x2F9D7D)],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        in: RoundedRectangle(cornerRadius: 7)
                    )
                Text("Astroshots")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(Theme.ink)
                    .tracking(-0.15)
            }

            HStack(spacing: 5) {
                Circle()
                    .fill(appState.isEmpty ? Color(hex: 0xB0AAA2) : Color(hex: 0x20A37F))
                    .frame(width: 5, height: 5)
                    .shadow(
                        color: appState.isEmpty ? .clear : Color(hex: 0x20A37F).opacity(0.35),
                        radius: 3
                    )
                Text(appState.isEmpty ? "idle" : "live")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(appState.isEmpty ? Theme.muted : Theme.green)
            }

            Spacer(minLength: 4)

            iconButton(
                systemImage: "gearshape",
                title: "Settings",
                active: appState.pane == .settings
            ) {
                appState.openSettings()
            }

            iconButton(
                systemImage: "pin",
                title: isPinned ? "Back to menu bar" : "Keep visible",
                active: isPinned
            ) {
                if isPinned {
                    dismiss()
                } else {
                    openWindow(id: "astroshots-panel")
                    dismiss()
                }
            }

            if !isPinned {
                iconButton(systemImage: "xmark", title: "Close") {
                    dismiss()
                }
            }
        }
        .frame(height: 36)
        .padding(.horizontal, 12)
        .padding(.top, isPinned ? 8 : 4)
        .padding(.bottom, 10)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.line).frame(height: 1)
        }
    }

    @ViewBuilder
    private var content: some View {
        switch appState.pane {
        case .stream:
            StreamView()
        case .detail:
            DetailView()
        case .settings:
            SettingsPane()
        }
    }

    @ViewBuilder
    private var toastOverlay: some View {
        if let toast = appState.toast {
            Text(toast)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.white)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    Color(hex: 0x201E1B).opacity(0.9),
                    in: RoundedRectangle(cornerRadius: 9)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 9)
                        .stroke(Color.white.opacity(0.13), lineWidth: 1)
                )
                .shadow(color: .black.opacity(0.24), radius: 17, y: 6)
                .padding(.bottom, 16)
                .transition(.move(edge: .bottom).combined(with: .opacity))
                .id(toast)
        }
    }

    private func iconButton(
        systemImage: String,
        title: String,
        active: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(active ? Theme.purple : Color(hex: 0x595650))
                .frame(width: 28, height: 28)
                .background(
                    active ? Theme.purpleSoft : Color.clear,
                    in: RoundedRectangle(cornerRadius: 7)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(title)
    }
}
