import Foundation
import Testing
@testable import Astroshots

@MainActor
struct ImagePagingTests {
    @Test
    func detailStepsThroughNewestFirstStreamWithLeftAsOlder() {
        let state = AppState(
            preferences: isolatedPreferences(),
            automaticallyStartsWatching: false
        )
        let newest = makeShot(path: "/tmp/wt/.astroshot/f/0003.png", sequence: "0003", daysAgo: 0)
        let middle = makeShot(path: "/tmp/wt/.astroshot/f/0002.png", sequence: "0002", daysAgo: 1)
        let oldest = makeShot(path: "/tmp/wt/.astroshot/f/0001.png", sequence: "0001", daysAgo: 2)

        // Mirror production order: newest first.
        state.handleNewShot(oldest)
        state.handleNewShot(middle)
        state.handleNewShot(newest)
        state.selectShot(newest)

        #expect(state.selectedShot?.fileName == "0003.png")
        #expect(state.canStepDetail(1))
        #expect(!state.canStepDetail(-1))
        #expect(state.detailPosition(for: newest.id)?.index == 1)
        #expect(state.detailPosition(for: newest.id)?.count == 3)

        state.stepDetail(1)
        #expect(state.selectedShot?.fileName == "0002.png")
        #expect(state.detailPosition(for: middle.id)?.index == 2)

        state.stepDetail(1)
        #expect(state.selectedShot?.fileName == "0001.png")
        #expect(!state.canStepDetail(1))
        #expect(state.canStepDetail(-1))

        state.stepDetail(-1)
        #expect(state.selectedShot?.fileName == "0002.png")
    }

    @Test
    func reviewSiblingsWalkOldestToNewestWithinTheSameRun() {
        let state = AppState(
            preferences: isolatedPreferences(),
            automaticallyStartsWatching: false
        )
        let a = makeShot(
            path: "/tmp/wt/.astroshot/settings/0001-a.png",
            sequence: "0001",
            runID: "run-1",
            daysAgo: 2
        )
        let b = makeShot(
            path: "/tmp/wt/.astroshot/settings/0002-b.png",
            sequence: "0002",
            runID: "run-1",
            daysAgo: 1
        )
        let c = makeShot(
            path: "/tmp/wt/.astroshot/settings/0003-c.png",
            sequence: "0003",
            runID: "run-1",
            daysAgo: 0
        )
        let otherRun = makeShot(
            path: "/tmp/wt/.astroshot/settings/0004-other.png",
            sequence: "0004",
            runID: "run-2",
            daysAgo: 0
        )
        for shot in [a, b, c, otherRun] {
            state.handleNewShot(shot)
        }

        #expect(state.reviewSiblings(from: b.id).map(\.fileName) == [
            "0001-a.png",
            "0002-b.png",
            "0003-c.png",
        ])
        #expect(state.reviewSibling(from: b.id, delta: -1)?.fileName == "0001-a.png")
        #expect(state.reviewSibling(from: b.id, delta: 1)?.fileName == "0003-c.png")
        #expect(state.reviewSibling(from: a.id, delta: -1) == nil)
        #expect(state.reviewSibling(from: c.id, delta: 1) == nil)
        #expect(state.reviewPosition(for: b.id)?.index == 2)
        #expect(state.reviewPosition(for: b.id)?.count == 3)
        #expect(state.reviewPosition(for: otherRun.id)?.count == 1)
    }

    private func isolatedPreferences() -> Preferences {
        let defaults = TestDefaults()
        return Preferences(defaults: defaults)
    }

    private func makeShot(
        path: String,
        sequence: String,
        runID: String? = "run-1",
        daysAgo: Int
    ) -> Shot {
        Shot(
            path: path,
            worktree: "wt",
            worktreePath: "/tmp/wt",
            feature: "settings",
            fileName: URL(fileURLWithPath: path).lastPathComponent,
            sequence: sequence,
            slug: sequence,
            title: sequence,
            description: "",
            url: nil,
            runID: runID,
            status: .pass,
            capturedAt: Date().addingTimeInterval(TimeInterval(-daysAgo * 86_400))
        )
    }
}
