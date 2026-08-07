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

    private var showsTabBar: Bool {
        appState.pane != .settings
            && appState.pane != .detail
            && appState.pane != .frictionLogDetail
            && appState.pane != .frictionStepDetail
    }

    private var tabBar: some View {
        HStack(spacing: 4) {
            ForEach(TrayTab.allCases) { tab in
                Button {
                    appState.selectTab(tab)
                } label: {
                    HStack(spacing: 5) {
                        Image(systemName: tab == .shots ? "camera.fill" : "list.bullet.rectangle")
                            .font(.system(size: 10, weight: .semibold))
                        Text(tab.label)
                            .font(.system(size: 11, weight: .semibold))
                        if tab == .shots, !appState.isEmpty {
                            Text("\(appState.shots.count)")
                                .font(.system(size: 9, weight: .bold, design: .monospaced))
                                .foregroundStyle(
                                    appState.activeTab == tab ? Theme.purple : Theme.muted2
                                )
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(
                                    (appState.activeTab == tab ? Theme.purpleSoft : Theme.surface),
                                    in: Capsule()
                                )
                        }
                        if tab == .frictionLogs, !appState.isFrictionLogsEmpty {
                            Text("\(appState.frictionLogs.count)")
                                .font(.system(size: 9, weight: .bold, design: .monospaced))
                                .foregroundStyle(
                                    appState.activeTab == tab ? Theme.purple : Theme.muted2
                                )
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(
                                    (appState.activeTab == tab ? Theme.purpleSoft : Theme.surface),
                                    in: Capsule()
                                )
                        }
                    }
                    .foregroundStyle(
                        appState.activeTab == tab ? Theme.ink : Theme.muted2
                    )
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 7)
                    .background(
                        appState.activeTab == tab
                            ? Color.white.opacity(0.92)
                            : Color.clear,
                        in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(
                                appState.activeTab == tab ? Theme.line : Color.clear,
                                lineWidth: 1
                            )
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("tray.tab.\(tab.rawValue)")
            }
        }
        .padding(4)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .padding(.horizontal, 10)
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
                    .fill(
                        appState.needsWatchRootSetup
                            ? Theme.amber
                            : (appState.isScanning
                                ? Theme.amber
                                : (isCatalogEmpty ? Color(hex: 0xB0AAA2) : Color(hex: 0x20A37F)))
                    )
                    .frame(width: 5, height: 5)
                    .shadow(
                        color: appState.needsWatchRootSetup
                            || isCatalogEmpty
                            || appState.isScanning
                            ? .clear
                            : Color(hex: 0x20A37F).opacity(0.35),
                        radius: 3
                    )
                Text(
                    appState.needsWatchRootSetup
                        ? "setup"
                        : (appState.isScanning ? "scanning" : (isCatalogEmpty ? "idle" : "live"))
                )
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(
                        appState.needsWatchRootSetup || appState.isScanning
                            ? Theme.amber
                            : (isCatalogEmpty ? Theme.muted : Theme.green)
                    )
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
                    trayChrome.close()
                } else {
                    trayChrome.openPinned()
                }
            }

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
