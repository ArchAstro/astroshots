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
        {"step":1,"id":"land","title":"Land","description":"Opened home","screenshots":["0001-land.png"],"good":["Clear CTA"],"improve":["Trust strip low"],"url":"/"}
        {"step":2,"id":"cart","title":"Cart","description":"Opened cart","screenshot":"missing.png","good":[],"improve":["Empty state is thin"]}
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
        #expect(steps[1].screenshotPaths.isEmpty)
        #expect(steps[1].improve == ["Empty state is thin"])
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
