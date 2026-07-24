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
    var toast: String?

    var overlayEnabled: Bool
    var autoDismiss: Bool
    var watchRootPath: String

    private var toastTask: Task<Void, Never>?
    private let watcher: AstroshotWatcher
    private let overlayController = OverlayController()
    /// When true, skip overlay (e.g. initial scan).
    private var suppressOverlay = true

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
            self?.shots = shots
            if self?.selectedShotID == nil {
                self?.selectedShotID = shots.first?.id
            }
        }
        watcher.onNewShot = { [weak self] shot in
            self?.handleNewShot(shot)
        }

        overlayController.onOpen = { [weak self] shot in
            self?.openDetail(shot)
        }

        watcher.start()
        // Allow overlays after the first scan settles.
        Task {
            try? await Task.sleep(nanoseconds: 800_000_000)
            self.suppressOverlay = false
        }
    }

    func handleNewShot(_ shot: Shot) {
        if selectedShotID == nil {
            selectedShotID = shot.id
        }
        unreadCount += 1
        guard !suppressOverlay, overlayEnabled else { return }
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
        // Bring tray forward via menu bar — user still needs the popover;
        // pinned window is optional. Post a notification apps can observe.
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
        // shots are newest-first; next older is +1
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
        watcher.updateRoots([URL(fileURLWithPath: expanded, isDirectory: true)])
        Task {
            try? await Task.sleep(nanoseconds: 500_000_000)
            self.suppressOverlay = false
        }
        showToast("Watching \(expanded)")
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
        showToast("Rescanned")
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

    /// Debug / demo: not used in production UI unless wired.
    func simulateShotForPreview() {
        // no-op in production
    }
}

extension Notification.Name {
    static let astroshotsOpenTray = Notification.Name("ai.archastro.astroshots.openTray")
}
