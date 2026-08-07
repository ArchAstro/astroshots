import AppKit
import SwiftUI

/// Owns the chromeless, screen-sized review surface.
///
/// The tray and desktop overlays remain lightweight entry points. Feedback and
/// Seen acknowledgement happen here, in a key window that can accept text.
/// Friction-log steps reuse the same panel shell with a notes rail instead of
/// the shot feedback composer.
@MainActor
final class ReviewWindowController {
    private enum Mode {
        case shot
        case frictionStep
    }

    private let appState: AppState
    private var panel: ReviewPanel?
    private var currentShotID: String?
    private var mode: Mode = .shot

    init(appState: AppState) {
        self.appState = appState
    }

    func open(_ shot: Shot) {
        mode = .shot
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
        present(root)
    }

    func openFrictionStep(_ step: FrictionLogStep) {
        mode = .frictionStep
        currentShotID = nil
        appState.selectedFrictionStepID = step.id
        let root = FrictionStepTakeoverView(
            step: step,
            appState: appState,
            onClose: { [weak self] in
                self?.close()
            },
            onNavigateStep: { [weak self] delta in
                self?.navigateFrictionStep(delta)
            }
        )
        present(root)
    }

    func close() {
        panel?.orderOut(nil)
        currentShotID = nil
        mode = .shot
    }

    private func present<Content: View>(_ root: Content) {
        let hosting = NSHostingController(rootView: root)

        let panel = panel ?? makePanel()
        panel.contentViewController = hosting
        panel.onEscape = { [weak self] in self?.close() }
        panel.onNavigate = { [weak self] delta in
            self?.navigate(delta)
        }

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

    private func navigate(_ delta: Int) {
        switch mode {
        case .shot:
            guard let currentShotID,
                  let shot = appState.reviewSibling(from: currentShotID, delta: delta)
            else { return }
            open(shot)
        case .frictionStep:
            navigateFrictionStep(delta)
        }
    }

    private func navigateFrictionStep(_ delta: Int) {
        guard appState.canStepFrictionStep(delta) else { return }
        appState.stepFrictionStep(delta)
        guard let step = appState.selectedFrictionStep else { return }
        openFrictionStep(step)
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
    /// Left = older (−1), right = newer (+1), matching photos-app paging.
    var onNavigate: ((Int) -> Void)?

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
        // Escape closes. Arrow keys page when the feedback editor is not
        // first responder, so caret movement still works while typing.
        switch event.keyCode {
        case 53: // escape
            onEscape?()
        case 123: // left arrow → older
            if isEditingText {
                super.keyDown(with: event)
            } else {
                onNavigate?(-1)
            }
        case 124: // right arrow → newer
            if isEditingText {
                super.keyDown(with: event)
            } else {
                onNavigate?(1)
            }
        default:
            super.keyDown(with: event)
        }
    }

    private var isEditingText: Bool {
        guard let responder = firstResponder else { return false }
        if let textView = responder as? NSTextView {
            return textView.isEditable
        }
        // Field editors used by NSTextField / SwiftUI text fields.
        if responder is NSText {
            return true
        }
        return false
    }
}

/// Opens the full-screen review window from SwiftUI hosted by AppKit.
struct ReviewChrome: Sendable {
    var open: @MainActor @Sendable (Shot) -> Void = { _ in }
    var openFrictionStep: @MainActor @Sendable (FrictionLogStep) -> Void = { _ in }
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
