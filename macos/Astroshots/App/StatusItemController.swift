import AppKit
import SwiftUI

/// AppKit status item: left-click toggles the tray popover; right-click shows
/// app actions, settings, version information, and Quit.
///
/// `MenuBarExtra` cannot attach a native context menu, so menu-bar interaction
/// is owned here instead of SwiftUI's menu-bar extra.
@MainActor
final class StatusItemController: NSObject {
    private let appState: AppState
    private var statusItem: NSStatusItem?
    private var popover: NSPopover?
    private var pinnedWindow: NSWindow?
    private var reviewWindowController: ReviewWindowController?
    private(set) var contextMenu: NSMenu?
    private var observationTask: Task<Void, Never>?

    init(appState: AppState) {
        self.appState = appState
        super.init()
    }

    func install() {
        let reviewWindowController = ReviewWindowController(appState: appState)
        self.reviewWindowController = reviewWindowController
        appState.onReviewRequested = { [weak reviewWindowController] shot in
            reviewWindowController?.open(shot)
        }

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            button.imagePosition = .imageLeft
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
            button.action = #selector(statusItemClicked(_:))
            button.target = self
            button.toolTip = "Astroshots"
        }
        statusItem = item

        let popover = NSPopover()
        popover.contentSize = NSSize(width: Theme.trayWidth, height: Theme.trayHeight)
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        let isTrayUITest =
            environment["ASTROSHOTS_UI_TEST_TRAY_PATH"] != nil
            || environment["ASTROSHOTS_UI_TEST_DETAIL_PATH"] != nil
            || environment["ASTROSHOTS_UI_TEST_TRAY_ROOT"] != nil
        popover.behavior = isTrayUITest ? .applicationDefined : .transient
        #else
        popover.behavior = .transient
        #endif
        popover.animates = true
        popover.delegate = self
        self.popover = popover
        reloadPopoverContent()

        let menu = NSMenu()
        let openItem = NSMenuItem(
            title: "Open Astroshots",
            action: #selector(openFromMenu(_:)),
            keyEquivalent: ""
        )
        openItem.target = self
        menu.addItem(openItem)

        let settingsItem = NSMenuItem(
            title: "Settings…",
            action: #selector(openSettingsFromMenu(_:)),
            keyEquivalent: ","
        )
        settingsItem.target = self
        menu.addItem(settingsItem)

        menu.addItem(.separator())

        let versionItem = NSMenuItem(
            title: Self.versionMenuTitle(),
            action: nil,
            keyEquivalent: ""
        )
        versionItem.isEnabled = false
        menu.addItem(versionItem)

        menu.addItem(.separator())
        let quitItem = NSMenuItem(
            title: "Quit Astroshots",
            action: #selector(quit(_:)),
            keyEquivalent: "q"
        )
        quitItem.target = self
        menu.addItem(quitItem)
        contextMenu = menu

        refreshButton()
        startObservingState()
    }

    func teardown() {
        observationTask?.cancel()
        observationTask = nil
        closePopover()
        closePinnedWindow()
        reviewWindowController?.close()
        reviewWindowController = nil
        appState.onReviewRequested = nil
        if let item = statusItem {
            NSStatusBar.system.removeStatusItem(item)
        }
        statusItem = nil
        popover = nil
        contextMenu = nil
    }

    /// Test-only launch bridge for exercising the real review window without
    /// automating the system menu bar.
    ///
    /// Loads every shot under the same worktree root so full-screen left/right
    /// paging has real siblings to walk through.
    func openReview(atImagePath path: String) {
        guard let shot = loadTestShots(atImagePath: path) else { return }
        reviewWindowController?.open(shot)
    }

    /// Seeds and opens the real stream popover so UI automation can prove the
    /// same thumbnail interaction a reviewer uses from the menu-bar app.
    ///
    /// Loads every sibling under the worktree so detail paging has a real set.
    func openTray(atImagePath path: String) {
        guard loadTestShots(atImagePath: path) != nil else { return }
        showPopover()
    }

    /// Seeds siblings and opens the tray already drilled into detail for `path`.
    func openTrayDetail(atImagePath path: String) {
        guard let shot = loadTestShots(atImagePath: path) else { return }
        appState.selectShot(shot)
        showPopover()
    }

    /// Seeds a complete multi-worktree stream for grouped-stream UI proofs.
    func openTray(atRootPath path: String) {
        let reader = AstroshotWatcher(
            configuration: .init(
                roots: [URL(fileURLWithPath: path, isDirectory: true)]
            )
        )
        let shots = reader.scanAll()
        guard !shots.isEmpty else { return }
        for shot in shots.reversed() {
            appState.handleNewShot(shot)
        }
        showPopover()
    }

    #if DEBUG
    /// Shows the production overlay without auto-dismiss so UI automation can
    /// prove the entire card opens review.
    func openOverlay(atImagePath path: String) {
        guard let shot = loadTestShot(atImagePath: path) else { return }
        appState.showOverlayForTesting(shot)
    }
    #endif

    private func loadTestShot(atImagePath path: String) -> Shot? {
        let imageURL = URL(fileURLWithPath: path)
        let root = imageURL
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let reader = AstroshotWatcher(configuration: .init(roots: [root]))
        guard let shot = reader.makeShot(at: path) else { return nil }
        appState.handleNewShot(shot)
        return shot
    }

    /// Loads the targeted frame plus every sibling under its worktree so
    /// review paging and multi-frame stream tests share one seed path.
    private func loadTestShots(atImagePath path: String) -> Shot? {
        let imageURL = URL(fileURLWithPath: path)
        let standardizedPath = imageURL.standardizedFileURL.path
        let root = imageURL
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let reader = AstroshotWatcher(configuration: .init(roots: [root]))
        let discovered = reader.scanAll()
        if discovered.isEmpty {
            return loadTestShot(atImagePath: path)
        }
        // Oldest first so handleNewShot's insert-at-front ends newest-first.
        for shot in discovered.sorted(by: { $0.capturedAt < $1.capturedAt }) {
            appState.handleNewShot(shot)
        }
        return discovered.first {
            $0.path == path
                || $0.path == standardizedPath
                || URL(fileURLWithPath: $0.path).standardizedFileURL.path
                    == standardizedPath
        } ?? reader.makeShot(at: path)
    }

    // MARK: - Button / menu

    @objc private func statusItemClicked(_ sender: Any?) {
        guard let event = NSApp.currentEvent else {
            togglePopover()
            return
        }
        switch event.type {
        case .rightMouseUp:
            showContextMenu()
        default:
            togglePopover()
        }
    }

    @objc private func openFromMenu(_ sender: Any?) {
        showPopover()
    }

    @objc private func openSettingsFromMenu(_ sender: Any?) {
        appState.pane = .settings
        showPopover()
    }

    /// First launch: open the tray, then ask which folders to watch.
    ///
    /// The status item needs a run-loop turn before its button has a window;
    /// otherwise the popover is a no-op.
    func presentFirstRunSetupIfNeeded() {
        guard appState.needsWatchRootSetup else { return }
        appState.pane = .stream
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            guard let self else { return }
            self.showPopover()
            // Let the popover present before the modal folder panel steals focus.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
                guard let self, self.appState.needsWatchRootSetup else { return }
                _ = self.appState.chooseWatchRoots(forSetup: true)
            }
        }
    }

    @objc private func quit(_ sender: Any?) {
        NSApp.terminate(nil)
    }

    static func versionMenuTitle(
        infoDictionary: [String: Any] = Bundle.main.infoDictionary ?? [:]
    ) -> String {
        let version = infoDictionary["CFBundleShortVersionString"] as? String
        let build = infoDictionary["CFBundleVersion"] as? String

        switch (version?.nonEmpty, build?.nonEmpty) {
        case let (version?, build?):
            return "Version \(version) (\(build))"
        case let (version?, nil):
            return "Version \(version)"
        case let (nil, build?):
            return "Build \(build)"
        case (nil, nil):
            return "Version unavailable"
        }
    }

    private func showContextMenu() {
        guard let button = statusItem?.button, let menu = contextMenu else { return }
        closePopover()
        // Anchor under the status item without swapping `statusItem.menu`
        // (that path makes left-click open the menu too).
        let point = NSPoint(x: 0, y: button.bounds.height + 2)
        menu.popUp(positioning: nil, at: point, in: button)
    }

    // MARK: - Popover

    private func togglePopover() {
        if popover?.isShown == true {
            closePopover()
        } else {
            showPopover()
        }
    }

    private func showPopover() {
        guard let button = statusItem?.button, let popover else { return }
        reloadPopoverContent()
        NSApp.activate(ignoringOtherApps: true)
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        button.highlight(true)
    }

    private func closePopover() {
        popover?.performClose(nil)
        statusItem?.button?.highlight(false)
    }

    private func reloadPopoverContent() {
        guard let popover else { return }
        let root = TrayView(isPinned: false)
            .environment(appState)
            .environment(\.trayChrome, trayChromeForPopover())
            .environment(\.reviewChrome, reviewChrome())
        popover.contentViewController = NSHostingController(rootView: root)
    }

    private func trayChromeForPopover() -> TrayChrome {
        TrayChrome(
            close: { [weak self] in
                self?.closePopover()
            },
            openPinned: { [weak self] in
                self?.openPinnedWindow()
            }
        )
    }

    // MARK: - Pinned window

    private func openPinnedWindow() {
        closePopover()
        if pinnedWindow == nil {
            let root = TrayView(isPinned: true)
                .environment(appState)
                .environment(\.trayChrome, trayChromeForPinned())
                .environment(\.reviewChrome, reviewChrome())
            let hosting = NSHostingController(rootView: root)
            let window = NSPanel(
                contentRect: NSRect(x: 0, y: 0, width: Theme.trayWidth, height: Theme.trayHeight),
                styleMask: [.titled, .closable, .fullSizeContentView, .nonactivatingPanel],
                backing: .buffered,
                defer: false
            )
            window.title = "Astroshots"
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.isFloatingPanel = true
            window.level = .floating
            window.hidesOnDeactivate = false
            window.isReleasedWhenClosed = false
            window.contentViewController = hosting
            window.setContentSize(NSSize(width: Theme.trayWidth, height: Theme.trayHeight))
            window.center()
            pinnedWindow = window
        }
        pinnedWindow?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func closePinnedWindow() {
        pinnedWindow?.orderOut(nil)
    }

    private func trayChromeForPinned() -> TrayChrome {
        TrayChrome(
            close: { [weak self] in
                self?.closePinnedWindow()
            },
            openPinned: { [weak self] in
                self?.openPinnedWindow()
            }
        )
    }

    private func reviewChrome() -> ReviewChrome {
        ReviewChrome(
            open: { [weak self] shot in
                self?.closePopover()
                self?.reviewWindowController?.open(shot)
            }
        )
    }

    // MARK: - Status appearance

    private func startObservingState() {
        observationTask?.cancel()
        observationTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                self.refreshButton()
                // Re-arm observation when any tracked property changes.
                await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                    withObservationTracking {
                        _ = self.appState.isEmpty
                        _ = self.appState.unreadCount
                        _ = self.appState.isScanning
                    } onChange: {
                        continuation.resume()
                    }
                }
            }
        }
    }

    private func refreshButton() {
        guard let button = statusItem?.button else { return }
        let image = NSImage(named: "AstroshotsMark")
        image?.isTemplate = true
        image?.accessibilityDescription = "Astroshots"
        button.image = image
        if appState.unreadCount > 0 {
            button.title = " \(appState.unreadCount)"
        } else {
            button.title = ""
        }
    }
}

private extension String {
    var nonEmpty: String? {
        isEmpty ? nil : self
    }
}

extension StatusItemController: NSPopoverDelegate {
    nonisolated func popoverDidClose(_ notification: Notification) {
        Task { @MainActor in
            self.statusItem?.button?.highlight(false)
        }
    }
}

// MARK: - Tray chrome actions (popover / pinned window)

/// Closures for tray chrome that is hosted outside a SwiftUI Scene
/// (NSPopover / NSPanel), where `dismiss` / `openWindow` are unavailable.
struct TrayChrome: Sendable {
    var close: @MainActor @Sendable () -> Void = {}
    var openPinned: @MainActor @Sendable () -> Void = {}
}

private enum TrayChromeKey: EnvironmentKey {
    static let defaultValue = TrayChrome()
}

extension EnvironmentValues {
    var trayChrome: TrayChrome {
        get { self[TrayChromeKey.self] }
        set { self[TrayChromeKey.self] = newValue }
    }
}
