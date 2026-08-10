import AppKit
import Foundation
import Observation
import Sparkle

/// Thin Observable view-model over Sparkle’s `SPUUpdater` for SwiftUI settings.
///
/// Matches Sparkle’s documented settings pattern: seed `@State` (here:
/// `@Observable` properties) from the updater, and write back **only** when the
/// user toggles a control. Do not mirror these into a second defaults layer.
///
/// See https://sparkle-project.org/documentation/preferences-ui/
@MainActor
@Observable
final class UpdaterSettingsModel {
    private let updater: SPUUpdater

    var automaticallyChecksForUpdates: Bool {
        didSet {
            guard automaticallyChecksForUpdates != updater.automaticallyChecksForUpdates else {
                return
            }
            SoftwareUpdateLog.logger.info(
                "setting autoCheck=\(self.automaticallyChecksForUpdates, privacy: .public)"
            )
            updater.automaticallyChecksForUpdates = automaticallyChecksForUpdates
        }
    }

    var automaticallyDownloadsUpdates: Bool {
        didSet {
            guard automaticallyDownloadsUpdates != updater.automaticallyDownloadsUpdates else {
                return
            }
            SoftwareUpdateLog.logger.info(
                "setting autoDownload=\(self.automaticallyDownloadsUpdates, privacy: .public)"
            )
            updater.automaticallyDownloadsUpdates = automaticallyDownloadsUpdates
        }
    }

    init(updater: SPUUpdater) {
        self.updater = updater
        automaticallyChecksForUpdates = updater.automaticallyChecksForUpdates
        automaticallyDownloadsUpdates = updater.automaticallyDownloadsUpdates
    }

    var canCheckForUpdates: Bool {
        updater.canCheckForUpdates
    }

    func checkForUpdates() {
        // Menu-bar apps are LSUIElement; bring us forward so Sparkle’s
        // standard update window is not stranded behind other apps.
        SoftwareUpdateLog.logger.info(
            "userCheckForUpdates installed=\(SoftwareUpdateLog.installedVersionSummary(), privacy: .public)"
        )
        NSApp.activate(ignoringOtherApps: true)
        updater.checkForUpdates()
    }
}
