import SwiftUI

/// Astroshots lives in the menu bar: stream of harness screenshots, desktop
/// overlay flash for new frames, detail drill-in. See docs/mocks/astroshots-menubar.html.
///
/// Status item interaction is AppKit (`StatusItemController`): left-click opens
/// the tray popover; right-click offers Open, Settings, version information,
/// and Quit. `MenuBarExtra` cannot host a native context menu, so it is not used.
@main
struct AstroshotsApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        // LSUIElement apps still need a Scene. Settings is never shown in the
        // Dock menu for accessory apps; the real UI is the status item + popover.
        Settings {
            EmptyView()
        }
    }
}
