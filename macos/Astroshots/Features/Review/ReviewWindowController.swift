import AppKit
import SwiftUI

/// Owns the chromeless, screen-sized review surface.
///
/// The tray and desktop overlays remain lightweight entry points. Deliberate
/// decisions and feedback happen here, in a key window that can accept text.
@MainActor
final class ReviewWindowController {
    private let appState: AppState
    private var panel: ReviewPanel?
    private var currentShotID: String?

    init(appState: AppState) {
        self.appState = appState
    }

    func open(_ shot: Shot) {
        currentShotID = shot.id
        let root = ReviewTakeoverView(
            shot: shot,
            appState: appState,
            onClose: { [weak self] in
                self?.close()
            },
            onNavigate: { [weak self] delta in
                self?.navigate(delta)
            }
        )
        let hosting = NSHostingController(rootView: root)

        let panel = panel ?? makePanel()
        panel.contentViewController = hosting
        panel.onEscape = { [weak self] in self?.close() }

        let screen = panel.screen ?? NSScreen.main ?? NSScreen.screens.first
        if let frame = screen?.visibleFrame {
            panel.setFrame(frame, display: true)
        }
        panel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        self.panel = panel

        #if DEBUG
        captureDebugSnapshotIfRequested(from: panel)
        #endif
    }

    func close() {
        panel?.orderOut(nil)
        currentShotID = nil
    }

    private func navigate(_ delta: Int) {
        guard let currentShotID,
              let shot = appState.reviewSibling(from: currentShotID, delta: delta)
        else { return }
        open(shot)
    }

    private func makePanel() -> ReviewPanel {
        let panel = ReviewPanel()
        panel.backgroundColor = NSColor.clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.level = .modalPanel
        panel.collectionBehavior = [
            .canJoinAllSpaces,
            .fullScreenAuxiliary,
            .stationary,
        ]
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        return panel
    }

    #if DEBUG
    /// Gives visual regression tooling a synthetic-data snapshot without
    /// requiring macOS Screen Recording permission.
    private func captureDebugSnapshotIfRequested(from panel: NSPanel) {
        guard let outputPath = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_CAPTURE_PATH"
        ] else { return }

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            guard let view = panel.contentView else { return }
            let bounds = view.bounds
            guard bounds.width > 0, bounds.height > 0,
                  let bitmap = view.bitmapImageRepForCachingDisplay(in: bounds)
            else { return }
            view.cacheDisplay(in: bounds, to: bitmap)
            guard let data = bitmap.representation(using: .png, properties: [:]) else {
                return
            }
            try? data.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
        }
    }
    #endif
}

final class ReviewPanel: NSPanel {
    var onEscape: (() -> Void)?

    init() {
        super.init(
            contentRect: .zero,
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 {
            onEscape?()
        } else {
            super.keyDown(with: event)
        }
    }
}

/// Opens the full-screen review window from SwiftUI hosted by AppKit.
struct ReviewChrome: Sendable {
    var open: @MainActor @Sendable (Shot) -> Void = { _ in }
}

private enum ReviewChromeKey: EnvironmentKey {
    static let defaultValue = ReviewChrome()
}

extension EnvironmentValues {
    var reviewChrome: ReviewChrome {
        get { self[ReviewChromeKey.self] }
        set { self[ReviewChromeKey.self] = newValue }
    }
}
