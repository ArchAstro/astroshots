import AppKit
import XCTest

final class ReviewFlowUITests: XCTestCase {
    private var root: URL!
    private var featureDirectory: URL!
    private var imageURL: URL!

    override func setUpWithError() throws {
        continueAfterFailure = false
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-review-ui-\(UUID().uuidString)", isDirectory: true)
        featureDirectory = root
            .appendingPathComponent(".astroshot/settings-review", isDirectory: true)
        imageURL = featureDirectory.appendingPathComponent("0001-settings.png")
        try FileManager.default.createDirectory(
            at: featureDirectory,
            withIntermediateDirectories: true
        )

        let png = try XCTUnwrap(
            Data(
                base64Encoded:
                    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
            )
        )
        try png.write(to: imageURL)
        try manifestJSON().write(
            to: featureDirectory.appendingPathComponent("manifest.json"),
            atomically: true,
            encoding: .utf8
        )
    }

    override func tearDownWithError() throws {
        if let root {
            try? FileManager.default.removeItem(at: root)
        }
    }

    @MainActor
    func testThumbnailTapOpensReviewTakeover() throws {
        // Setup boundary: seed the real stream popover with a filesystem frame.
        let app = launchTrayApp()
        let thumbnail = app.buttons["stream.review.0001-settings.png"]
        XCTAssertTrue(thumbnail.waitForExistence(timeout: 8))

        // Human entry path: tapping the stream image must create the
        // chromeless review surface, not merely navigate to tray detail.
        thumbnail.click()
        let takeover = app.descendants(matching: .any)["review.takeover"]
        XCTAssertTrue(takeover.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["review.close"].exists)

        app.buttons["review.close"].click()
        XCTAssertFalse(takeover.waitForExistence(timeout: 1))
    }

    @MainActor
    func testOverlayCardOpensReviewFromItsThumbnail() throws {
        terminateRunningAstroshots()
        let app = XCUIApplication()
        app.launchEnvironment["ASTROSHOTS_UI_TEST_OVERLAY_PATH"] = imageURL.path
        app.launch()

        let overlay = app.buttons["overlay.open.0001-settings.png"]
        XCTAssertTrue(overlay.waitForExistence(timeout: 8))
        let proof = XCTAttachment(screenshot: overlay.screenshot())
        proof.name = "Clickable screenshot overlay"
        proof.lifetime = .keepAlways
        add(proof)

        // Click in the image region, away from the textual affordance.
        overlay.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.64)).click()
        XCTAssertTrue(
            app.descendants(matching: .any)["review.takeover"]
                .waitForExistence(timeout: 5)
        )
    }

    @MainActor
    func testCompactDetailSupportsSeenAndFeedbackWithoutClipping() throws {
        // Setup boundary: seed and open the production tray popover with a
        // filesystem-backed frame whose temporary path is intentionally long.
        let app = launchTrayApp()
        let detailEntry = app.buttons["stream.detail.0001-settings.png"]
        XCTAssertTrue(detailEntry.waitForExistence(timeout: 8))
        detailEntry.click()

        // Compact layout boundary: every primary region must remain within the
        // real 400-point popover instead of expanding around the long path.
        let viewport = app.descendants(matching: .any)["detail.viewport"]
        let compact = app.descendants(matching: .any)["detail.compact"]
        let metadata = app.descendants(matching: .any)["detail.metadata"]
        let controls = app.descendants(matching: .any)["detail.review.controls"]
        XCTAssertTrue(viewport.waitForExistence(timeout: 3))
        XCTAssertTrue(compact.waitForExistence(timeout: 3))
        XCTAssertTrue(metadata.waitForExistence(timeout: 3))
        XCTAssertTrue(controls.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(metadata.frame.minX, compact.frame.minX - 1)
        XCTAssertLessThanOrEqual(metadata.frame.maxX, compact.frame.maxX + 1)
        XCTAssertGreaterThanOrEqual(controls.frame.minX, compact.frame.minX - 1)
        XCTAssertLessThanOrEqual(controls.frame.maxX, compact.frame.maxX + 1)
        XCTAssertGreaterThanOrEqual(controls.frame.minY, viewport.frame.minY - 1)
        XCTAssertLessThanOrEqual(controls.frame.maxY, viewport.frame.maxY + 1)
        XCTAssertTrue(controls.isHittable)
        let visualProof = XCTAttachment(screenshot: viewport.screenshot())
        visualProof.name = "Compact detail review controls"
        visualProof.lifetime = .keepAlways
        add(visualProof)
        let sidecar = featureDirectory.appendingPathComponent("review.json")
        let note = app.descendants(matching: .any)["detail.feedback.note"]
        XCTAssertTrue(note.waitForExistence(timeout: 3))
        note.click()
        note.typeText("Keep the primary action above the fold.")
        let sendFeedback = app.buttons["detail.feedback.send"]
        XCTAssertTrue(sendFeedback.isEnabled)
        sendFeedback.click()
        XCTAssertTrue(waitForFile(sidecar, timeout: 5))
        let review = try reviewEntry(from: sidecar)
        XCTAssertNil(review["decision"])
        let comments = try XCTUnwrap(review["comments"] as? [[String: Any]])
        XCTAssertEqual(
            comments.last?["body"] as? String,
            "Keep the primary action above the fold."
        )

        // Acknowledging from detail persists Seen and returns to the stream.
        let seen = app.buttons["detail.seen"]
        XCTAssertTrue(seen.waitForExistence(timeout: 3))
        seen.click()
        XCTAssertTrue(waitForDecision("seen", in: sidecar, timeout: 5))
        XCTAssertFalse(compact.waitForExistence(timeout: 1))
        XCTAssertTrue(
            app.descendants(matching: .any)["stream.seen.empty"]
                .waitForExistence(timeout: 3)
        )
    }

    @MainActor
    func testReviewerFeedbackAndSeenAcrossRevision() throws {
        // Setup and process boundary: launch the real LSUIElement application
        // directly into its production review window with a filesystem frame.
        let app = launchReviewApp()
        let takeover = app.descendants(matching: .any)["review.takeover"]
        XCTAssertTrue(takeover.waitForExistence(timeout: 8))
        XCTAssertGreaterThan(takeover.frame.width, 900)
        XCTAssertGreaterThan(takeover.frame.height, 600)
        XCTAssertTrue(app.buttons["review.close"].exists)
        XCTAssertTrue(
            app.descendants(matching: .any)["review.status.pending"].exists
        )

        // Human action: leave comment-only feedback without changing the
        // pending decision.
        let editor = app.textViews["review.comment.editor"]
        XCTAssertTrue(editor.waitForExistence(timeout: 3))
        editor.click()
        editor.typeText("The save button is clipped at the bottom.")

        let sendFeedback = app.buttons["review.feedback.send"]
        XCTAssertTrue(sendFeedback.isEnabled)
        sendFeedback.click()

        let sidecar = featureDirectory.appendingPathComponent("review.json")
        XCTAssertTrue(waitForFile(sidecar, timeout: 5))
        var review = try reviewEntry(from: sidecar)
        XCTAssertNil(review["decision"])
        var comments = try XCTUnwrap(review["comments"] as? [[String: Any]])
        XCTAssertEqual(
            comments.last?["body"] as? String,
            "The save button is clipped at the bottom."
        )
        XCTAssertEqual(comments.count, 1)
        let firstCommentID = try XCTUnwrap(comments.last?["id"] as? String)
        XCTAssertTrue(
            app.descendants(matching: .any)["review.comment.\(firstCommentID)"]
                .waitForExistence(timeout: 3)
        )
        XCTAssertTrue(
            app.descendants(matching: .any)["review.status.pending"].exists
        )

        // Seen acknowledges the exact current image bytes and closes review.
        let seen = app.buttons["review.seen"]
        XCTAssertTrue(seen.isEnabled)
        seen.click()
        XCTAssertTrue(waitForDecision("seen", in: sidecar, timeout: 5))
        XCTAssertFalse(takeover.waitForExistence(timeout: 1))
        review = try reviewEntry(from: sidecar)
        XCTAssertEqual(review["decision"] as? String, "seen")
        let seenHash = try XCTUnwrap(review["image_sha256"] as? String)

        // Inject a new revision through the real filesystem boundary. The next
        // app launch must preserve same-run feedback but revoke Seen.
        app.terminate()
        var revised = try Data(contentsOf: imageURL)
        revised.append(0)
        try revised.write(to: imageURL, options: .atomic)

        let revisedApp = launchReviewApp()
        XCTAssertTrue(
            revisedApp.descendants(matching: .any)["review.status.pending"]
                .waitForExistence(timeout: 8)
        )
        XCTAssertTrue(
            revisedApp.descendants(matching: .any)["review.stale"]
                .waitForExistence(timeout: 3)
        )
        XCTAssertTrue(
            revisedApp.descendants(matching: .any)["review.comment.\(firstCommentID)"]
                .waitForExistence(timeout: 3)
        )

        // Seen can carry new feedback for the replacement in the same action.
        let revisedEditor = revisedApp.textViews["review.comment.editor"]
        XCTAssertTrue(revisedEditor.waitForExistence(timeout: 3))
        revisedEditor.click()
        revisedEditor.typeText("Restore the missing primary action.")
        let revisedSeen = revisedApp.buttons["review.seen"]
        XCTAssertTrue(revisedSeen.isEnabled)
        revisedSeen.click()
        XCTAssertTrue(waitForDecision("seen", in: sidecar, timeout: 5))
        XCTAssertFalse(
            revisedApp.descendants(matching: .any)["review.takeover"]
                .waitForExistence(timeout: 1)
        )

        review = try reviewEntry(from: sidecar)
        XCTAssertEqual(review["decision"] as? String, "seen")
        XCTAssertNotEqual(review["image_sha256"] as? String, seenHash)
        comments = try XCTUnwrap(review["comments"] as? [[String: Any]])
        XCTAssertEqual(comments.count, 2)
        XCTAssertEqual(
            comments.last?["body"] as? String,
            "Restore the missing primary action."
        )
    }

    @MainActor
    private func launchReviewApp() -> XCUIApplication {
        terminateRunningAstroshots()
        let app = XCUIApplication()
        app.launchEnvironment["ASTROSHOTS_UI_TEST_REVIEW_PATH"] = imageURL.path
        app.launch()
        return app
    }

    @MainActor
    private func launchTrayApp() -> XCUIApplication {
        terminateRunningAstroshots()
        let app = XCUIApplication()
        app.launchEnvironment["ASTROSHOTS_UI_TEST_TRAY_PATH"] = imageURL.path
        app.launch()
        return app
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

    private func waitForFile(_ url: URL, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if FileManager.default.fileExists(atPath: url.path) {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        return false
    }

    private func waitForDecision(
        _ expected: String,
        in sidecar: URL,
        timeout: TimeInterval
    ) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if let review = try? reviewEntry(from: sidecar),
               review["decision"] as? String == expected {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        return false
    }

    private func reviewEntry(from sidecar: URL) throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(contentsOf: sidecar))
        let document = try XCTUnwrap(object as? [String: Any])
        let reviews = try XCTUnwrap(document["reviews"] as? [String: Any])
        return try XCTUnwrap(reviews[imageURL.lastPathComponent] as? [String: Any])
    }

    private func manifestJSON() -> String {
        """
        {
          "version": 1,
          "feature": "settings-review",
          "run_id": "settings-review-run-1",
          "status": "pass",
          "shots": [
            {
              "id": "0001",
              "file": "0001-settings.png",
              "slug": "settings",
              "title": "Settings panel",
              "description": "Review the save action and spacing.",
              "captured_at": "2026-07-26T14:00:00Z"
            }
          ]
        }
        """
    }
}
