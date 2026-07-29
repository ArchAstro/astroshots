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

        let image = NSImage(size: NSSize(width: 1_600, height: 900))
        image.lockFocus()
        NSColor(calibratedRed: 0.16, green: 0.22, blue: 0.31, alpha: 1).setFill()
        NSRect(x: 0, y: 0, width: 1_600, height: 900).fill()
        image.unlockFocus()
        let tiff = try XCTUnwrap(image.tiffRepresentation)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: tiff))
        let png = try XCTUnwrap(
            bitmap.representation(using: .png, properties: [:])
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
    func testCompactDetailSupportsApproveAndRequestChangesWithoutClipping() throws {
        // Setup boundary: seed and open the production tray popover with a
        // wide filesystem-backed frame whose temporary path is intentionally long.
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
        XCTAssertGreaterThanOrEqual(compact.frame.minX, viewport.frame.minX - 1)
        XCTAssertLessThanOrEqual(compact.frame.maxX, viewport.frame.maxX + 1)
        XCTAssertGreaterThanOrEqual(metadata.frame.minX, viewport.frame.minX - 1)
        XCTAssertLessThanOrEqual(metadata.frame.maxX, viewport.frame.maxX + 1)
        XCTAssertGreaterThanOrEqual(controls.frame.minX, viewport.frame.minX - 1)
        XCTAssertLessThanOrEqual(controls.frame.maxX, viewport.frame.maxX + 1)
        XCTAssertGreaterThanOrEqual(controls.frame.minY, viewport.frame.minY - 1)
        XCTAssertLessThanOrEqual(controls.frame.maxY, viewport.frame.maxY + 1)
        XCTAssertTrue(controls.isHittable)
        let visualProof = XCTAttachment(screenshot: viewport.screenshot())
        visualProof.name = "Compact detail review controls"
        visualProof.lifetime = .keepAlways
        add(visualProof)
        let requestChanges = app.buttons["detail.review.requestChanges"]
        XCTAssertTrue(requestChanges.exists)
        XCTAssertFalse(requestChanges.isEnabled)

        // Human action: approve directly in the compact detail without opening
        // the takeover, then observe the persisted sidecar and live badge.
        let approve = app.buttons["detail.review.approve"]
        XCTAssertTrue(approve.waitForExistence(timeout: 3))
        approve.click()

        let sidecar = featureDirectory.appendingPathComponent("review.json")
        XCTAssertTrue(waitForDecision("approved", in: sidecar, timeout: 5))
        XCTAssertTrue(
            app.descendants(matching: .any)["review.status.approved"]
                .waitForExistence(timeout: 3)
        )

        // Human correction: enter required feedback and request changes from
        // the same compact surface, proving both decisions cross the real store.
        let note = app.descendants(matching: .any)["detail.review.note"]
        XCTAssertTrue(note.waitForExistence(timeout: 3))
        note.click()
        note.typeText("Keep the primary action above the fold.")
        XCTAssertTrue(requestChanges.isEnabled)
        requestChanges.click()

        XCTAssertTrue(
            waitForDecision("changes_requested", in: sidecar, timeout: 5)
        )
        XCTAssertTrue(
            app.descendants(matching: .any)["review.status.changesRequested"]
                .waitForExistence(timeout: 3)
        )
        let review = try reviewEntry(from: sidecar)
        let comments = try XCTUnwrap(review["comments"] as? [[String: Any]])
        XCTAssertEqual(
            comments.last?["body"] as? String,
            "Keep the primary action above the fold."
        )
    }

    @MainActor
    func testReviewerCommentsApprovesAndRequestsChangesAcrossRevision() throws {
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

        let addComment = app.buttons["review.comment.add"]
        XCTAssertTrue(addComment.isEnabled)
        addComment.click()

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

        // A separate human action approves the exact current image bytes.
        let approve = app.buttons["review.approve"]
        XCTAssertTrue(approve.isEnabled)
        approve.click()
        XCTAssertTrue(waitForDecision("approved", in: sidecar, timeout: 5))
        XCTAssertTrue(
            app.descendants(matching: .any)["review.status.approved"]
                .waitForExistence(timeout: 3)
        )
        review = try reviewEntry(from: sidecar)
        XCTAssertEqual(review["decision"] as? String, "approved")
        let approvedHash = try XCTUnwrap(review["image_sha256"] as? String)

        // Inject a new revision through the real filesystem boundary. The next
        // app launch must preserve same-run feedback but revoke approval.
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

        // Human action on the replacement: request changes with a required
        // new comment, replacing the stale approval for the new image hash.
        let revisedEditor = revisedApp.textViews["review.comment.editor"]
        XCTAssertTrue(revisedEditor.waitForExistence(timeout: 3))
        revisedEditor.click()
        revisedEditor.typeText("Restore the missing primary action.")
        let requestChanges = revisedApp.buttons["review.requestChanges"]
        XCTAssertTrue(requestChanges.isEnabled)
        requestChanges.click()
        XCTAssertTrue(
            waitForDecision("changes_requested", in: sidecar, timeout: 5)
        )
        XCTAssertTrue(
            revisedApp.descendants(matching: .any)["review.status.changesRequested"]
                .waitForExistence(timeout: 3)
        )

        review = try reviewEntry(from: sidecar)
        XCTAssertEqual(review["decision"] as? String, "changes_requested")
        XCTAssertNotEqual(review["image_sha256"] as? String, approvedHash)
        comments = try XCTUnwrap(review["comments"] as? [[String: Any]])
        XCTAssertEqual(comments.count, 2)
        XCTAssertEqual(
            comments.last?["body"] as? String,
            "Restore the missing primary action."
        )

        // Window lifecycle boundary: the chromeless X closes the takeover.
        revisedApp.buttons["review.close"].click()
        XCTAssertFalse(
            revisedApp.descendants(matching: .any)["review.takeover"]
                .waitForExistence(timeout: 1)
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
