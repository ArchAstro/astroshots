import AppKit
import SwiftUI

/// AppKit status item: left-click toggles the tray popover; right-click shows Quit.
///
/// `MenuBarExtra` cannot attach a native context menu, so menu-bar interaction
/// is owned here instead of SwiftUI's menu-bar extra.
@MainActor
final class StatusItemController: NSObject {
    private let appState: AppState
    private var statusItem: NSStatusItem?
    private var popover: NSPopover?
    private var pinnedWindow: NSWindow?
    private var contextMenu: NSMenu?
    private var observationTask: Task<Void, Never>?

    init(appState: AppState) {
        self.appState = appState
        super.init()
    }

    func install() {
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
        popover.behavior = .transient
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
        if let item = statusItem {
            NSStatusBar.system.removeStatusItem(item)
        }
        statusItem = nil
        popover = nil
        contextMenu = nil
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

    @objc private func quit(_ sender: Any?) {
        NSApp.terminate(nil)
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
        let symbol = appState.isEmpty ? "camera" : "camera.fill"
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: "Astroshots")
        image?.isTemplate = true
        button.image = image
        if appState.unreadCount > 0 {
            button.title = " \(appState.unreadCount)"
        } else {
            button.title = ""
        }
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
