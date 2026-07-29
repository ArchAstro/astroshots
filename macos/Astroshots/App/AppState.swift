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
    var toast: String?
    /// AppKit owns the chromeless review window; SwiftUI and overlays request it
    /// through this callback without knowing window lifecycle details.
    var onReviewRequested: ((Shot) -> Void)?

    var overlayEnabled: Bool
    var autoDismiss: Bool
    var watchRootPaths: [String]

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
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        let isReviewUITest =
            environment["ASTROSHOTS_UI_TEST_REVIEW_PATH"] != nil
                || environment["ASTROSHOTS_UI_TEST_TRAY_PATH"] != nil
                || environment["ASTROSHOTS_UI_TEST_OVERLAY_PATH"] != nil
        #else
        let isReviewUITest = false
        #endif
        self.preferences = preferences
        self.overlayController = overlayController
        overlayEnabled = preferences.overlayEnabled
        autoDismiss = preferences.autoDismiss
        watchRootPaths = rootPaths

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
        // Scanning ~/archastro (or any large tree) on the main thread made the
        // app look like it crashed: no status item until the walk finished.
        if automaticallyStartsWatching && !isReviewUITest {
            Task { @MainActor in
                self.startWatching()
            }
        }
    }

    func startWatching() {
        guard !didStartWatching else { return }
        didStartWatching = true
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
        shots.removeAll {
            URL(fileURLWithPath: $0.path)
                .deletingLastPathComponent()
                .standardizedFileURL.path == directory
        }
        shots.append(contentsOf: replacement)
        shots.sort { $0.capturedAt > $1.capturedAt }

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

    /// Navigate within the same worktree, feature, and run while the takeover
    /// remains open. A missing run id intentionally groups only the feature.
    func reviewSibling(from shotID: String, delta: Int) -> Shot? {
        guard let current = shots.first(where: { $0.id == shotID }) else { return nil }
        let runShots = shots
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
        guard let index = runShots.firstIndex(where: { $0.id == shotID }) else { return nil }
        let next = index + delta
        guard runShots.indices.contains(next) else { return nil }
        return runShots[next]
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

    func stepDetail(_ delta: Int) {
        guard let current = selectedShotID,
              let index = shots.firstIndex(where: { $0.id == current })
        else { return }
        let next = index + delta
        guard shots.indices.contains(next) else { return }
        selectedShotID = shots[next].id
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

    func removeWatchRoot(_ path: String) {
        guard watchRootPaths.count > 1 else {
            showToast("Keep at least one watched folder")
            return
        }
        setWatchRoots(watchRootPaths.filter { $0 != path })
    }

    private func setWatchRoots(_ paths: [String]) {
        preferences.watchRootPaths = paths
        watchRootPaths = preferences.watchRootPaths
        shots.removeAll {
            !Preferences.isPath($0.worktreePath, coveredBy: watchRootPaths)
        }
        if let selectedShotID,
           !shots.contains(where: { $0.id == selectedShotID })
        {
            self.selectedShotID = shots.first?.id
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

    func chooseWatchRoots() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        panel.directoryURL = URL(
            fileURLWithPath: watchRootPaths.first ?? Preferences.defaultWatchRoot.path,
            isDirectory: true
        )
        panel.message = "Choose one or more folders to scan for .astroshot directories"
        panel.prompt = "Add"
        if panel.runModal() == .OK {
            addWatchRoots(panel.urls.map(\.path))
        }
    }

    func rescan() {
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
