import Foundation
import Testing
@testable import Astroshots

struct FrictionLogLoaderTests {
    @Test func parsesJSONLStepsAndResolvesScreenshots() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("friction-jsonl-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let png = root.appendingPathComponent("0001-land.png")
        try Data([0x89, 0x50, 0x4E, 0x47]).write(to: png)

        let log = root.appendingPathComponent("log.jsonl")
        let line = """
        {"step":1,"id":"land","title":"Land","description":"Opened home","transcript":"I land on home. The CTA is clear, but the trust strip sits low. Next I open the cart.","screenshots":["0001-land.png"],"good":["Clear CTA"],"improve":["Trust strip low"],"url":"/"}
        {"step":2,"id":"cart","title":"Cart","description":"Opened cart","narration":"From home I open the cart. The empty state is thin.","screenshot":"missing.png","good":[],"improve":["Empty state is thin"]}
        """
        try line.write(to: log, atomically: true, encoding: .utf8)

        let steps = FrictionLogLoader.parseJSONL(at: log, runDirectory: root)
        #expect(steps.count == 2)
        #expect(steps[0].step == 1)
        #expect(steps[0].stepID == "land")
        #expect(steps[0].screenshotPaths.count == 1)
        #expect(steps[0].screenshotPaths[0].hasSuffix("0001-land.png"))
        #expect(steps[0].good == ["Clear CTA"])
        #expect(steps[0].improve == ["Trust strip low"])
        #expect(steps[0].hasTranscript)
        #expect(steps[0].transcript.contains("land on home"))
        #expect(steps[1].screenshotPaths.isEmpty)
        #expect(steps[1].improve == ["Empty state is thin"])
        #expect(steps[1].transcript.contains("open the cart"))
    }

    @Test func parsesMissingTranscriptAsEmpty() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("friction-no-tx-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let log = root.appendingPathComponent("log.jsonl")
        try """
        {"step":1,"id":"legacy","title":"Legacy","description":"No transcript field","screenshots":[],"good":[],"improve":[]}
        """.write(to: log, atomically: true, encoding: .utf8)

        let steps = FrictionLogLoader.parseJSONL(at: log, runDirectory: root)
        #expect(steps.count == 1)
        #expect(steps[0].transcript.isEmpty)
        #expect(!steps[0].hasTranscript)
    }

    @Test func loadsNestedRunAndPrompt() throws {
        let worktree = FileManager.default.temporaryDirectory
            .appendingPathComponent("friction-load-\(UUID().uuidString)", isDirectory: true)
        let slugDir = worktree
            .appendingPathComponent(".astroshot/friction-logs/checkout-as-new-user", isDirectory: true)
        let runDir = slugDir.appendingPathComponent("runs/20260807T120000Z", isDirectory: true)
        try FileManager.default.createDirectory(at: runDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: worktree) }

        try """
        # Checkout as new user
        Walk the happy path.
        """.write(
            to: slugDir.appendingPathComponent("prompt.md"),
            atomically: true,
            encoding: .utf8
        )
        try """
        {"version":1,"slug":"checkout-as-new-user","title":"Checkout as new user","description":"Fresh account","status":"complete"}
        """.write(
            to: slugDir.appendingPathComponent("meta.json"),
            atomically: true,
            encoding: .utf8
        )

        let shot = runDir.appendingPathComponent("0001-home.png")
        try Data([0x89, 0x50, 0x4E, 0x47]).write(to: shot)
        try """
        {"step":1,"id":"home","title":"Home","description":"Landed","screenshots":["0001-home.png"],"good":["Hero clear"],"improve":[]}
        """.write(
            to: runDir.appendingPathComponent("log.jsonl"),
            atomically: true,
            encoding: .utf8
        )

        let logs = FrictionLogLoader.loadLogs(
            inAstroshot: worktree.appendingPathComponent(".astroshot", isDirectory: true)
        )
        #expect(logs.count == 1)
        let log = try #require(logs.first)
        #expect(log.slug == "checkout-as-new-user")
        #expect(log.title == "Checkout as new user")
        #expect(log.status == .complete)
        #expect(log.promptMarkdown?.contains("Walk the happy path") == true)
        #expect(log.runs.count == 1)
        #expect(log.runs[0].runID == "20260807T120000Z")
        #expect(log.runs[0].steps.count == 1)
        let located = try #require(FrictionLogLoader.loadRun(directory: runDir))
        #expect(located.0.slug == "checkout-as-new-user")
        #expect(located.1.runID == "20260807T120000Z")
        #expect(FrictionLogLoader.loadRun(directory: worktree) == nil)
        #expect(log.stepCount == 1)
        #expect(log.goodCount == 1)
        #expect(log.reviewState == .pending)
    }

    @Test func runReviewSidecarMarksTheLatestRunSeen() async throws {
        let worktree = FileManager.default.temporaryDirectory
            .appendingPathComponent("friction-review-\(UUID().uuidString)", isDirectory: true)
        let slugDir = worktree
            .appendingPathComponent(".astroshot/friction-logs/checkout", isDirectory: true)
        let first = slugDir.appendingPathComponent("runs/20260807T120000Z", isDirectory: true)
        try FileManager.default.createDirectory(at: first, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: worktree) }

        try "# prompt".write(
            to: slugDir.appendingPathComponent("prompt.md"),
            atomically: true,
            encoding: .utf8
        )
        try """
        {"step":1,"id":"home","title":"Home","description":"Landed","screenshots":[],"good":[],"improve":[]}
        """.write(
            to: first.appendingPathComponent("log.jsonl"),
            atomically: true,
            encoding: .utf8
        )

        let store = ReviewStore()
        _ = try await store.markSeen(
            forImage: first.appendingPathComponent("log.jsonl"),
            featureDirectory: first,
            runID: "20260807T120000Z"
        )

        let astroshot = worktree.appendingPathComponent(".astroshot", isDirectory: true)
        let seenLogs = FrictionLogLoader.loadLogs(inAstroshot: astroshot)
        #expect(seenLogs.first?.reviewState == .seen)

        let second = slugDir.appendingPathComponent("runs/20260807T130000Z", isDirectory: true)
        try FileManager.default.createDirectory(at: second, withIntermediateDirectories: true)
        try """
        {"step":1,"id":"home","title":"Home","description":"Again","screenshots":[],"good":[],"improve":[]}
        """.write(
            to: second.appendingPathComponent("log.jsonl"),
            atomically: true,
            encoding: .utf8
        )
        try FileManager.default.setAttributes(
            [.modificationDate: Date().addingTimeInterval(60)],
            ofItemAtPath: second.path
        )

        let nextLogs = FrictionLogLoader.loadLogs(inAstroshot: astroshot)
        #expect(nextLogs.first?.latestRun?.runID == "20260807T130000Z")
        #expect(nextLogs.first?.reviewState == .pending)
    }

    @Test func humanizesSkillStyleRunIDs() {
        let run = FrictionLogRun(
            runID: "20260809T140000Z",
            directoryPath: "/tmp",
            steps: [],
            capturedAt: Date(),
            status: nil
        )
        #expect(FrictionLogRun.parseSkillRunID("20260809T140000Z") != nil)
        #expect(FrictionLogRun.parseSkillRunID("20260809T140000Z-1") != nil)
        // Display title is locale/timezone dependent; just require a short non-raw label.
        #expect(run.displayTitle != run.runID)
        #expect(run.displayTitle.count < run.runID.count)
        #expect(run.stepCountLabel == "0 steps")
    }

    @Test func loadsAllNestedRunsNewestFirst() throws {
        let worktree = FileManager.default.temporaryDirectory
            .appendingPathComponent("friction-multirun-\(UUID().uuidString)", isDirectory: true)
        let slugDir = worktree
            .appendingPathComponent(".astroshot/friction-logs/checkout", isDirectory: true)
        try FileManager.default.createDirectory(at: slugDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: worktree) }

        try "# prompt".write(
            to: slugDir.appendingPathComponent("prompt.md"),
            atomically: true,
            encoding: .utf8
        )

        // Older run
        let older = slugDir.appendingPathComponent("runs/20260101T100000Z", isDirectory: true)
        try FileManager.default.createDirectory(at: older, withIntermediateDirectories: true)
        try Data([0x89, 0x50, 0x4E, 0x47]).write(to: older.appendingPathComponent("0001-a.png"))
        try """
        {"step":1,"id":"a","title":"A","description":"older","transcript":"Older run.","screenshots":["0001-a.png"],"good":[],"improve":[]}
        """.write(to: older.appendingPathComponent("log.jsonl"), atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes(
            [.modificationDate: Date(timeIntervalSince1970: 1_700_000_000)],
            ofItemAtPath: older.path
        )

        // Empty stub (skill created dir then aborted) — must not appear
        let empty = slugDir.appendingPathComponent("runs/20260101T110000Z", isDirectory: true)
        try FileManager.default.createDirectory(at: empty, withIntermediateDirectories: true)
        try Data().write(to: empty.appendingPathComponent("log.jsonl"))

        // Newer run
        let newer = slugDir.appendingPathComponent("runs/20260101T120000Z", isDirectory: true)
        try FileManager.default.createDirectory(at: newer, withIntermediateDirectories: true)
        try Data([0x89, 0x50, 0x4E, 0x47]).write(to: newer.appendingPathComponent("0001-b.png"))
        try """
        {"step":1,"id":"b","title":"B","description":"newer","transcript":"Newer run.","screenshots":["0001-b.png"],"good":["ok"],"improve":[]}
        """.write(to: newer.appendingPathComponent("log.jsonl"), atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes(
            [.modificationDate: Date(timeIntervalSince1970: 1_800_000_000)],
            ofItemAtPath: newer.path
        )

        let logs = FrictionLogLoader.loadLogs(
            inAstroshot: worktree.appendingPathComponent(".astroshot", isDirectory: true)
        )
        #expect(logs.count == 1)
        let log = try #require(logs.first)
        #expect(log.runs.count == 2, "Empty stub must be dropped; both real runs must load")
        #expect(log.runs.map(\.runID) == ["20260101T120000Z", "20260101T100000Z"])
        #expect(log.latestRun?.runID == "20260101T120000Z")
        #expect(log.stepCount == 1)
        #expect(log.goodCount == 1)
    }

    @Test func shotPathRejectsFrictionLogImages() {
        let nested =
            "/Users/x/proj/.astroshot/friction-logs/checkout/runs/r1/0001-home.png"
        #expect(ShotPath.parse(imagePath: nested) == nil)
        #expect(FrictionLogPath.containsImage(path: nested))

        let normal = "/Users/x/proj/.astroshot/install-wizard/0001-home.png"
        #expect(ShotPath.parse(imagePath: normal) != nil)
        #expect(!FrictionLogPath.containsImage(path: normal))
    }

    @Test func scanFeatureDirsExcludesFrictionLogsFromShots() throws {
        let worktree = FileManager.default.temporaryDirectory
            .appendingPathComponent("friction-exclude-\(UUID().uuidString)", isDirectory: true)
        let feature = worktree.appendingPathComponent(
            ".astroshot/install-wizard",
            isDirectory: true
        )
        let frictionRun = worktree.appendingPathComponent(
            ".astroshot/friction-logs/checkout/runs/r1",
            isDirectory: true
        )
        try FileManager.default.createDirectory(at: feature, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: frictionRun, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: worktree) }

        try Data("shot".utf8).write(to: feature.appendingPathComponent("0001-a.png"))
        try Data("flog".utf8).write(to: frictionRun.appendingPathComponent("0001-b.png"))
        try """
        {"step":1,"id":"a","title":"A","description":"d","screenshots":["0001-b.png"],"good":[],"improve":[]}
        """.write(
            to: frictionRun.appendingPathComponent("log.jsonl"),
            atomically: true,
            encoding: .utf8
        )
        try "# prompt".write(
            to: worktree.appendingPathComponent(
                ".astroshot/friction-logs/checkout/prompt.md"
            ),
            atomically: true,
            encoding: .utf8
        )

        let cacheURL = worktree.appendingPathComponent("shot-index.json")
        let watcher = AstroshotWatcher(
            configuration: .init(roots: [worktree], cacheFileURL: cacheURL)
        )
        defer { watcher.stop() }

        let shots = watcher.scanAll()
        #expect(shots.count == 1)
        #expect(shots[0].feature == "install-wizard")
        #expect(shots.allSatisfy { !$0.path.contains("friction-logs") })
    }
}
