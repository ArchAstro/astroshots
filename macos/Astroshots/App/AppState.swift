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
    /// True while a background full scan of the watch root is running.
    var isScanning = false
    var toast: String?

    var overlayEnabled: Bool
    var autoDismiss: Bool
    var watchRootPath: String

    private var toastTask: Task<Void, Never>?
    private let watcher: AstroshotWatcher
    private let overlayController = OverlayController()
    /// When true, skip overlay (e.g. initial scan).
    private var suppressOverlay = true
    private var didStartWatching = false

    var selectedShot: Shot? {
        guard let selectedShotID else { return shots.first }
        return shots.first { $0.id == selectedShotID } ?? shots.first
    }

    var isEmpty: Bool { shots.isEmpty }

    init() {
        let prefs = Preferences.shared
        let rootPath = prefs.watchRootPath
        overlayEnabled = prefs.overlayEnabled
        autoDismiss = prefs.autoDismiss
        watchRootPath = rootPath

        let watcher = AstroshotWatcher(
            configuration: .init(roots: [URL(fileURLWithPath: rootPath, isDirectory: true)])
        )
        self.watcher = watcher

        watcher.onShotsChanged = { [weak self] shots in
            self?.applyFullShotList(shots)
        }
        watcher.onNewShot = { [weak self] shot in
            self?.handleNewShot(shot)
        }
        watcher.onScanStateChanged = { [weak self] scanning in
            self?.isScanning = scanning
        }

        overlayController.onOpen = { [weak self] shot in
            self?.openDetail(shot)
        }

        // Defer filesystem work so MenuBarExtra can appear immediately.
        // Scanning ~/archastro (or any large tree) on the main thread made the
        // app look like it crashed: no status item until the walk finished.
        Task { @MainActor in
            self.startWatching()
        }
    }

    func startWatching() {
        guard !didStartWatching else { return }
        didStartWatching = true
        watcher.start()
        Task {
            // Allow overlays once the first scan has had a chance to complete,
            // or after a short grace if the tree is empty/fast.
            try? await Task.sleep(nanoseconds: 1_500_000_000)
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
        let seconds = Preferences.shared.autoDismissSeconds
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
        Preferences.shared.overlayEnabled = enabled
    }

    func setAutoDismiss(_ enabled: Bool) {
        autoDismiss = enabled
        Preferences.shared.autoDismiss = enabled
    }

    func setWatchRoot(_ path: String) {
        let expanded = (path as NSString).expandingTildeInPath
        watchRootPath = expanded
        Preferences.shared.watchRootPath = expanded
        suppressOverlay = true
        didStartWatching = true
        watcher.updateRoots([URL(fileURLWithPath: expanded, isDirectory: true)])
        Task {
            try? await Task.sleep(nanoseconds: 800_000_000)
            self.suppressOverlay = false
        }
        showToast("Watching \(displayPath(expanded))")
    }

    func chooseWatchRoot() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = URL(fileURLWithPath: watchRootPath, isDirectory: true)
        panel.message = "Choose a folder to scan for .astroshot directories"
        panel.prompt = "Watch"
        if panel.runModal() == .OK, let url = panel.url {
            setWatchRoot(url.path)
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
