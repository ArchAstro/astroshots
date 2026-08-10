import AVKit
import SwiftUI
import WebKit

/// Stable AppKit-backed movie player used in both compact and full-screen review.
/// SwiftUI's `VideoPlayer` currently crashes while constructing its AVKit bridge
/// on macOS 26.5, so the player and its lifecycle live in `AVPlayerView`.
struct MoviePlayerView: NSViewRepresentable {
    enum Backend: Equatable {
        case avKit
        case webKit
    }

    let path: String
    var autoplay = false

    final class Coordinator: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
        var path: String?
        var autoplay = false
        var player: AVPlayer?
        var webView: WKWebView?
        var statusObservation: NSKeyValueObservation?
        weak var container: MoviePlayerContainerView?

        func userContentController(
            _ userContentController: WKUserContentController,
            didReceive message: WKScriptMessage
        ) {
            guard let status = message.body as? String else { return }
            if status == "ready" {
                container?.showPlayer()
            } else if status == "error" {
                container?.showError("This movie could not be played.")
            }
        }

        func webView(
            _ webView: WKWebView,
            didFail navigation: WKNavigation!,
            withError error: any Error
        ) {
            container?.showError("This movie could not be loaded.")
        }

        func webView(
            _ webView: WKWebView,
            didFailProvisionalNavigation navigation: WKNavigation!,
            withError error: any Error
        ) {
            container?.showError("This movie could not be loaded.")
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> MoviePlayerContainerView {
        let container = MoviePlayerContainerView()
        context.coordinator.container = container
        installPlayer(in: container, coordinator: context.coordinator)
        return container
    }

    func updateNSView(_ container: MoviePlayerContainerView, context: Context) {
        guard context.coordinator.path != path
                || context.coordinator.autoplay != autoplay
        else { return }
        installPlayer(in: container, coordinator: context.coordinator)
    }

    static func dismantleNSView(
        _ container: MoviePlayerContainerView,
        coordinator: Coordinator
    ) {
        removePlayer(from: container, coordinator: coordinator)
        coordinator.container = nil
    }

    /// Internal so the focused regression test can assert native controls.
    static func configuredAVPlayerView() -> AVPlayerView {
        let playerView = AVPlayerView()
        playerView.controlsStyle = .inline
        playerView.videoGravity = .resizeAspect
        playerView.showsFullScreenToggleButton = true
        playerView.showsFrameSteppingButtons = true
        playerView.showsSharingServiceButton = false
        playerView.updatesNowPlayingInfoCenter = false
        return playerView
    }

    static func backend(for path: String) -> Backend {
        URL(fileURLWithPath: path).pathExtension.lowercased() == "webm"
            ? .webKit
            : .avKit
    }

    private func installPlayer(
        in container: MoviePlayerContainerView,
        coordinator: Coordinator
    ) {
        Self.removePlayer(from: container, coordinator: coordinator)

        coordinator.path = path
        coordinator.autoplay = autoplay
        coordinator.container = container
        container.showLoading()

        switch Self.backend(for: path) {
        case .avKit:
            installAVPlayer(in: container, coordinator: coordinator)
        case .webKit:
            installWebPlayer(in: container, coordinator: coordinator)
        }
    }

    private func installAVPlayer(
        in container: MoviePlayerContainerView,
        coordinator: Coordinator
    ) {
        let playerView = Self.configuredAVPlayerView()
        pin(playerView, in: container.playerHost)

        let player = AVPlayer(url: URL(fileURLWithPath: path))
        coordinator.player = player
        playerView.player = player
        coordinator.statusObservation = player.currentItem?.observe(
            \.status,
            options: [.initial, .new]
        ) { [weak container] item, _ in
            DispatchQueue.main.async {
                switch item.status {
                case .readyToPlay:
                    container?.showPlayer()
                case .failed:
                    container?.showError("This movie could not be played.")
                case .unknown:
                    container?.showLoading()
                @unknown default:
                    container?.showError("This movie could not be played.")
                }
            }
        }

        if autoplay {
            player.play()
        }
    }

    private func installWebPlayer(
        in container: MoviePlayerContainerView,
        coordinator: Coordinator
    ) {
        let configuration = WKWebViewConfiguration()
        configuration.mediaTypesRequiringUserActionForPlayback = autoplay ? [] : .all
        configuration.userContentController.add(coordinator, name: "movieStatus")
        configuration.userContentController.addUserScript(
            WKUserScript(
                source: Self.webMovieScript,
                injectionTime: .atDocumentEnd,
                forMainFrameOnly: true
            )
        )

        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.underPageBackgroundColor = .black
        webView.navigationDelegate = coordinator
        pin(webView, in: container.playerHost)

        let videoURL = URL(fileURLWithPath: path)
        webView.loadFileURL(
            videoURL,
            allowingReadAccessTo: videoURL.deletingLastPathComponent()
        )
        coordinator.webView = webView
    }

    private func pin(_ child: NSView, in container: NSView) {
        child.frame = container.bounds
        child.autoresizingMask = [.width, .height]
        container.addSubview(child)
    }

    private static func removePlayer(
        from container: MoviePlayerContainerView,
        coordinator: Coordinator
    ) {
        coordinator.statusObservation?.invalidate()
        coordinator.statusObservation = nil
        coordinator.player?.pause()
        coordinator.player = nil
        coordinator.webView?.configuration.userContentController
            .removeScriptMessageHandler(forName: "movieStatus")
        coordinator.webView?.navigationDelegate = nil
        coordinator.webView?.stopLoading()
        coordinator.webView = nil
        coordinator.path = nil
        container.playerHost.subviews.forEach { $0.removeFromSuperview() }
    }

    private static let webMovieScript = """
    (() => {
      document.documentElement.style.background = '#000';
      document.body.style.cssText = 'margin:0;background:#000;overflow:hidden';
      const video = document.querySelector('video');
      if (!video) {
        window.webkit.messageHandlers.movieStatus.postMessage('error');
        return;
      }
      video.controls = true;
      video.style.cssText = 'width:100vw;height:100vh;object-fit:contain;background:#000';
      const ready = () => window.webkit.messageHandlers.movieStatus.postMessage('ready');
      const failed = () => window.webkit.messageHandlers.movieStatus.postMessage('error');
      if (video.readyState >= 2) ready();
      else video.addEventListener('loadeddata', ready, { once: true });
      video.addEventListener('error', failed, { once: true });
    })();
    """
}

final class MoviePlayerContainerView: NSView {
    let playerHost = NSView()

    private let loadingOverlay = NSView()
    private let spinner = NSProgressIndicator()
    private let statusLabel = NSTextField(labelWithString: "Loading movie…")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)

        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor

        playerHost.frame = bounds
        playerHost.autoresizingMask = [.width, .height]
        addSubview(playerHost)

        loadingOverlay.frame = bounds
        loadingOverlay.autoresizingMask = [.width, .height]
        loadingOverlay.wantsLayer = true
        loadingOverlay.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.82).cgColor
        addSubview(loadingOverlay)

        spinner.style = .spinning
        spinner.controlSize = .regular
        spinner.appearance = NSAppearance(named: .darkAqua)

        statusLabel.font = .systemFont(ofSize: 12, weight: .medium)
        statusLabel.textColor = NSColor.white.withAlphaComponent(0.76)
        statusLabel.alignment = .center

        let stack = NSStackView(views: [spinner, statusLabel])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false
        loadingOverlay.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: loadingOverlay.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: loadingOverlay.centerYAnchor),
        ])

        showLoading()
    }

    required init?(coder: NSCoder) {
        nil
    }

    func showLoading() {
        statusLabel.stringValue = "Loading movie…"
        spinner.isHidden = false
        spinner.startAnimation(nil)
        loadingOverlay.isHidden = false
    }

    func showPlayer() {
        spinner.stopAnimation(nil)
        loadingOverlay.isHidden = true
    }

    func showError(_ message: String) {
        spinner.stopAnimation(nil)
        spinner.isHidden = true
        statusLabel.stringValue = message
        loadingOverlay.isHidden = false
    }
}

struct ReviewBadge: View {
    let state: ReviewState

    private var presentation: (title: String, symbol: String, foreground: Color, background: Color) {
        switch state {
        case .pending:
            return ("Unseen", "circle.dashed", Theme.amber, Theme.amberSoft)
        case .seen:
            return ("Seen", "checkmark.circle.fill", Theme.blue, Theme.blue.opacity(0.1))
        }
    }

    private var stateIdentifier: String {
        switch state {
        case .pending: "pending"
        case .seen: "seen"
        }
    }

    var body: some View {
        let style = presentation
        Label(style.title, systemImage: style.symbol)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(style.foreground)
            .padding(.horizontal, 9)
            .padding(.vertical, 5)
            .background(style.background, in: Capsule())
            .accessibilityIdentifier("review.status.\(stateIdentifier)")
    }
}

struct ReviewCommentView: View {
    let comment: ReviewComment

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 7) {
                Image(systemName: "person.crop.circle.fill")
                    .font(.system(size: 16))
                    .foregroundStyle(Theme.purple.opacity(0.82))

                Text("Reviewer")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.ink)

                Spacer(minLength: 8)

                Text(displayDate)
                    .font(.system(size: 9.5))
                    .foregroundStyle(Theme.muted2)
            }

            Text(comment.body)
                .font(.system(size: 12))
                .foregroundStyle(Theme.ink2)
                .lineSpacing(2)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .fill(Color.white.opacity(0.78))
                .overlay(
                    RoundedRectangle(cornerRadius: 11, style: .continuous)
                        .stroke(Theme.line, lineWidth: 1)
                )
        )
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("review.comment.\(comment.id)")
    }

    private var displayDate: String {
        if let date = parseISO8601(comment.createdAt) {
            return date.formatted(date: .abbreviated, time: .shortened)
        }
        return comment.createdAt
    }

    private func parseISO8601(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) {
            return date
        }

        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: value)
    }
}

struct ReviewActionButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    enum Tone {
        case quiet
        case primary
    }

    let tone: Tone

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(foreground)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 9)
            .background(background(configuration: configuration), in: RoundedRectangle(cornerRadius: 9))
            .overlay(
                RoundedRectangle(cornerRadius: 9)
                    .stroke(border, lineWidth: tone == .quiet ? 1 : 0)
            )
            .scaleEffect(configuration.isPressed ? 0.985 : 1)
            .saturation(isEnabled ? 1 : 0.2)
            .opacity(isEnabled ? (configuration.isPressed ? 0.86 : 1) : 0.42)
            .animation(.easeOut(duration: 0.1), value: configuration.isPressed)
            .animation(.easeOut(duration: 0.12), value: isEnabled)
    }

    private var foreground: Color {
        switch tone {
        case .quiet: Theme.ink2
        case .primary: .white
        }
    }

    private func background(configuration: Configuration) -> Color {
        switch tone {
        case .quiet:
            return configuration.isPressed ? Theme.surface : Color.white.opacity(0.7)
        case .primary:
            return Theme.blue
        }
    }

    private var border: Color {
        tone == .quiet ? Theme.lineStrong : .clear
    }
}

#Preview("Review badges") {
    HStack(spacing: 10) {
        ReviewBadge(state: .pending)
        ReviewBadge(state: .seen)
    }
    .padding(20)
    .background(Theme.paper)
}
