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

    @Test
    func navigateReusesHostedContentViewController() throws {
        let state = AppState(
            preferences: isolatedPreferences(),
            automaticallyStartsWatching: false
        )
        let older = makeShot()
        let newer = makeShot(sequence: "0002")
        state.handleNewShot(older)
        state.handleNewShot(newer)
        let controller = ReviewWindowController(appState: state)
        controller.open(older)
        defer { controller.close() }
        let panel = try #require(controller.panel)
        let hosting = try #require(panel.contentViewController)
        let content = hosting.view

        for (delta, expected) in [(1, newer), (1, newer), (-1, older), (-1, older)] {
            panel.onNavigate?(delta)
            #expect(controller.currentShotID == expected.id)
            #expect(controller.panel === panel)
            #expect(panel.contentViewController === hosting)
            #expect(hosting.view === content)
            #expect(panel.firstResponder === panel)
        }
    }

    private func isolatedPreferences() -> Preferences {
        let defaults = TestDefaults()
        return Preferences(defaults: defaults)
    }

    private func makeShot(sequence: String = "0001") -> Shot {
        Shot(
            path: "/tmp/wt/.astroshot/settings/\(sequence).png",
            worktree: "wt",
            worktreePath: "/tmp/wt",
            feature: "settings",
            fileName: "\(sequence).png",
            sequence: sequence,
            slug: sequence,
            title: sequence,
            description: "",
            url: nil,
            runID: "run-1",
            status: .pass,
            capturedAt: Date()
        )
    }
}
