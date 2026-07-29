import AppKit
import SwiftUI

/// Floating panels above all windows for newly landed screenshots.
@MainActor
final class OverlayController {
    var onOpen: ((Shot) -> Void)?

    private var panels: [OverlayPanel] = []
    private let maxStack = 3

    func show(shot: Shot, autoDismiss: Bool, duration: TimeInterval) {
        let panel = OverlayPanel()
        let root = OverlayView(
            shot: shot,
            onOpen: { [weak self, weak panel] in
                guard let panel else { return }
                self?.onOpen?(shot)
                self?.dismiss(panel)
            }
        )
        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 400, height: 320)
        panel.contentView = hosting
        panel.setContentSize(hosting.frame.size)

        position(panel, index: 0)
        panel.orderFrontRegardless()

        #if DEBUG
        captureDebugSnapshotIfRequested(from: panel)
        #endif

        // Stack existing downward.
        for (i, existing) in panels.enumerated() {
            position(existing, index: i + 1)
        }
        panels.insert(panel, at: 0)
        while panels.count > maxStack {
            if let last = panels.popLast() {
                last.orderOut(nil)
            }
        }

        if autoDismiss {
            DispatchQueue.main.asyncAfter(deadline: .now() + duration) { [weak self, weak panel] in
                guard let panel, let self, self.panels.contains(where: { $0 === panel }) else { return }
                self.dismiss(panel)
            }
        }
    }

    private func dismiss(_ panel: OverlayPanel) {
        panels.removeAll { $0 === panel }
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.2
            panel.animator().alphaValue = 0
        } completionHandler: {
            panel.orderOut(nil)
        }
        for (i, existing) in panels.enumerated() {
            position(existing, index: i)
        }
    }

    private func position(_ panel: OverlayPanel, index: Int) {
        guard let screen = NSScreen.main else { return }
        let visible = screen.visibleFrame
        let size = panel.frame.size
        let margin: CGFloat = 28
        let gap: CGFloat = 12
        let x = visible.maxX - size.width - margin
        let y = visible.maxY - size.height - margin - CGFloat(index) * (size.height * 0.15 + gap)
        panel.setFrameOrigin(NSPoint(x: x, y: y))
        panel.alphaValue = index == 0 ? 1 : 0.92
    }

    #if DEBUG
    private func captureDebugSnapshotIfRequested(from panel: NSPanel) {
        guard let outputPath = ProcessInfo.processInfo.environment[
            "ASTROSHOTS_UI_TEST_OVERLAY_CAPTURE_PATH"
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

final class OverlayPanel: NSPanel {
    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 320),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        isFloatingPanel = true
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        becomesKeyOnlyIfNeeded = true
        hidesOnDeactivate = false
    }

    override var canBecomeKey: Bool { true }
}
