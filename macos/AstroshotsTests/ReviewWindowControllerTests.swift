import Foundation
import Testing
@testable import Astroshots

@MainActor
struct ReviewWindowControllerTests {
    @Test
    func closeNotifiesOnlyAfterTheViewerWasShown() {
        let state = AppState(
            preferences: isolatedPreferences(),
            automaticallyStartsWatching: false
        )
        let controller = ReviewWindowController(appState: state)
        var closed = 0
        controller.onClosed = { closed += 1 }

        controller.close()
        #expect(closed == 0)

        controller.open(makeShot())
        controller.close()
        #expect(closed == 1)

        controller.close()
        #expect(closed == 1)
    }

    private func isolatedPreferences() -> Preferences {
        let suiteName = "astroshots-review-window-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        return Preferences(defaults: defaults)
    }

    private func makeShot() -> Shot {
        Shot(
            path: "/tmp/wt/.astroshot/settings/0001.png",
            worktree: "wt",
            worktreePath: "/tmp/wt",
            feature: "settings",
            fileName: "0001.png",
            sequence: "0001",
            slug: "0001",
            title: "0001",
            description: "",
            url: nil,
            runID: "run-1",
            status: .pass,
            capturedAt: Date()
        )
    }
}
