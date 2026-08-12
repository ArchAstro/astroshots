import AppKit
import Testing
@testable import Astroshots

struct BrandAssetsTests {
    @Test @MainActor
    func menuBarMarkLoadsAsAResolutionIndependentTemplate() throws {
        let mark = try #require(NSImage(named: "AstroshotsMark"))

        #expect(mark.isTemplate)
        #expect(mark.size.width > 0)
        #expect(mark.size.height > 0)
    }

    @Test @MainActor
    func statusItemContextMenuOffersSettingsAndTheAppVersion() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(
            "astroshots-status-menu-\(UUID().uuidString)",
            isDirectory: true
        )
        try FileManager.default.createDirectory(
            at: root,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: root) }

        let suiteName = "astroshots-status-menu-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)
        preferences.watchRootPaths = [root.path]
        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [root],
                cacheFileURL: root.appendingPathComponent("shot-index.json")
            )
        )
        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: false
        )
        let controller = StatusItemController(appState: state)
        controller.install()
        defer {
            controller.teardown()
            watcher.stop()
        }

        let items = try #require(controller.contextMenu?.items)
        #expect(items.map(\.title) == [
            "Open Astroshots",
            "Settings…",
            "Check for Updates…",
            "",
            StatusItemController.versionMenuTitle(),
            "",
            "Quit Astroshots",
        ])
        #expect(items[1].isEnabled)
        // "Check for Updates…" stays disabled until AppDelegate attaches Sparkle
        // (unit tests never start the updater).
        #expect(!items[2].isEnabled)
        #expect(!items[4].isEnabled)

        let settingsAction = try #require(items[1].action)
        #expect(NSApp.sendAction(settingsAction, to: items[1].target, from: items[1]))
        #expect(state.pane == .settings)
    }

    @Test @MainActor
    func versionMenuTitleIncludesMarketingAndBuildVersions() {
        #expect(
            StatusItemController.versionMenuTitle(
                infoDictionary: [
                    "CFBundleShortVersionString": "1.2.3",
                    "CFBundleVersion": "45",
                ]
            ) == "Version 1.2.3 (45)"
        )
    }

    @Test
    func sparkleFeedAndPublicKeyAreEmbedded() throws {
        let info = try #require(Bundle.main.infoDictionary)
        let feed = try #require(info["SUFeedURL"] as? String)
        // Must not use releases/latest — dual npm/macOS tracks fight over that badge.
        #expect(feed == "https://github.com/ArchAstro/astroshots/releases/download/appcast/appcast.xml")

        let publicKey = try #require(info["SUPublicEDKey"] as? String)
        #expect(publicKey == "kddwW0gHUJ6CvdcYukaQd+cfNxEEHwQZHjzUYsjtVgc=")
    }

    @Test
    func updateChecksUseFreshCacheBustingParameters() throws {
        let first = SoftwareUpdateFeedRequest.makeRefreshToken()
        let second = SoftwareUpdateFeedRequest.makeRefreshToken()
        #expect(first != second)

        let parameters = SoftwareUpdateFeedRequest.parameters(refreshToken: first)
        let parameter = try #require(parameters.first)
        #expect(parameter["key"] == "refresh")
        #expect(parameter["value"] == first)
        #expect(parameter["displayKey"] == "Feed refresh")
    }
}
