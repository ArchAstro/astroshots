import Foundation
import Testing
@testable import Astroshots

struct PreferencesTests {
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

        #expect(preferences.watchRootPaths == [parent, unrelated])
        #expect(defaults.stringArray(forKey: "watchRoots") == [parent, unrelated])
        #expect(defaults.string(forKey: "watchRoot") == parent)
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
}
