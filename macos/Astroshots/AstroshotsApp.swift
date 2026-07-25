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
            // Always show a camera glyph so the status item is visible even
            // while the background scan is still walking the watch root.
            if appState.isScanning && appState.isEmpty {
                Image(systemName: "camera")
            } else {
                Image(systemName: appState.isEmpty ? "camera" : "camera.fill")
            }
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
