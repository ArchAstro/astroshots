import AppKit

/// Owns the shared `AppState` and menu-bar status item for the LSUIElement app.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private(set) var appState: AppState!
    private var statusItemController: StatusItemController!

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Keep the process alive as a menu-bar app with no Dock icon.
        NSApp.setActivationPolicy(.accessory)

        let state = AppState()
        appState = state
        let controller = StatusItemController(appState: state)
        statusItemController = controller
        controller.install()

        #if DEBUG
        if let reviewPath = ProcessInfo.processInfo.environment["ASTROSHOTS_UI_TEST_REVIEW_PATH"] {
            DispatchQueue.main.async {
                controller.openReview(atImagePath: reviewPath)
            }
        } else if let trayPath = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_TRAY_PATH"
        ] {
            DispatchQueue.main.async {
                controller.openTray(atImagePath: trayPath)
            }
        }
        #endif
    }

    func applicationWillTerminate(_ notification: Notification) {
        statusItemController?.teardown()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }
}
