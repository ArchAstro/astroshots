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
    func narrationCaptionsDefaultOffAndPersist() {
        let suiteName = "astroshots-preferences-captions-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)

        #expect(preferences.narrationCaptionsEnabled == false)
        preferences.narrationCaptionsEnabled = true
        #expect(Preferences(defaults: defaults).narrationCaptionsEnabled)
    }

    @Test @MainActor
    func hiddenFrictionLogIDsAreDeduplicatedAndPersisted() {
        let suiteName = "astroshots-hidden-friction-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let preferences = Preferences(defaults: defaults)

        preferences.hiddenFrictionLogIDs = ["/tmp/b::two", "", "/tmp/a::one", "/tmp/b::two"]

        #expect(preferences.hiddenFrictionLogIDs == ["/tmp/a::one", "/tmp/b::two"])
        #expect(Preferences(defaults: defaults).hiddenFrictionLogIDs == [
            "/tmp/a::one", "/tmp/b::two",
        ])
    }

    @Test @MainActor
    func hidingFiltersTheUXPersistsAndLeavesFilesOnDisk() throws {
        let suiteName = "astroshots-hidden-friction-state-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-hidden-friction-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let alphaPrompt = root.appendingPathComponent("alpha-prompt.md")
        try Data("Keep me".utf8).write(to: alphaPrompt)
        let alpha = makeFrictionLog(root: root, slug: "alpha", promptPath: alphaPrompt.path)
        let beta = makeFrictionLog(root: root, slug: "beta")
        let preferences = Preferences(defaults: defaults)
        let watcher = makeWatcher(root: root)
        defer { watcher.stop() }
        let state = AppState(
            preferences: preferences,
            watcher: watcher,
            automaticallyStartsWatching: false
        )
        state.replaceFrictionLogs([alpha, beta])
        state.selectFrictionLog(alpha)

        state.hideFrictionLog(alpha)

        #expect(state.discoveredFrictionLogs == [alpha, beta])
        #expect(state.frictionLogs == [beta])
        #expect(state.hiddenFrictionLogs == [alpha])
        #expect(state.selectedFrictionLogID == beta.id)
        #expect(state.pane == .stream)
        #expect(FileManager.default.fileExists(atPath: alphaPrompt.path))
        #expect(preferences.hiddenFrictionLogIDs == [alpha.id])

        let relaunchedWatcher = makeWatcher(root: root)
        defer { relaunchedWatcher.stop() }
        let relaunched = AppState(
            preferences: Preferences(defaults: defaults),
            watcher: relaunchedWatcher,
            automaticallyStartsWatching: false
        )
        relaunched.replaceFrictionLogs([alpha, beta])
        #expect(relaunched.frictionLogs == [beta])

        relaunched.restoreFrictionLog(id: alpha.id)
        #expect(relaunched.frictionLogs == [alpha, beta])
        #expect(relaunched.hiddenFrictionLogIDs.isEmpty)
    }

    @Test @MainActor
    func unseenCountsBadgeShotsAndFrictionLogs() async throws {
        let suiteName = "astroshots-unseen-badge-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-unseen-badge-\(UUID().uuidString)")
        let runDir = root.appendingPathComponent("run", isDirectory: true)
        try FileManager.default.createDirectory(at: runDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        try """
        {"step":1,"id":"home","title":"Home","description":"Landed","screenshots":[],"good":[],"improve":[]}
        """.write(
            to: runDir.appendingPathComponent("log.jsonl"),
            atomically: true,
            encoding: .utf8
        )
        let run = FrictionLogRun(
            runID: "20260813T120000Z",
            directoryPath: runDir.path,
            steps: [
                FrictionLogStep(
                    step: 1,
                    stepID: "home",
                    title: "Home",
                    description: "Landed",
                    transcript: "",
                    screenshotPaths: [],
                    good: [],
                    improve: [],
                    url: nil,
                    capturedAt: nil
                ),
            ],
            capturedAt: Date(),
            status: .complete
        )
        let log = FrictionLog(
            worktree: "demo",
            worktreePath: root.path,
            slug: "checkout",
            title: "Checkout",
            description: "",
            promptPath: nil,
            promptMarkdown: nil,
            status: .complete,
            runs: [run],
            updatedAt: Date()
        )
        let state = AppState(
            preferences: Preferences(defaults: defaults),
            watcher: makeWatcher(root: root),
            automaticallyStartsWatching: false
        )
        state.replaceFrictionLogs([log])

        #expect(state.unseenFrictionLogCount == 1)
        #expect(state.unseenCount(for: .frictionLogs) == 1)
        #expect(state.unseenShotCount == 0)

        try await state.markFrictionLogSeen(log)
        #expect(state.unseenFrictionLogCount == 0)
        #expect(state.frictionLogs.first?.reviewState == .seen)
        #expect(
            FileManager.default.fileExists(
                atPath: runDir.appendingPathComponent("review.json").path
            )
        )
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

    private func makeWatcher(root: URL) -> AstroshotWatcher {
        AstroshotWatcher(
            configuration: .init(
                roots: [],
                cacheFileURL: root.appendingPathComponent("shot-index-\(UUID().uuidString).json")
            )
        )
    }

    private func makeFrictionLog(
        root: URL,
        slug: String,
        promptPath: String? = nil
    ) -> FrictionLog {
        FrictionLog(
            worktree: "demo",
            worktreePath: root.path,
            slug: slug,
            title: slug.capitalized,
            description: "",
            promptPath: promptPath,
            promptMarkdown: nil,
            status: .ready,
            runs: [],
            updatedAt: Date()
        )
    }
}
