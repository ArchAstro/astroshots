import Foundation
import OSLog
import Sparkle

/// `SPUUpdaterDelegate` that records every stage of the upgrade flow to the
/// unified system log (`SoftwareUpdateLog`). Keep this object alive for the
/// lifetime of `SPUStandardUpdaterController` (the controller holds it weakly).
final class SoftwareUpdateDelegate: NSObject, SPUUpdaterDelegate {
    private let log = SoftwareUpdateLog.logger

    /// One-shot boot summary so a release test has a clear start line.
    func logStartup(updater: SPUUpdater) {
        let installed = SoftwareUpdateLog.installedVersionSummary()
        let feed = SoftwareUpdateLog.feedURLString()
        let autoCheck = updater.automaticallyChecksForUpdates
        let autoDownload = updater.automaticallyDownloadsUpdates
        let interval = updater.updateCheckInterval
        log.info(
            "startup installed=\(installed, privacy: .public) feed=\(feed, privacy: .public) autoCheck=\(autoCheck, privacy: .public) autoDownload=\(autoDownload, privacy: .public) checkIntervalSec=\(interval, privacy: .public)"
        )
    }

    // MARK: - Check lifecycle

    func updater(
        _ updater: SPUUpdater,
        mayPerform updateCheck: SPUUpdateCheck,
        error: NSErrorPointer
    ) -> Bool {
        let type = SoftwareUpdateLog.describe(updateCheck)
        let installed = SoftwareUpdateLog.installedVersionSummary()
        log.info("mayPerformCheck type=\(type, privacy: .public) installed=\(installed, privacy: .public)")
        return true
    }

    func updater(
        _ updater: SPUUpdater,
        didFinishLoading appcast: SUAppcast
    ) {
        let count = appcast.items.count
        let versions = appcast.items.prefix(5).map {
            "\($0.displayVersionString)[\($0.versionString)]"
        }.joined(separator: ", ")
        let more = count > 5 ? " …+\(count - 5)" : ""
        let head = versions + more
        let feed = SoftwareUpdateLog.feedURLString()
        log.info(
            "appcastLoaded itemCount=\(count, privacy: .public) head=\(head, privacy: .public) feed=\(feed, privacy: .public)"
        )
    }

    func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
        let update = SoftwareUpdateLog.describe(item)
        let from = SoftwareUpdateLog.installedVersionSummary()
        log.info("foundUpdate \(update, privacy: .public) from=\(from, privacy: .public)")
    }

    func updaterDidNotFindUpdate(_ updater: SPUUpdater, error: Error) {
        let ns = error as NSError
        let detail = SoftwareUpdateLog.describe(error)
        let keys = Array(ns.userInfo.keys).map(String.init(describing:)).sorted().joined(separator: ",")
        log.info("noUpdate \(detail, privacy: .public) userInfoKeys=\(keys, privacy: .public)")
    }

    func updater(
        _ updater: SPUUpdater,
        userDidMake choice: SPUUserUpdateChoice,
        forUpdate updateItem: SUAppcastItem,
        state: SPUUserUpdateState
    ) {
        let choiceText = SoftwareUpdateLog.describe(choice)
        let update = SoftwareUpdateLog.describe(updateItem)
        log.info(
            "userChoice \(choiceText, privacy: .public) update=\(update, privacy: .public) userInitiated=\(state.userInitiated, privacy: .public)"
        )
    }

    // MARK: - Download / extract / install

    func updater(
        _ updater: SPUUpdater,
        willDownloadUpdate item: SUAppcastItem,
        with request: NSMutableURLRequest
    ) {
        let update = SoftwareUpdateLog.describe(item)
        let url = request.url?.absoluteString ?? "(nil)"
        log.info("willDownload \(update, privacy: .public) request=\(url, privacy: .public)")
    }

    func updater(_ updater: SPUUpdater, didDownloadUpdate item: SUAppcastItem) {
        let update = SoftwareUpdateLog.describe(item)
        log.info("didDownload \(update, privacy: .public)")
    }

    func updater(
        _ updater: SPUUpdater,
        failedToDownloadUpdate item: SUAppcastItem,
        error: Error
    ) {
        let update = SoftwareUpdateLog.describe(item)
        let detail = SoftwareUpdateLog.describe(error)
        log.error("downloadFailed \(update, privacy: .public) error=\(detail, privacy: .public)")
    }

    func userDidCancelDownload(_ updater: SPUUpdater) {
        let installed = SoftwareUpdateLog.installedVersionSummary()
        log.info("downloadCancelled installed=\(installed, privacy: .public)")
    }

    func updater(_ updater: SPUUpdater, willExtractUpdate item: SUAppcastItem) {
        let update = SoftwareUpdateLog.describe(item)
        log.info("willExtract \(update, privacy: .public)")
    }

    func updater(_ updater: SPUUpdater, didExtractUpdate item: SUAppcastItem) {
        let update = SoftwareUpdateLog.describe(item)
        log.info("didExtract \(update, privacy: .public)")
    }

    func updater(_ updater: SPUUpdater, willInstallUpdate item: SUAppcastItem) {
        let update = SoftwareUpdateLog.describe(item)
        let replacing = SoftwareUpdateLog.installedVersionSummary()
        log.notice("willInstall \(update, privacy: .public) replacing=\(replacing, privacy: .public)")
    }

    func updater(
        _ updater: SPUUpdater,
        willInstallUpdateOnQuit item: SUAppcastItem,
        immediateInstallationBlock immediateInstallHandler: @escaping () -> Void
    ) -> Bool {
        let update = SoftwareUpdateLog.describe(item)
        log.info("willInstallOnQuit \(update, privacy: .public) (install deferred until quit)")
        // false = do not take over installation; let Sparkle’s default on-quit path run.
        return false
    }

    func updaterWillRelaunchApplication(_ updater: SPUUpdater) {
        let installed = SoftwareUpdateLog.installedVersionSummary()
        log.notice("willRelaunch installed=\(installed, privacy: .public)")
    }

    // MARK: - Scheduling / cycle end

    func updater(_ updater: SPUUpdater, willScheduleUpdateCheckAfterDelay delay: TimeInterval) {
        log.info(
            "scheduleNextCheck delaySec=\(delay, privacy: .public) autoCheck=\(updater.automaticallyChecksForUpdates, privacy: .public)"
        )
    }

    func updaterWillNotScheduleUpdateCheck(_ updater: SPUUpdater) {
        log.info(
            "noSchedule autoCheck=\(updater.automaticallyChecksForUpdates, privacy: .public) (automatic checks disabled or updater idle)"
        )
    }

    func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        let detail = SoftwareUpdateLog.describe(error)
        log.error("aborted \(detail, privacy: .public)")
    }

    func updater(
        _ updater: SPUUpdater,
        didFinishUpdateCycleFor updateCheck: SPUUpdateCheck,
        error: (any Error)?
    ) {
        let type = SoftwareUpdateLog.describe(updateCheck)
        if let error {
            let detail = SoftwareUpdateLog.describe(error)
            log.error("cycleFinished type=\(type, privacy: .public) error=\(detail, privacy: .public)")
        } else {
            log.info("cycleFinished type=\(type, privacy: .public) ok")
        }
    }
}
