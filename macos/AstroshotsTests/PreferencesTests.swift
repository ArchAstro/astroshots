import Foundation
import Testing
@testable import Astroshots

struct PreferencesTests {
    @Test @MainActor
    func freshInstallHasNoWatchRootsUntilConfigured() {
        let suiteName = "astroshots-preferences-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)

        #expect(preferences.hasConfiguredWatchRoots == false)
        #expect(preferences.watchRootPaths.isEmpty)
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
    func appStateSkipsWatchingUntilRootsAreConfigured() {
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

        #expect(state.needsWatchRootSetup)
        #expect(state.watchRootPaths.isEmpty)
        #expect(state.isWatching == false)

        state.addWatchRoots([temporary.path])

        #expect(state.needsWatchRootSetup == false)
        #expect(state.watchRootPaths == [temporary.path])
        #expect(state.isWatching)
        #expect(preferences.hasConfiguredWatchRoots)
    }
}
