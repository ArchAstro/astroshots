import AppKit
import Sparkle

/// Owns the shared `AppState`, menu-bar status item, and Sparkle updater for
/// the LSUIElement app.
///
/// Sparkle setup follows the standard Cocoa pattern: hold an
/// `SPUStandardUpdaterController`, wire “Check for Updates…” to
/// `checkForUpdates:`, and read feed/public-key from Info.plist.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private(set) var appState: AppState!
    private var statusItemController: StatusItemController!
    /// Standard Sparkle controller (scheduler + update UI). Nil under XCTest /
    /// UI-test launches so automated runs never hit the network.
    private(set) var updaterController: SPUStandardUpdaterController?
    /// Strong ref: `SPUStandardUpdaterController` keeps the delegate weakly.
    private var softwareUpdateDelegate: SoftwareUpdateDelegate?

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

        // Sparkle: start after the status item exists. Skip UI-test launches.
        let isUITestLaunch = ProcessInfo.processInfo.environment.keys.contains {
            $0.hasPrefix("ASTROSHOTS_UI_TEST_") || $0 == "ASTROSHOTS_FRICTION_CAPTURE_DIR"
        }
        if !isUITestLaunch {
            // startingUpdater: true begins scheduled background checks using
            // SUFeedURL / SUPublicEDKey from Info.plist. Delegate logs every
            // stage of the upgrade flow to Console (category SoftwareUpdate).
            let updateDelegate = SoftwareUpdateDelegate()
            softwareUpdateDelegate = updateDelegate
            let sparkle = SPUStandardUpdaterController(
                startingUpdater: true,
                updaterDelegate: updateDelegate,
                userDriverDelegate: nil
            )
            updaterController = sparkle
            updateDelegate.logStartup(updater: sparkle.updater)
            let updaterSettings = UpdaterSettingsModel(updater: sparkle.updater)
            state.updaterSettings = updaterSettings
            updateDelegate.onUpdateCycleChanged = { [weak updaterSettings] active in
                updaterSettings?.setUpdateCycleActive(active)
            }
            controller.attachUpdaterController(sparkle)

            #if DEBUG
            // Local verification only: ASTROSHOTS_CHECK_UPDATES_ON_LAUNCH=1 forces a
            // user-initiated check after the run loop is up.
            if ProcessInfo.processInfo.environment["ASTROSHOTS_CHECK_UPDATES_ON_LAUNCH"] == "1" {
                SoftwareUpdateLog.logger.info("debug forced check-on-launch")
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    NSApp.activate(ignoringOtherApps: true)
                    sparkle.checkForUpdates(nil)
                }
            }
            #endif
        }

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
