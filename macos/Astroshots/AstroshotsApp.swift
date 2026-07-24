import SwiftUI

/// Astroshots lives in the menu bar: stream of harness screenshots, desktop
/// overlay flash for new frames, detail drill-in. See docs/mocks/astroshots-menubar.html.
@main
struct AstroshotsApp: App {
    @State private var appState = AppState()

    var body: some Scene {
        MenuBarExtra {
            TrayView()
                .environment(appState)
        } label: {
            Image(systemName: appState.isEmpty ? "camera" : "camera.fill")
            if appState.unreadCount > 0 {
                Text("\(appState.unreadCount)")
            }
        }
        .menuBarExtraStyle(.window)

        Window("Astroshots", id: "astroshots-panel") {
            TrayView(isPinned: true)
                .environment(appState)
        }
        .windowResizability(.contentSize)
        .defaultPosition(.topTrailing)
    }
}
