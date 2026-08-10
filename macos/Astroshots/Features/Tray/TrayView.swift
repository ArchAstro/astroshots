import SwiftUI

/// Menu-bar tray: stream home, detail drill-in, settings via gear.
struct TrayView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.trayChrome) private var trayChrome

    var isPinned = false

    var body: some View {
        VStack(spacing: 0) {
            header
            if showsTabBar {
                tabBar
            }
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

    private var isCatalogEmpty: Bool {
        appState.isEmpty && appState.isFrictionLogsEmpty
    }

    private var statusLabel: String {
        if appState.needsWatchRootSetup { return "setup" }
        if appState.isScanning { return "scanning" }
        if appState.isFixtureSession { return "seeded" }
        if isCatalogEmpty { return "idle" }
        return "live"
    }

    private var statusColor: Color {
        if appState.needsWatchRootSetup || appState.isScanning { return Theme.amber }
        if appState.isFixtureSession { return Theme.blue }
        if isCatalogEmpty { return Theme.muted }
        return Theme.green
    }

    private var statusDotColor: Color {
        if appState.needsWatchRootSetup || appState.isScanning { return Theme.amber }
        if appState.isFixtureSession { return Theme.blue }
        if isCatalogEmpty { return Color(hex: 0xB0AAA2) }
        return Color(hex: 0x20A37F)
    }

    private var showsTabBar: Bool {
        appState.pane != .settings
            && appState.pane != .detail
            && appState.pane != .frictionLogDetail
            && appState.pane != .frictionStepDetail
    }

    /// Equal-width text tabs — matches agent-rooms tray (no icons, no purple chips).
    private var tabBar: some View {
        HStack(spacing: 2) {
            ForEach(Array(TrayTab.allCases.enumerated()), id: \.element.id) { index, tab in
                Button {
                    appState.selectTab(tab)
                } label: {
                    Text(tab.label)
                        .font(.system(size: 10, weight: .semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 6)
                        .foregroundStyle(appState.activeTab == tab ? Theme.ink : Theme.muted)
                        .background(
                            appState.activeTab == tab ? Color.white : Color.clear,
                            in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                        )
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .keyboardShortcut(
                    KeyEquivalent(Character("\(index + 1)")),
                    modifiers: .command
                )
                .accessibilityIdentifier("tray.tab.\(tab.rawValue)")
                .accessibilityAddTraits(appState.activeTab == tab ? .isSelected : [])
            }
        }
        .padding(.horizontal, 14)
        .padding(.bottom, 8)
    }

    private var header: some View {
        HStack(spacing: 8) {
            HStack(spacing: 8) {
                Image("AstroshotsMark")
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .foregroundStyle(.white)
                    .padding(5)
                    .frame(width: 24, height: 24)
                    .background(
                        LinearGradient(
                            colors: [Theme.green, Theme.purple],
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
                    .fill(statusDotColor)
                    .frame(width: 5, height: 5)
                    .shadow(
                        color: statusLabel == "live"
                            ? Color(hex: 0x20A37F).opacity(0.35)
                            : .clear,
                        radius: 3
                    )
                Text(statusLabel)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(statusColor)
                    .accessibilityIdentifier("tray.status")
            }

            Spacer(minLength: 4)

            iconButton(
                systemImage: "gearshape",
                title: "Settings",
                active: appState.pane == .settings
            ) {
                appState.openSettings()
            }

            // Labeled pin control (friction log: icon-only was easy to miss).
            Button {
                if isPinned {
                    trayChrome.close()
                } else {
                    trayChrome.openPinned()
                }
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: isPinned ? "pin.slash" : "pin")
                        .font(.system(size: 10, weight: .semibold))
                    Text(isPinned ? "Unpin" : "Pin")
                        .font(.system(size: 10, weight: .semibold))
                }
                .foregroundStyle(isPinned ? Theme.purple : Theme.ink2)
                .padding(.horizontal, 8)
                .frame(height: 28)
                .background(
                    isPinned ? Theme.purpleSoft : Theme.surface,
                    in: RoundedRectangle(cornerRadius: 6, style: .continuous)
                )
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(isPinned ? "Back to menu bar" : "Keep tray visible as a window")
            .accessibilityLabel(isPinned ? "Unpin tray" : "Pin tray keep visible")
            .accessibilityIdentifier("tray.pin")

            if !isPinned {
                iconButton(systemImage: "xmark", title: "Close") {
                    trayChrome.close()
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
            switch appState.activeTab {
            case .shots:
                StreamView()
            case .frictionLogs:
                FrictionLogListView()
            }
        case .detail:
            DetailView()
        case .frictionLogDetail:
            FrictionLogDetailView()
        case .frictionStepDetail:
            FrictionStepDetailView()
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
