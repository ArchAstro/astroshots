import AppKit

/// Owns the shared `AppState` and menu-bar status item for the LSUIElement app.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private(set) var appState: AppState!
    private var statusItemController: StatusItemController!

    func applicationDidFinishLaunching(_ notification: Notification) {
        #if DEBUG
        // The unit-test bundle is hosted inside Astroshots.app. Do not start a
        // second production AppState/watcher alongside the objects under test:
        // it would read and rewrite the user's real Application Support index.
        if NSClassFromString("XCTestCase") != nil {
            return
        }
        #endif

        // Keep the process alive as a menu-bar app with no Dock icon.
        NSApp.setActivationPolicy(.accessory)

        let state = AppState()
        appState = state
        let controller = StatusItemController(appState: state)
        statusItemController = controller
        controller.install()

        #if DEBUG
        if let overlayPath = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_OVERLAY_PATH"
        ] {
            DispatchQueue.main.async {
                controller.openOverlay(atImagePath: overlayPath)
            }
        } else if let reviewPath = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_REVIEW_PATH"
        ] {
            DispatchQueue.main.async {
                controller.openReview(atImagePath: reviewPath)
            }
        } else if let trayRoot = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_TRAY_ROOT"
        ] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                controller.openTray(atRootPath: trayRoot)
            }
        } else if let detailPath = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_DETAIL_PATH"
        ] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                controller.openTrayDetail(atImagePath: detailPath)
            }
        } else if let trayPath = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_TRAY_PATH"
        ] {
            // Give AppKit one pass to attach the status item to the menu bar.
            // Showing an NSPopover before its anchor has a window is a no-op.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
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
