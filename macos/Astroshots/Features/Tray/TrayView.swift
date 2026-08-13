import AppKit
import SwiftUI

/// Menu-bar tray: stream home, detail drill-in, settings via gear.
struct TrayView: View {
    @Environment(AppState.self) private var appState
    @Environment(\.trayChrome) private var trayChrome

    var isPinned = false

    var body: some View {
        VStack(spacing: 0) {
            header
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(width: Theme.trayWidth, height: Theme.trayHeight)
        .background(Theme.paper)
        .foregroundStyle(Theme.ink)
        .overlay(alignment: .bottom) { toastOverlay }
        .onAppear {
            appState.markTrayOpened()
        }
    }

    private var isCatalogEmpty: Bool {
        appState.isEmpty && appState.isFrictionLogsEmpty
    }

    private var statusLabel: String {
        if appState.needsWatchRootSetup { return "Choose watch folders" }
        if appState.isScanning { return "Scanning for captures" }
        if appState.isFixtureSession { return "Preview data" }
        if isCatalogEmpty { return "Waiting for captures" }
        return "Live review stream"
    }

    private var statusColor: Color {
        if appState.needsWatchRootSetup || appState.isScanning { return Theme.amber }
        if appState.isFixtureSession { return Theme.blue }
        if isCatalogEmpty { return Theme.muted }
        return Theme.green
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
                        .frame(width: 99)
                        .padding(.vertical, 6)
                        .foregroundStyle(appState.activeTab == tab ? Theme.ink : Theme.muted)
                        .background(
                            appState.activeTab == tab ? Theme.elevated : Color.clear,
                            in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                        )
                        .frame(maxWidth: .infinity)
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
        VStack(spacing: 0) {
            HStack(spacing: 9) {
                Image("AstroshotsMark")
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .foregroundStyle(.white)
                    .padding(5)
                    .frame(width: 27, height: 27)
                    .background(Theme.green, in: RoundedRectangle(cornerRadius: 8))

                VStack(alignment: .leading, spacing: 1) {
                    Text("Astroshots")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(Theme.ink)
                    Text(statusLabel)
                        .font(.system(size: 9))
                        .foregroundStyle(statusColor)
                        .lineLimit(1)
                        .accessibilityIdentifier("tray.status")
                }

                Spacer()

                iconButton(
                    systemImage: "arrow.up.right.square",
                    title: "Show current capture in Finder"
                ) {
                    showCurrentCaptureInFinder()
                }

                iconButton(
                    systemImage: "gearshape.fill",
                    title: "Settings",
                    active: appState.pane == .settings
                ) {
                    appState.openSettings()
                }

                Button {
                    if isPinned {
                        trayChrome.close()
                    } else {
                        trayChrome.openPinned()
                    }
                } label: {
                    Image(systemName: "pin")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(isPinned ? Theme.green : Theme.muted)
                        .frame(width: 28, height: 28)
                        .background(
                            isPinned ? Theme.greenSoft : Color.clear,
                            in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                        )
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .hoverHighlight()
                .help(isPinned ? "Back to menu bar" : "Keep tray visible as a window")
                .accessibilityLabel(isPinned ? "Unpin tray" : "Pin tray keep visible")
                .accessibilityIdentifier("tray.pin")

                if !isPinned {
                    iconButton(systemImage: "xmark", title: "Close") {
                        trayChrome.close()
                    }
                }
            }
            .frame(height: 43)
            .padding(.horizontal, 14)

            if showsTabBar {
                tabBar
            }

            Rectangle().fill(Theme.line).frame(height: 1)
        }
        .padding(.top, isPinned ? 8 : 4)
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
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(active ? Theme.green : Theme.muted)
                .frame(width: 28, height: 28)
                .background(
                    active ? Theme.greenSoft : Color.clear,
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .hoverHighlight()
        .help(title)
    }

    private func showCurrentCaptureInFinder() {
        let currentPath = appState.selectedFrictionStep?.primaryScreenshotPath
            ?? appState.selectedShot?.path
            ?? appState.watchRootPaths.first

        guard let currentPath else {
            appState.openSettings()
            return
        }

        NSWorkspace.shared.activateFileViewerSelecting([
            URL(fileURLWithPath: currentPath),
        ])
    }
}
