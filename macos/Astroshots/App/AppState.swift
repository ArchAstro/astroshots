import AppKit
import Foundation
import Observation
import SwiftUI

enum TrayPane: Equatable {
    case stream
    case detail
    case settings
}

@MainActor
@Observable
final class AppState {
    var shots: [Shot] = []
    var selectedShotID: String?
    var pane: TrayPane = .stream
    var unreadCount = 0
    var isWatching = true
    /// True while a background full scan of the watched roots is running.
    var isScanning = false
    var isMarkingSeen = false
    var toast: String?
    /// AppKit owns the chromeless review window; SwiftUI and overlays request it
    /// through this callback without knowing window lifecycle details.
    var onReviewRequested: ((Shot) -> Void)?

    var overlayEnabled: Bool
    var autoDismiss: Bool
    var watchRootPaths: [String]
    /// True when no watch roots are configured (empty stream / setup CTA).
    var needsWatchRootSetup: Bool
    /// True while first-run setup is incomplete — launch should present the
    /// startup sequence (tray + folder panel). Driven by an explicit preference
    /// flag, not merely empty roots.
    var shouldPresentFirstRunStartup: Bool

    private var toastTask: Task<Void, Never>?
    private let watcher: AstroshotWatcher
    private let preferences: Preferences
    private let reviewStore = ReviewStore()
    private let overlayController: OverlayController
    /// When true, skip overlay (e.g. initial scan).
    private var suppressOverlay = true
    private var didStartWatching = false

    var selectedShot: Shot? {
        guard let selectedShotID else { return shots.first }
        return shots.first { $0.id == selectedShotID } ?? shots.first
    }

    var isEmpty: Bool { shots.isEmpty }

    init(
        preferences: Preferences = .shared,
        watcher: AstroshotWatcher? = nil,
        overlayController: OverlayController = OverlayController(),
        automaticallyStartsWatching: Bool = true
    ) {
        let rootPaths = preferences.watchRootPaths
        let needsSetup = rootPaths.isEmpty
        let presentFirstRun = preferences.shouldPresentFirstRunStartup
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        let isReviewUITest =
            environment["ASTROSHOTS_UI_TEST_REVIEW_PATH"] != nil
                || environment["ASTROSHOTS_UI_TEST_TRAY_PATH"] != nil
                || environment["ASTROSHOTS_UI_TEST_DETAIL_PATH"] != nil
                || environment["ASTROSHOTS_UI_TEST_TRAY_ROOT"] != nil
                || environment["ASTROSHOTS_UI_TEST_OVERLAY_PATH"] != nil
        #else
        let isReviewUITest = false
        #endif
        self.preferences = preferences
        self.overlayController = overlayController
        overlayEnabled = preferences.overlayEnabled
        autoDismiss = preferences.autoDismiss
        watchRootPaths = rootPaths
        needsWatchRootSetup = needsSetup
        shouldPresentFirstRunStartup = presentFirstRun
        isWatching = !needsSetup

        self.watcher = watcher ?? AstroshotWatcher(
            configuration: .init(
                roots: rootPaths.map {
                    URL(fileURLWithPath: $0, isDirectory: true)
                }
            )
        )

        self.watcher.onShotsChanged = { [weak self] shots in
            self?.applyFullShotList(shots)
        }
        self.watcher.onFeatureShotsChanged = { [weak self] directoryPath, shots in
            self?.applyFeatureShotList(directoryPath: directoryPath, shots: shots)
        }
        self.watcher.onNewShot = { [weak self] shot in
            self?.handleNewShot(shot)
        }
        self.watcher.onScanStateChanged = { [weak self] scanning in
            self?.isScanning = scanning
        }

        overlayController.onOpen = { [weak self] shot in
            self?.requestReview(shot)
        }

        // Defer filesystem work so the status item can appear immediately.
        // Do not start watching until the user has chosen folders (first run).
        if automaticallyStartsWatching && !isReviewUITest && !needsSetup {
            Task { @MainActor in
                self.startWatching()
            }
        }
    }

    func startWatching() {
        guard !didStartWatching else { return }
        guard !watchRootPaths.isEmpty else { return }
        didStartWatching = true
        isWatching = true
        watcher.start()
        Task {
            // Warm cache can populate the tray in tens of ms; wait a short
            // grace so the warm emit does not flash overlays for old frames.
            try? await Task.sleep(nanoseconds: 1_200_000_000)
            self.suppressOverlay = false
        }
    }

    private func applyFullShotList(_ shots: [Shot]) {
        self.shots = shots
        if selectedShotID == nil {
            selectedShotID = shots.first?.id
        } else if let id = selectedShotID, !shots.contains(where: { $0.id == id }) {
            selectedShotID = shots.first?.id
        }
    }

    private func applyFeatureShotList(directoryPath: String, shots replacement: [Shot]) {
        let directory = URL(fileURLWithPath: directoryPath, isDirectory: true).standardizedFileURL.path
        var replacements = Dictionary(
            uniqueKeysWithValues: replacement.map { ($0.path, $0) }
        )
        var updated: [Shot] = []
        for shot in shots {
            let shotDirectory = URL(fileURLWithPath: shot.path)
                .deletingLastPathComponent()
                .standardizedFileURL.path
            if shotDirectory == directory {
                if let refreshed = replacements.removeValue(forKey: shot.path) {
                    updated.append(refreshed)
                }
            } else {
                updated.append(shot)
            }
        }
        let newlyDiscovered = replacements.values.sorted {
            $0.capturedAt > $1.capturedAt
        }
        shots = newlyDiscovered + updated

        if let selectedShotID, !shots.contains(where: { $0.id == selectedShotID }) {
            self.selectedShotID = shots.first?.id
        } else if selectedShotID == nil {
            selectedShotID = shots.first?.id
        }
    }

    /// Insert or replace a shot without waiting for a full rescan.
    func handleNewShot(_ shot: Shot) {
        let isNewPath: Bool
        if let index = shots.firstIndex(where: { $0.path == shot.path }) {
            shots[index] = shot
            let updated = shots.remove(at: index)
            shots.insert(updated, at: 0)
            isNewPath = false
        } else {
            shots.insert(shot, at: 0)
            unreadCount += 1
            isNewPath = true
        }
        if selectedShotID == nil {
            selectedShotID = shot.id
        }

        guard isNewPath, !suppressOverlay, overlayEnabled else { return }
        let seconds = preferences.autoDismissSeconds
        overlayController.show(
            shot: shot,
            autoDismiss: autoDismiss,
            duration: seconds
        )
    }

    func markTrayOpened() {
        unreadCount = 0
    }

    func selectShot(_ shot: Shot) {
        selectedShotID = shot.id
        pane = .detail
    }

    func openDetail(_ shot: Shot) {
        selectedShotID = shot.id
        pane = .detail
        NotificationCenter.default.post(name: .astroshotsOpenTray, object: nil)
    }

    func requestReview(_ shot: Shot) {
        selectedShotID = shot.id
        onReviewRequested?(shot)
    }

    #if DEBUG
    func showOverlayForTesting(_ shot: Shot) {
        overlayController.show(shot: shot, autoDismiss: false, duration: 0)
    }
    #endif

    /// Shots that belong with `shotID` in full-screen review, ordered oldest → newest.
    /// Same worktree + feature + run; a missing run id groups the whole feature.
    func reviewSiblings(from shotID: String) -> [Shot] {
        guard let current = shots.first(where: { $0.id == shotID }) else { return [] }
        return shots
            .filter {
                $0.worktreePath == current.worktreePath
                    && $0.feature == current.feature
                    && $0.runID == current.runID
            }
            .sorted { lhs, rhs in
                if let left = lhs.sequence, let right = rhs.sequence, left != right {
                    return left < right
                }
                return lhs.capturedAt < rhs.capturedAt
            }
    }

    /// Navigate within the same worktree, feature, and run while the takeover
    /// remains open. A missing run id intentionally groups only the feature.
    ///
    /// `delta` is relative to oldest→newest order: negative is older, positive is newer.
    func reviewSibling(from shotID: String, delta: Int) -> Shot? {
        let runShots = reviewSiblings(from: shotID)
        guard let index = runShots.firstIndex(where: { $0.id == shotID }) else { return nil }
        let next = index + delta
        guard runShots.indices.contains(next) else { return nil }
        return runShots[next]
    }

    /// 1-based position among review siblings, or `nil` when the shot is unknown.
    func reviewPosition(for shotID: String) -> (index: Int, count: Int)? {
        let runShots = reviewSiblings(from: shotID)
        guard let index = runShots.firstIndex(where: { $0.id == shotID }) else { return nil }
        return (index + 1, runShots.count)
    }

    /// 1-based position in the tray stream (newest-first), or `nil` when empty.
    func detailPosition(for shotID: String) -> (index: Int, count: Int)? {
        guard let index = shots.firstIndex(where: { $0.id == shotID }) else { return nil }
        return (index + 1, shots.count)
    }

    func canStepDetail(_ delta: Int) -> Bool {
        guard let current = selectedShotID,
              let index = shots.firstIndex(where: { $0.id == current })
        else { return false }
        return shots.indices.contains(index + delta)
    }

    func addReviewComment(_ body: String, to shot: Shot) async throws {
        let snapshot = try await reviewStore.addComment(body, to: shot)
        applyReview(snapshot, to: shot.id)
        showToast("Comment added")
    }

    func markSeen(
        note: String?,
        for shot: Shot
    ) async throws {
        let snapshot = try await reviewStore.markSeen(note: note, for: shot)
        applyReview(snapshot, to: shot.id)
        showToast("Seen")
    }

    func markAllSeen(_ candidates: [Shot]) async {
        guard !isMarkingSeen else { return }
        let candidateIDs = Set(candidates.map(\.id))
        let unseen = shots.filter {
            candidateIDs.contains($0.id)
                && ($0.review?.state ?? .pending) != .seen
        }
        guard !unseen.isEmpty else {
            showToast("Already seen")
            return
        }

        isMarkingSeen = true
        defer { isMarkingSeen = false }

        var markedCount = 0
        var failureCount = 0
        for shot in unseen {
            do {
                let snapshot = try await reviewStore.markSeen(
                    note: nil,
                    for: shot
                )
                applyReview(snapshot, to: shot.id)
                markedCount += 1
            } catch {
                failureCount += 1
            }
        }

        if failureCount > 0 {
            showToast(
                markedCount == 0
                    ? "Couldn’t mark frames as seen"
                    : "Marked \(markedCount) seen; \(failureCount) failed"
            )
            return
        }

        showToast(
            markedCount == 1
                ? "Marked 1 frame seen"
                : "Marked \(markedCount) frames seen"
        )
    }

    private func applyReview(_ snapshot: ReviewSnapshot, to shotID: String) {
        guard let index = shots.firstIndex(where: { $0.id == shotID }) else { return }
        shots[index].review = snapshot
    }

    func backToStream() {
        pane = .stream
    }

    func openSettings() {
        pane = pane == .settings ? .stream : .settings
    }

    /// Page the tray detail view through the stream.
    ///
    /// Stream order is newest-first, so positive `delta` moves older and
    /// negative `delta` moves newer — matching left = older, right = newer.
    func stepDetail(_ delta: Int) {
        guard canStepDetail(delta),
              let current = selectedShotID,
              let index = shots.firstIndex(where: { $0.id == current })
        else { return }
        selectedShotID = shots[index + delta].id
    }

    func setOverlayEnabled(_ enabled: Bool) {
        overlayEnabled = enabled
        preferences.overlayEnabled = enabled
    }

    func setAutoDismiss(_ enabled: Bool) {
        autoDismiss = enabled
        preferences.autoDismiss = enabled
    }

    func addWatchRoots(_ paths: [String]) {
        setWatchRoots(watchRootPaths + paths)
    }

    /// Apply roots chosen during first-run (or empty-state) setup and mark
    /// first-run complete when the list is non-empty.
    func finishWatchRootSetup(_ paths: [String]) {
        setWatchRoots(paths, completingFirstRun: true)
    }

    func removeWatchRoot(_ path: String) {
        guard watchRootPaths.count > 1 else {
            showToast("Keep at least one watched folder")
            return
        }
        setWatchRoots(watchRootPaths.filter { $0 != path })
    }

    private func setWatchRoots(_ paths: [String], completingFirstRun: Bool = false) {
        preferences.watchRootPaths = paths
        watchRootPaths = preferences.watchRootPaths
        needsWatchRootSetup = watchRootPaths.isEmpty
        isWatching = !watchRootPaths.isEmpty

        if completingFirstRun, !watchRootPaths.isEmpty {
            preferences.markFirstRunSetupComplete()
            shouldPresentFirstRunStartup = false
        } else {
            shouldPresentFirstRunStartup = preferences.shouldPresentFirstRunStartup
        }

        shots.removeAll {
            !Preferences.isPath($0.worktreePath, coveredBy: watchRootPaths)
        }
        if let selectedShotID,
           !shots.contains(where: { $0.id == selectedShotID })
        {
            self.selectedShotID = shots.first?.id
        }
        guard !watchRootPaths.isEmpty else {
            didStartWatching = false
            watcher.stop()
            return
        }
        suppressOverlay = true
        didStartWatching = true
        watcher.updateRoots(
            watchRootPaths.map {
                URL(fileURLWithPath: $0, isDirectory: true)
            }
        )
        Task {
            try? await Task.sleep(nanoseconds: 800_000_000)
            self.suppressOverlay = false
        }
        showToast(
            watchRootPaths.count == 1
                ? "Watching \(displayPath(watchRootPaths[0]))"
                : "Watching \(watchRootPaths.count) folders"
        )
    }

    /// Opens a multi-select folder panel.
    ///
    /// - Parameter forSetup: When true (first launch or empty configuration),
    ///   replaces the root list and uses setup copy. Otherwise appends.
    ///   Completing setup with a non-empty selection marks first-run done.
    @discardableResult
    func chooseWatchRoots(forSetup: Bool = false) -> Bool {
        let isSetup = forSetup || needsWatchRootSetup || shouldPresentFirstRunStartup
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        panel.directoryURL = URL(
            fileURLWithPath: watchRootPaths.first ?? Preferences.folderPickerStartURL.path,
            isDirectory: true
        )
        panel.message = isSetup
            ? "Choose one or more folders Astroshots should watch for .astroshot screenshots."
            : "Choose one or more folders to scan for .astroshot directories"
        panel.prompt = isSetup ? "Start watching" : "Add"
        guard panel.runModal() == .OK else { return false }
        let paths = panel.urls.map(\.path)
        guard !paths.isEmpty else { return false }
        if isSetup {
            finishWatchRootSetup(paths)
        } else {
            addWatchRoots(paths)
        }
        return true
    }

    func rescan() {
        guard !needsWatchRootSetup, !watchRootPaths.isEmpty else {
            showToast("Choose folders to watch first")
            return
        }
        watcher.rescan()
        showToast("Scanning…")
    }

    func showToast(_ message: String) {
        toastTask?.cancel()
        toast = message
        toastTask = Task {
            try? await Task.sleep(nanoseconds: 1_600_000_000)
            guard !Task.isCancelled else { return }
            toast = nil
        }
    }

    private func displayPath(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path.hasPrefix(home) {
            return "~" + path.dropFirst(home.count)
        }
        return path
    }
}

extension Notification.Name {
    static let astroshotsOpenTray = Notification.Name("ai.archastro.astroshots.openTray")
}
