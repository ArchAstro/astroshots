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
        let env = ProcessInfo.processInfo.environment
        if let overlayPath = env["ASTROSHOTS_UI_TEST_OVERLAY_PATH"] {
            DispatchQueue.main.async {
                controller.openOverlay(atImagePath: overlayPath)
            }
        } else if let reviewPath = env["ASTROSHOTS_UI_TEST_REVIEW_PATH"] {
            DispatchQueue.main.async {
                controller.openReview(atImagePath: reviewPath)
            }
        } else if let captureDir = env["ASTROSHOTS_FRICTION_CAPTURE_DIR"],
                  let trayRoot = env["ASTROSHOTS_UI_TEST_TRAY_ROOT"]
        {
            // Friction-log agent runner: walk panes and dump tray PNGs.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                controller.captureFrictionLogJourney(
                    fixtureRoot: trayRoot,
                    outputDir: captureDir
                )
            }
        } else if let trayRoot = env["ASTROSHOTS_UI_TEST_TRAY_ROOT"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                controller.openTray(atRootPath: trayRoot)
            }
        } else if let detailPath = env["ASTROSHOTS_UI_TEST_DETAIL_PATH"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                controller.openTrayDetail(atImagePath: detailPath)
            }
        } else if let trayPath = env["ASTROSHOTS_UI_TEST_TRAY_PATH"] {
            // Give AppKit one pass to attach the status item to the menu bar.
            // Showing an NSPopover before its anchor has a window is a no-op.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                controller.openTray(atImagePath: trayPath)
            }
        } else {
            controller.presentFirstRunSetupIfNeeded()
        }
        #else
        controller.presentFirstRunSetupIfNeeded()
        #endif
    }

    func applicationWillTerminate(_ notification: Notification) {
        statusItemController?.teardown()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }
}
