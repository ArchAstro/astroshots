import Foundation
import Testing
@testable import Astroshots

struct PreferencesTests {
    @Test @MainActor
    func narrationVoicePersistsAndRejectsUnknownValues() {
        let suiteName = "astroshots-preferences-voice-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)

        #expect(preferences.narrationVoice == "Ryan")
        preferences.narrationVoice = "Aiden"
        #expect(Preferences(defaults: defaults).narrationVoice == "Aiden")
        preferences.narrationVoice = "Not a voice"
        #expect(preferences.narrationVoice == "Ryan")
    }

    @Test @MainActor
    func freshInstallNeedsFirstRunStartup() {
        let suiteName = "astroshots-preferences-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)

        #expect(preferences.hasCompletedFirstRunSetup == false)
        #expect(preferences.shouldPresentFirstRunStartup)
        #expect(preferences.hasConfiguredWatchRoots == false)
        #expect(preferences.watchRootPaths.isEmpty)
    }

    @Test @MainActor
    func upgradeWithSavedRootsSkipsFirstRunStartup() {
        let suiteName = "astroshots-preferences-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set(["/tmp/astroshots-existing-work"], forKey: "watchRoots")

        let preferences = Preferences(defaults: defaults)
        let expected = URL(
            fileURLWithPath: "/tmp/astroshots-existing-work",
            isDirectory: true
        ).standardizedFileURL.path

        #expect(preferences.hasCompletedFirstRunSetup)
        #expect(preferences.shouldPresentFirstRunStartup == false)
        #expect(preferences.watchRootPaths == [expected])
    }

    @Test @MainActor
    func legacyWatchRootMigratesIntoTheMultipleRootModel() {
        let suiteName = "astroshots-preferences-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        defaults.set("~/legacy-work", forKey: "watchRoot")

        let preferences = Preferences(defaults: defaults)
        let expected = URL(
            fileURLWithPath: ("~/legacy-work" as NSString).expandingTildeInPath,
            isDirectory: true
        ).standardizedFileURL.path

        #expect(preferences.hasConfiguredWatchRoots)
        #expect(preferences.hasCompletedFirstRunSetup)
        #expect(preferences.shouldPresentFirstRunStartup == false)
        #expect(preferences.watchRootPaths == [expected])
    }

    @Test @MainActor
    func rootsAreDeduplicatedAndPersistedForOlderBuilds() {
        let suiteName = "astroshots-preferences-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)
        let parent = "/tmp/astroshots-work"
        let nested = "\(parent)/nested-project"
        let unrelated = "/Volumes/code"

        preferences.watchRootPaths = [nested, parent, parent, unrelated]

        #expect(preferences.hasConfiguredWatchRoots)
        #expect(preferences.watchRootPaths == [parent, unrelated])
        #expect(defaults.stringArray(forKey: "watchRoots") == [parent, unrelated])
        #expect(defaults.string(forKey: "watchRoot") == parent)
    }

    @Test @MainActor
    func folderPickerStartsInProjectsWhenPresent() {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let projects = home.appendingPathComponent("Projects", isDirectory: true)
        let expected = FileManager.default.fileExists(atPath: projects.path)
            ? projects
            : home
        #expect(Preferences.folderPickerStartURL.path == expected.path)
    }

    @Test @MainActor
    func symlinkAndRealPathCollapseToOneWatchedRoot() throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-root-alias-\(UUID().uuidString)",
                isDirectory: true
            )
        let realRoot = temporary.appendingPathComponent("real", isDirectory: true)
        let aliasRoot = temporary.appendingPathComponent("alias", isDirectory: true)
        try FileManager.default.createDirectory(
            at: realRoot,
            withIntermediateDirectories: true
        )
        try FileManager.default.createSymbolicLink(
            at: aliasRoot,
            withDestinationURL: realRoot
        )
        defer { try? FileManager.default.removeItem(at: temporary) }

        #expect(
            Preferences.normalizeWatchRootPaths([aliasRoot.path, realRoot.path])
                == [realRoot.path]
        )
    }

    @Test @MainActor
    func appStatePresentsFirstRunUntilSetupCompletes() {
        let suiteName = "astroshots-first-run-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-first-run-\(UUID().uuidString)",
                isDirectory: true
            )
        try? FileManager.default.createDirectory(
            at: temporary,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: temporary) }

        let watcher = AstroshotWatcher(
            configuration: .init(
                roots: [],
                cacheFileURL: temporary.appendingPathComponent("shot-index.json")
            )
        )
        defer { watcher.stop() }

        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: true
        )

        #expect(state.shouldPresentFirstRunStartup)
        #expect(state.needsWatchRootSetup)
        #expect(state.watchRootPaths.isEmpty)
        #expect(state.isWatching == false)

        // Completing setup marks first-run done (same path as the folder panel).
        state.finishWatchRootSetup([temporary.path])

        #expect(state.shouldPresentFirstRunStartup == false)
        #expect(state.needsWatchRootSetup == false)
        #expect(state.watchRootPaths == [temporary.path])
        #expect(state.isWatching)
        #expect(preferences.hasCompletedFirstRunSetup)
        #expect(preferences.hasConfiguredWatchRoots)
    }

    @Test @MainActor
    func cancellingFirstRunKeepsStartupPendingForNextLaunch() {
        let suiteName = "astroshots-first-run-cancel-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)

        #expect(preferences.shouldPresentFirstRunStartup)

        // Simulate cancel: roots stay empty and first-run is not marked complete.
        #expect(preferences.hasCompletedFirstRunSetup == false)

        let again = Preferences(defaults: defaults)
        #expect(again.shouldPresentFirstRunStartup)
        #expect(again.watchRootPaths.isEmpty)
    }
}
