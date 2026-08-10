import AppKit
import Sparkle
import SwiftUI

/// AppKit status item: left-click toggles the tray popover; right-click shows
/// app actions, settings, Check for Updates…, version, and Quit.
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
    private var checkForUpdatesMenuItem: NSMenuItem?
    private var observationTask: Task<Void, Never>?
    /// Sparkle controller; menu item target/action point at this when set.
    private var updaterController: SPUStandardUpdaterController?

    init(appState: AppState) {
        self.appState = appState
        super.init()
    }

    /// Idiomatic Sparkle menu wiring: target the standard controller and use
    /// `checkForUpdates:`. The controller’s `validateMenuItem:` disables the
    /// item while a check is already running (`canCheckForUpdates`).
    func attachUpdaterController(_ controller: SPUStandardUpdaterController) {
        updaterController = controller
        if let item = checkForUpdatesMenuItem {
            item.target = controller
            item.action = #selector(SPUStandardUpdaterController.checkForUpdates(_:))
        }
    }

    func install() {
        let reviewWindowController = ReviewWindowController(appState: appState)
        self.reviewWindowController = reviewWindowController
        appState.onReviewRequested = { [weak reviewWindowController] shot in
            reviewWindowController?.open(shot)
        }
        appState.onFrictionStepReviewRequested = { [weak reviewWindowController] step in
            reviewWindowController?.openFrictionStep(step)
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
            || environment["ASTROSHOTS_FRICTION_CAPTURE_DIR"] != nil
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

        // Placeholder target until AppDelegate attaches SPUStandardUpdaterController.
        // Title matches Apple / Sparkle convention (ellipsis for a further UI step).
        let checkItem = NSMenuItem(
            title: "Check for Updates…",
            action: #selector(SPUStandardUpdaterController.checkForUpdates(_:)),
            keyEquivalent: ""
        )
        checkItem.target = nil
        checkItem.isEnabled = false
        menu.addItem(checkItem)
        checkForUpdatesMenuItem = checkItem

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
        appState.onFrictionStepReviewRequested = nil
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
        presentTraySurface()
    }

    /// Seeds siblings and opens the tray already drilled into detail for `path`.
    func openTrayDetail(atImagePath path: String) {
        guard let shot = loadTestShots(atImagePath: path) else { return }
        appState.selectShot(shot)
        presentTraySurface()
    }

    /// Seeds a complete multi-worktree stream for grouped-stream UI proofs.
    /// Also loads any `.astroshot/friction-logs/` scenarios under the root.
    func openTray(atRootPath path: String) {
        let reader = AstroshotWatcher(
            configuration: .init(
                roots: [URL(fileURLWithPath: path, isDirectory: true)]
            )
        )
        let shots = reader.scanAll()
        let logs = reader.scanAllFrictionLogs()
        guard !shots.isEmpty || !logs.isEmpty else { return }
        for shot in shots.reversed() {
            appState.handleNewShot(shot)
        }
        if !logs.isEmpty {
            appState.frictionLogs = logs
            appState.selectedFrictionLogID = logs.first?.id
            appState.selectedFrictionRunID = logs.first?.latestRun?.id
            appState.selectedFrictionStepID = logs.first?.latestRun?.steps.first?.id
        }
        presentTraySurface()
    }

    /// UI-test / friction / movie-capture launches prefer a real on-screen
    /// panel so `astroshot movie desktop.window` can target Astroshots chrome.
    /// Interactive menu-bar use still defaults to the status-item popover.
    private func presentTraySurface() {
        #if DEBUG
        if Self.shouldPresentPinnedTrayForAutomation {
            appState.isFixtureSession = true
            openPinnedWindow(forceOnScreen: true)
            return
        }
        #endif
        showPopover()
    }

    #if DEBUG
    /// True when launched under UI-test / friction-log capture env vars.
    static var shouldPresentPinnedTrayForAutomation: Bool {
        let env = ProcessInfo.processInfo.environment
        if env["ASTROSHOTS_UI_TEST_POPOVER"] == "1" {
            // Escape hatch: force classic popover for specific UI tests.
            return false
        }
        return env["ASTROSHOTS_UI_TEST_TRAY_ROOT"] != nil
            || env["ASTROSHOTS_UI_TEST_TRAY_PATH"] != nil
            || env["ASTROSHOTS_UI_TEST_DETAIL_PATH"] != nil
            || env["ASTROSHOTS_FRICTION_CAPTURE_DIR"] != nil
            || env["ASTROSHOTS_UI_TEST_PINNED"] == "1"
    }
    #endif

    #if DEBUG
    /// Shows the production overlay without auto-dismiss so UI automation can
    /// prove the entire card opens review.
    func openOverlay(atImagePath path: String) {
        guard let shot = loadTestShot(atImagePath: path) else { return }
        appState.showOverlayForTesting(shot)
    }

    /// Agent friction-log runner: seed tray from `fixtureRoot`, walk Shots →
    /// Friction Logs → step detail via `AppState`, and write tray PNGs into
    /// `outputDir` (no Accessibility permission required).
    func captureFrictionLogJourney(fixtureRoot: String, outputDir: String) {
        let out = URL(fileURLWithPath: outputDir, isDirectory: true)
        try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

        openTray(atRootPath: fixtureRoot)

        Task { @MainActor in
            // Pinned NSWindow needs an extra beat for hosting layout before capture.
            try? await Task.sleep(nanoseconds: 900_000_000)
            self.captureTraySurface(to: out.appendingPathComponent("0001-open-tray-stream.png"))
            try? await Task.sleep(nanoseconds: 250_000_000)
            self.captureTraySurface(to: out.appendingPathComponent("0002-stream-chrome-tabs.png"))

            if let shot = self.appState.shots.first {
                self.appState.selectShot(shot)
            }
            try? await Task.sleep(nanoseconds: 450_000_000)
            self.captureTraySurface(to: out.appendingPathComponent("0003-shot-detail.png"))

            self.appState.backToStream()
            self.appState.selectTab(.frictionLogs)
            try? await Task.sleep(nanoseconds: 450_000_000)
            self.captureTraySurface(to: out.appendingPathComponent("0004-friction-logs-list.png"))

            if let log = self.appState.frictionLogs.first {
                self.appState.selectFrictionLog(log)
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
            self.captureTraySurface(to: out.appendingPathComponent("0005-friction-log-detail.png"))

            // When the fixture has multiple runs, flip to the next run so the
            // run picker / history UI is visible with a non-default selection.
            if let log = self.appState.selectedFrictionLog, log.runs.count > 1 {
                self.appState.selectFrictionRun(log.runs[1])
                try? await Task.sleep(nanoseconds: 400_000_000)
                self.captureTraySurface(
                    to: out.appendingPathComponent("0005b-friction-run-history.png")
                )
                // Return to newest so step captures match the default path.
                if let latest = log.latestRun {
                    self.appState.selectFrictionRun(latest)
                    try? await Task.sleep(nanoseconds: 300_000_000)
                }
            }

            // Step detail is its own tray page (not inline under the table).
            if let run = self.appState.selectedFrictionRun, let first = run.steps.first {
                self.appState.selectFrictionStep(first)
            }
            try? await Task.sleep(nanoseconds: 450_000_000)
            self.captureTraySurface(to: out.appendingPathComponent("0006-friction-step-detail.png"))

            if let run = self.appState.selectedFrictionRun, run.steps.count > 1 {
                self.appState.selectFrictionStep(run.steps[1])
            } else {
                self.appState.stepFrictionStep(1)
            }
            try? await Task.sleep(nanoseconds: 450_000_000)
            self.captureTraySurface(to: out.appendingPathComponent("0007-friction-step-two.png"))

            try? "ok".write(
                to: out.appendingPathComponent("CAPTURE_OK"),
                atomically: true,
                encoding: .utf8
            )
            try? await Task.sleep(nanoseconds: 150_000_000)
            NSApp.terminate(nil)
        }
    }

    private func captureTraySurface(to url: URL) {
        // Prefer the on-screen pinned window (automation / movie capture path).
        if let window = pinnedWindow, window.isVisible {
            window.layoutIfNeeded()
            if let content = window.contentView {
                content.layoutSubtreeIfNeeded()
                let bounds = content.bounds
                if bounds.width > 1, bounds.height > 1,
                   let rep = content.bitmapImageRepForCachingDisplay(in: bounds)
                {
                    content.cacheDisplay(in: bounds, to: rep)
                    if let data = rep.representation(using: .png, properties: [:]) {
                        try? data.write(to: url, options: .atomic)
                        return
                    }
                }
            }
            // Fallback: CGWindow capture of the real window (matches desktop.window).
            let windowID = CGWindowID(window.windowNumber)
            if let cgImage = CGWindowListCreateImage(
                .null,
                .optionIncludingWindow,
                windowID,
                [.boundsIgnoreFraming, .bestResolution]
            ) {
                let rep = NSBitmapImageRep(cgImage: cgImage)
                if let data = rep.representation(using: .png, properties: [:]) {
                    try? data.write(to: url, options: .atomic)
                    return
                }
            }
        }

        let view = popover?.contentViewController?.view
            ?? pinnedWindow?.contentViewController?.view
        guard let view else { return }
        view.layoutSubtreeIfNeeded()
        view.window?.layoutIfNeeded()
        let bounds = view.bounds
        guard bounds.width > 1, bounds.height > 1,
              let rep = view.bitmapImageRepForCachingDisplay(in: bounds)
        else { return }
        view.cacheDisplay(in: bounds, to: rep)
        guard let data = rep.representation(using: .png, properties: [:]) else { return }
        try? data.write(to: url, options: .atomic)
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
    /// Gated by `AppState.shouldPresentFirstRunStartup` (explicit preference
    /// flag), not merely an empty root list — so upgrades with saved roots and
    /// later empty states do not all look like first run.
    ///
    /// The status item needs a run-loop turn before its button has a window;
    /// otherwise the popover is a no-op.
    func presentFirstRunSetupIfNeeded() {
        guard appState.shouldPresentFirstRunStartup else { return }
        appState.pane = .stream
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            guard let self, self.appState.shouldPresentFirstRunStartup else { return }
            self.showPopover()
            // Let the popover present before the modal folder panel steals focus.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
                guard let self, self.appState.shouldPresentFirstRunStartup else { return }
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

    private func openPinnedWindow(forceOnScreen: Bool = false) {
        closePopover()
        if pinnedWindow == nil {
            let root = TrayView(isPinned: true)
                .environment(appState)
                .environment(\.trayChrome, trayChromeForPinned())
                .environment(\.reviewChrome, reviewChrome())
            let hosting = NSHostingController(rootView: root)
            // Use a normal titled window (not floating panel) so CGWindowList
            // reports layer 0 + onScreen when agents run desktop.window capture.
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: Theme.trayWidth, height: Theme.trayHeight),
                styleMask: [.titled, .closable, .fullSizeContentView],
                backing: .buffered,
                defer: false
            )
            window.title = "Astroshots"
            window.titleVisibility = .visible
            window.titlebarAppearsTransparent = false
            window.level = .normal
            window.isReleasedWhenClosed = false
            window.collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary]
            window.contentViewController = hosting
            window.setContentSize(NSSize(width: Theme.trayWidth, height: Theme.trayHeight))
            window.setFrameAutosaveName("AstroshotsPinnedTray")
            pinnedWindow = window
        }
        placePinnedWindowOnScreen(force: forceOnScreen)
        // Accessory (menu-bar-only) apps can still host normal windows; force
        // activation so the tray is on-screen for desktop.window capture.
        if forceOnScreen {
            NSApp.setActivationPolicy(.accessory)
        }
        pinnedWindow?.makeKeyAndOrderFront(nil)
        pinnedWindow?.orderFrontRegardless()
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Centers the pinned tray on the main screen so it is on-screen and
    /// capturable (avoids the off-screen 0,1192 popover host artifact).
    private func placePinnedWindowOnScreen(force: Bool) {
        guard let window = pinnedWindow else { return }
        let size = NSSize(width: Theme.trayWidth, height: Theme.trayHeight)
        window.setContentSize(size)
        guard let screen = NSScreen.main ?? NSScreen.screens.first else {
            window.center()
            return
        }
        let visible = screen.visibleFrame
        let origin = NSPoint(
            x: visible.midX - size.width / 2,
            y: visible.midY - size.height / 2
        )
        window.setFrame(
            NSRect(origin: origin, size: NSSize(width: size.width, height: size.height + 28)),
            display: true
        )
        if force {
            // Clear any remembered off-screen frame from a previous session.
            window.setFrameOrigin(origin)
        }
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
            },
            openFrictionStep: { [weak self] step in
                self?.closePopover()
                self?.reviewWindowController?.openFrictionStep(step)
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
