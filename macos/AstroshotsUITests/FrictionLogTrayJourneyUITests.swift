import AppKit
import XCTest

/// Drive the real tray through the review-tray-ux friction-log steps and dump
/// PNGs (+ a scaffold log.jsonl) into ASTROSHOTS_FRICTION_RUN_DIR.
///
/// Not part of the default CI suite expectations beyond "UI is reachable";
/// intended for agentic friction-log runs:
///
/// ```bash
/// export ASTROSHOTS_FRICTION_RUN_DIR=.../runs/<id>
/// xcodebuild test -scheme AstroshotsReviewUITests \
///   -only-testing:AstroshotsUITests/FrictionLogTrayJourneyUITests/testCaptureReviewTrayUXJourney
/// ```
final class FrictionLogTrayJourneyUITests: XCTestCase {
    private var fixtureRoot: URL!

    override func setUpWithError() throws {
        continueAfterFailure = false
        fixtureRoot = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "astroshots-friction-journey-\(UUID().uuidString)",
                isDirectory: true
            )
        try buildFixture(at: fixtureRoot)
    }

    override func tearDownWithError() throws {
        if let fixtureRoot {
            try? FileManager.default.removeItem(at: fixtureRoot)
        }
    }

    @MainActor
    func testCaptureReviewTrayUXJourney() throws {
        let runDirPath = ProcessInfo.processInfo.environment["ASTROSHOTS_FRICTION_RUN_DIR"]
        let runDir: URL
        if let runDirPath, !runDirPath.isEmpty {
            runDir = URL(fileURLWithPath: runDirPath, isDirectory: true)
            try FileManager.default.createDirectory(
                at: runDir,
                withIntermediateDirectories: true
            )
        } else {
            // Local Xcode runs still produce artifacts next to the fixture.
            runDir = fixtureRoot.appendingPathComponent("captured-run", isDirectory: true)
            try FileManager.default.createDirectory(at: runDir, withIntermediateDirectories: true)
        }

        terminateRunningAstroshots()
        let app = XCUIApplication()
        app.launchEnvironment["ASTROSHOTS_UI_TEST_TRAY_ROOT"] = fixtureRoot.path
        app.launch()

        // 1–2. Stream on Shots tab
        let streamDetail = app.buttons["stream.detail.0002-configure.png"]
        XCTAssertTrue(streamDetail.waitForExistence(timeout: 10), "Seeded stream should appear")
        try saveScreenshot(app.screenshot(), as: "0001-open-tray-stream.png", in: runDir)

        let shotsTab = app.buttons["tray.tab.shots"]
        let logsTab = app.buttons["tray.tab.frictionLogs"]
        XCTAssertTrue(shotsTab.waitForExistence(timeout: 3))
        XCTAssertTrue(logsTab.exists)
        try saveScreenshot(app.screenshot(), as: "0002-stream-chrome-tabs.png", in: runDir)

        // 3. Shot detail
        streamDetail.click()
        let detailPreview = app.buttons["detail.preview"]
        XCTAssertTrue(detailPreview.waitForExistence(timeout: 5))
        try saveScreenshot(app.screenshot(), as: "0003-shot-detail.png", in: runDir)

        // Back to stream for tab switch
        let streamBack = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Stream"))
            .firstMatch
        if streamBack.exists {
            streamBack.click()
        } else {
            // Label is "Stream" on the chevron button; fall back to keyboard-less path.
            app.buttons.element(boundBy: 0).click()
        }
        XCTAssertTrue(logsTab.waitForExistence(timeout: 5))

        // 4. Friction Logs list
        logsTab.click()
        let frictionRow = app.buttons["friction.row.sample-onboarding"]
        XCTAssertTrue(
            frictionRow.waitForExistence(timeout: 5),
            "Fixture friction log should list under Friction Logs tab"
        )
        try saveScreenshot(app.screenshot(), as: "0004-friction-logs-list.png", in: runDir)

        // 5. Open friction log detail
        frictionRow.click()
        let stepsTable = app.descendants(matching: .any)["friction.steps.table"]
        XCTAssertTrue(stepsTable.waitForExistence(timeout: 5))
        try saveScreenshot(app.screenshot(), as: "0005-friction-log-detail.png", in: runDir)

        // Prompt toggle
        let promptToggle = app.buttons["friction.prompt.toggle"]
        if promptToggle.exists {
            promptToggle.click()
            let promptBody = app.descendants(matching: .any)["friction.prompt.body"]
            _ = promptBody.waitForExistence(timeout: 2)
            try saveScreenshot(app.screenshot(), as: "0005b-friction-prompt.png", in: runDir)
        }

        // 6. Step detail (first step selected by default)
        let stepDetail = app.descendants(matching: .any)["friction.step.detail"]
        XCTAssertTrue(stepDetail.waitForExistence(timeout: 3))
        try saveScreenshot(app.screenshot(), as: "0006-friction-step-detail.png", in: runDir)

        // 7. Next step
        if app.buttons["friction.step.2"].waitForExistence(timeout: 2) {
            app.buttons["friction.step.2"].click()
        } else {
            // Use chrome next control if labeled
            let next = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Next step"))
                .firstMatch
            if next.exists { next.click() }
        }
        try saveScreenshot(app.screenshot(), as: "0007-friction-step-two.png", in: runDir)

        // Scaffold JSONL so the run directory is valid even before human polish.
        try writeScaffoldLog(to: runDir)
        let marker = runDir.appendingPathComponent("CAPTURE_OK")
        try "ok".write(to: marker, atomically: true, encoding: .utf8)
    }

    @MainActor
    func testHideAndRestoreFrictionLogWithoutDeletingIt() throws {
        terminateRunningAstroshots()
        let app = XCUIApplication()
        app.launchEnvironment["ASTROSHOTS_UI_TEST_TRAY_ROOT"] = fixtureRoot.path
        app.launch()

        let logsTab = app.buttons["tray.tab.frictionLogs"]
        XCTAssertTrue(logsTab.waitForExistence(timeout: 10))
        logsTab.click()

        let row = app.buttons["friction.row.sample-onboarding"]
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        let hide = app.buttons["friction.hide.sample-onboarding"]
        XCTAssertTrue(hide.waitForExistence(timeout: 3))
        hide.click()

        XCTAssertTrue(app.buttons["friction.hidden.manage"].waitForExistence(timeout: 3))
        XCTAssertFalse(row.exists)
        XCTAssertTrue(
            FileManager.default.fileExists(
                atPath: fixtureRoot.appendingPathComponent(
                    "demo-app/.astroshot/friction-logs/sample-onboarding/prompt.md"
                ).path
            )
        )

        app.buttons["friction.hidden.manage"].click()
        XCTAssertTrue(
            app.descendants(matching: .any)["settings.hidden-friction-logs"]
                .waitForExistence(timeout: 3)
        )
        let restore = app.buttons["settings.hidden-friction.restore"]
        XCTAssertTrue(restore.waitForExistence(timeout: 3))
        restore.click()

        let back = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Stream"))
            .firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 3))
        back.click()
        XCTAssertTrue(logsTab.waitForExistence(timeout: 3))
        logsTab.click()
        XCTAssertTrue(row.waitForExistence(timeout: 3))
    }

    // MARK: - Fixture

    private func buildFixture(at root: URL) throws {
        let feature = root.appendingPathComponent(
            "demo-app/.astroshot/install-wizard",
            isDirectory: true
        )
        let frictionRun = root.appendingPathComponent(
            "demo-app/.astroshot/friction-logs/sample-onboarding/runs/seed",
            isDirectory: true
        )
        try FileManager.default.createDirectory(at: feature, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: frictionRun, withIntermediateDirectories: true)

        let welcome = feature.appendingPathComponent("0001-welcome.png")
        let configure = feature.appendingPathComponent("0002-configure.png")
        try fixturePNG(width: 1280, height: 720, label: "Welcome").write(to: welcome)
        try fixturePNG(width: 1280, height: 720, label: "Configure").write(to: configure)
        try """
        {
          "version": 1,
          "feature": "install-wizard",
          "run_id": "fixture-run-1",
          "status": "pass",
          "description": "Fixture install wizard",
          "shots": [
            {
              "id": "0001",
              "file": "0001-welcome.png",
              "slug": "welcome",
              "title": "Welcome",
              "description": "Marketing welcome step.",
              "captured_at": "2026-08-07T12:00:00Z"
            },
            {
              "id": "0002",
              "file": "0002-configure.png",
              "slug": "configure",
              "title": "Configure project",
              "description": "Org and project fields.",
              "captured_at": "2026-08-07T12:01:00Z"
            }
          ]
        }
        """.write(
            to: feature.appendingPathComponent("manifest.json"),
            atomically: true,
            encoding: .utf8
        )

        let slugDir = frictionRun
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        try """
        # Sample onboarding

        Walk a new user through first project setup.
        """.write(
            to: slugDir.appendingPathComponent("prompt.md"),
            atomically: true,
            encoding: .utf8
        )
        try """
        {
          "version": 1,
          "slug": "sample-onboarding",
          "title": "Sample onboarding",
          "description": "Fixture friction log for tray demos",
          "status": "complete"
        }
        """.write(
            to: slugDir.appendingPathComponent("meta.json"),
            atomically: true,
            encoding: .utf8
        )

        let stepA = frictionRun.appendingPathComponent("0001-land.png")
        let stepB = frictionRun.appendingPathComponent("0002-create.png")
        try fixturePNG(width: 960, height: 540, label: "Land").write(to: stepA)
        try fixturePNG(width: 960, height: 540, label: "Create").write(to: stepB)
        try """
        {"step":1,"id":"land","title":"Land on home","description":"Opened / as a new user.","transcript":"I open the product as a new user. The hero is readable, but the primary CTA label is generic. Next I start creating a project.","screenshots":["0001-land.png"],"good":["Hero is readable"],"improve":["CTA label is generic"],"url":"/","captured_at":"2026-08-07T12:10:00Z"}
        {"step":2,"id":"create","title":"Create project","description":"Clicked Create and filled the form.","transcript":"From home I click Create and fill the form. Fields are labeled clearly, yet Submit stays disabled with no explanation when required values are short.","screenshots":["0002-create.png"],"good":["Fields are labeled"],"improve":["Submit stays disabled without explanation"],"url":"/projects/new","captured_at":"2026-08-07T12:11:00Z"}
        """.write(
            to: frictionRun.appendingPathComponent("log.jsonl"),
            atomically: true,
            encoding: .utf8
        )
    }

    private func writeScaffoldLog(to runDir: URL) throws {
        let now = ISO8601DateFormatter().string(from: Date())
        let lines: [[String: Any]] = [
            [
                "step": 1,
                "id": "open-tray-stream",
                "title": "Open tray on seeded stream",
                "description": "Launched Astroshots against the fixture root; Shots tab shows frames.",
                "transcript": "I launch Astroshots against the fixture root. The Shots tab already shows frames, so the stream is alive. Next I scan the chrome and tabs.",
                "screenshots": ["0001-open-tray-stream.png"],
                "good": [],
                "improve": [],
                "captured_at": now,
            ],
            [
                "step": 2,
                "id": "stream-chrome",
                "title": "Scan stream chrome and tabs",
                "description": "Observed status, stream list, and Shots | Friction Logs tab bar.",
                "transcript": "Looking over the chrome, status and stream list are clear, and the Shots versus Friction Logs tabs are easy to find. I open a shot for a closer look.",
                "screenshots": ["0002-stream-chrome-tabs.png"],
                "good": [],
                "improve": [],
                "captured_at": now,
            ],
            [
                "step": 3,
                "id": "shot-detail",
                "title": "Open a shot detail",
                "description": "Opened a stream row into compact detail with preview and metadata.",
                "transcript": "A stream row opens into compact detail with preview and metadata. That works cleanly. From here I switch over to Friction Logs.",
                "screenshots": ["0003-shot-detail.png"],
                "good": [],
                "improve": [],
                "captured_at": now,
            ],
            [
                "step": 4,
                "id": "friction-list",
                "title": "Switch to Friction Logs",
                "description": "Selected Friction Logs tab and saw the scenario list.",
                "transcript": "I select Friction Logs and the scenario list appears. Next I open one of the logs.",
                "screenshots": ["0004-friction-logs-list.png"],
                "good": [],
                "improve": [],
                "captured_at": now,
            ],
            [
                "step": 5,
                "id": "friction-detail",
                "title": "Open a friction log",
                "description": "Opened a log; step table and run metadata visible.",
                "transcript": "Opening a log shows the step table and run metadata. I dive into a step to read the notes.",
                "screenshots": ["0005-friction-log-detail.png", "0005b-friction-prompt.png"],
                "good": [],
                "improve": [],
                "captured_at": now,
            ],
            [
                "step": 6,
                "id": "friction-step",
                "title": "Inspect step notes",
                "description": "Reviewed screenshot plus Looks good / Can improve panels.",
                "transcript": "The step view pairs the screenshot with Looks good and Can improve panels. I move to the next step to confirm navigation holds up.",
                "screenshots": ["0006-friction-step-detail.png"],
                "good": [],
                "improve": [],
                "captured_at": now,
            ],
            [
                "step": 7,
                "id": "step-nav",
                "title": "Navigate to next step",
                "description": "Moved to step 2 and confirmed selection/position update.",
                "transcript": "Stepping forward updates selection and position as expected. The friction-log path through the tray holds together end to end.",
                "screenshots": ["0007-friction-step-two.png"],
                "good": [],
                "improve": [],
                "captured_at": now,
            ],
        ]
        var body = ""
        for line in lines {
            let data = try JSONSerialization.data(withJSONObject: line, options: [.sortedKeys])
            body += String(data: data, encoding: .utf8)! + "\n"
        }
        try body.write(
            to: runDir.appendingPathComponent("log.jsonl"),
            atomically: true,
            encoding: .utf8
        )
    }

    @MainActor
    private func saveScreenshot(
        _ screenshot: XCUIScreenshot,
        as name: String,
        in directory: URL
    ) throws {
        let url = directory.appendingPathComponent(name)
        try screenshot.pngRepresentation.write(to: url, options: .atomic)
    }

    private func fixturePNG(width: Int, height: Int, label: String) throws -> Data {
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let context = try XCTUnwrap(
            CGContext(
                data: nil,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: colorSpace,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
        )
        context.setFillColor(red: 0.10, green: 0.11, blue: 0.14, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(red: 0.38, green: 0.34, blue: 0.85, alpha: 1)
        context.fill(CGRect(x: 40, y: height - 120, width: 280, height: 48))

        let image = try XCTUnwrap(context.makeImage())
        let bitmap = NSBitmapImageRep(cgImage: image)
        // label kept for future text drawing; unused keeps compiler quiet if unused
        _ = label
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }

    @MainActor
    private func terminateRunningAstroshots() {
        let runningApps = NSRunningApplication
            .runningApplications(withBundleIdentifier: "ai.archastro.Astroshots")
        for runningApp in runningApps {
            runningApp.forceTerminate()
        }
        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline,
              NSRunningApplication
                .runningApplications(withBundleIdentifier: "ai.archastro.Astroshots")
                .isEmpty == false
        {
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
    }
}
