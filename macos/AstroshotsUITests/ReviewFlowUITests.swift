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

        try fixturePNG(width: 1_600, height: 900)
            .write(to: imageURL, options: .atomic)
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
        try fixturePNG(width: 1_200, height: 400, showsEdgeMarkers: true)
            .write(to: imageURL, options: .atomic)
        terminateRunningAstroshots()
        let app = XCUIApplication()
        app.launchEnvironment["ASTROSHOTS_UI_TEST_OVERLAY_PATH"] = imageURL.path
        app.launch()

        let overlay = app.buttons["overlay.open.0001-settings.png"]
        XCTAssertTrue(overlay.waitForExistence(timeout: 8))
        let overlayScreenshot = overlay.screenshot()
        XCTAssertTrue(
            screenshot(overlayScreenshot, contains: { red, green, blue in
                red > 0.8 && green < 0.2 && blue < 0.2
            }),
            "The overlay cropped the red left-edge marker from the panoramic fixture."
        )
        XCTAssertTrue(
            screenshot(overlayScreenshot, contains: { red, green, blue in
                red < 0.2 && green > 0.8 && blue < 0.2
            }),
            "The overlay cropped the green right-edge marker from the panoramic fixture."
        )
        let proof = XCTAttachment(screenshot: overlayScreenshot)
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
        let preview = app.buttons["detail.preview"]
        let metadata = app.descendants(matching: .any)["detail.metadata"]
        let controls = app.descendants(matching: .any)["detail.review.controls"]
        XCTAssertTrue(viewport.waitForExistence(timeout: 3))
        XCTAssertTrue(compact.waitForExistence(timeout: 3))
        XCTAssertTrue(preview.waitForExistence(timeout: 3))
        XCTAssertTrue(metadata.waitForExistence(timeout: 3))
        XCTAssertTrue(controls.waitForExistence(timeout: 3))
        XCTAssertGreaterThanOrEqual(preview.frame.width, compact.frame.width - 26)
        XCTAssertLessThanOrEqual(preview.frame.height, 241)
        XCTAssertEqual(preview.frame.width / preview.frame.height, 16 / 9, accuracy: 0.08)
        XCTAssertGreaterThanOrEqual(controls.frame.minY, preview.frame.maxY - 1)
        XCTAssertGreaterThanOrEqual(metadata.frame.minX, compact.frame.minX - 1)
        XCTAssertLessThanOrEqual(metadata.frame.maxX, compact.frame.maxX + 1)
        XCTAssertGreaterThanOrEqual(controls.frame.minX, compact.frame.minX - 1)
        XCTAssertLessThanOrEqual(controls.frame.maxX, compact.frame.maxX + 1)
        XCTAssertGreaterThanOrEqual(controls.frame.minY, viewport.frame.minY - 1)
        XCTAssertLessThanOrEqual(controls.frame.maxY, viewport.frame.maxY + 1)
        XCTAssertTrue(controls.isHittable)
        let visualProof = XCTAttachment(
            screenshot: viewport.screenshot()
        )
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
    func testCompactPortraitDetailFitsThePreviewBox() throws {
        try fixturePNG(width: 900, height: 1_600)
            .write(to: imageURL, options: .atomic)

        let app = launchTrayApp()
        let detailEntry = app.buttons["stream.detail.0001-settings.png"]
        XCTAssertTrue(detailEntry.waitForExistence(timeout: 8))
        detailEntry.click()

        let compact = app.descendants(matching: .any)["detail.compact"]
        let preview = app.buttons["detail.preview"]
        XCTAssertTrue(compact.waitForExistence(timeout: 3))
        XCTAssertTrue(preview.waitForExistence(timeout: 3))
        XCTAssertLessThanOrEqual(preview.frame.height, 241)
        XCTAssertEqual(preview.frame.width / preview.frame.height, 9 / 16, accuracy: 0.08)
        XCTAssertGreaterThan(preview.frame.minX, compact.frame.minX + 12)
        XCTAssertLessThan(preview.frame.maxX, compact.frame.maxX - 12)

        let viewport = app.descendants(matching: .any)["detail.viewport"]
        XCTAssertTrue(viewport.waitForExistence(timeout: 3))
        let visualProof = XCTAttachment(
            screenshot: viewport.screenshot()
        )
        visualProof.name = "Compact portrait detail preview"
        visualProof.lifetime = .keepAlways
        add(visualProof)
    }

    @MainActor
    func testGroupedStreamCollapsesAndMarksSectionsSeen() throws {
        let groupedRoot = root.appendingPathComponent(
            "grouped-stream",
            isDirectory: true
        )
        let alphaFeature = groupedRoot.appendingPathComponent(
            "alpha-wt1/.astroshot/run",
            isDirectory: true
        )
        let betaFeature = groupedRoot.appendingPathComponent(
            "beta-wt2/.astroshot/run",
            isDirectory: true
        )
        try FileManager.default.createDirectory(
            at: alphaFeature,
            withIntermediateDirectories: true
        )
        try FileManager.default.createDirectory(
            at: betaFeature,
            withIntermediateDirectories: true
        )

        let alphaNewest = alphaFeature.appendingPathComponent("0004-alpha-newest.png")
        let alphaNext = alphaFeature.appendingPathComponent("0003-alpha-next.png")
        let beta = betaFeature.appendingPathComponent("0002-beta.png")
        let alphaOldest = alphaFeature.appendingPathComponent("0001-alpha-oldest.png")
        for image in [alphaNewest, alphaNext, beta, alphaOldest] {
            try fixturePNG(width: 800, height: 450)
                .write(to: image, options: .atomic)
        }
        let now = Date()
        try FileManager.default.setAttributes(
            [.modificationDate: now],
            ofItemAtPath: alphaNewest.path
        )
        try FileManager.default.setAttributes(
            [.modificationDate: now.addingTimeInterval(-1)],
            ofItemAtPath: alphaNext.path
        )
        try FileManager.default.setAttributes(
            [.modificationDate: now.addingTimeInterval(-2)],
            ofItemAtPath: beta.path
        )
        try FileManager.default.setAttributes(
            [.modificationDate: now.addingTimeInterval(-3)],
            ofItemAtPath: alphaOldest.path
        )

        terminateRunningAstroshots()
        let app = XCUIApplication()
        app.launchEnvironment["ASTROSHOTS_UI_TEST_TRAY_ROOT"] = groupedRoot.path
        app.launch()

        let toggle = app.buttons.matching(
            identifier: "stream.group.toggle"
        ).firstMatch
        let markGroupSeen = app.buttons.matching(
            identifier: "stream.group.seen"
        ).firstMatch
        XCTAssertTrue(toggle.waitForExistence(timeout: 8))
        XCTAssertTrue(markGroupSeen.waitForExistence(timeout: 3))
        XCTAssertTrue(app.buttons["stream.detail.0004-alpha-newest.png"].exists)
        XCTAssertTrue(app.buttons["stream.detail.0003-alpha-next.png"].exists)

        let visualProof = XCTAttachment(screenshot: app.screenshot())
        visualProof.name = "Grouped stream controls"
        visualProof.lifetime = .keepAlways
        add(visualProof)

        toggle.click()
        XCTAssertFalse(app.buttons["stream.detail.0004-alpha-newest.png"].exists)
        XCTAssertFalse(app.buttons["stream.detail.0003-alpha-next.png"].exists)
        toggle.click()
        XCTAssertTrue(
            app.buttons["stream.detail.0004-alpha-newest.png"]
                .waitForExistence(timeout: 3)
        )

        markGroupSeen.click()
        let alphaSidecar = alphaFeature.appendingPathComponent("review.json")
        XCTAssertTrue(
            waitForDecision(
                "seen",
                fileName: alphaNewest.lastPathComponent,
                in: alphaSidecar,
                timeout: 5
            )
        )
        XCTAssertTrue(
            waitForDecision(
                "seen",
                fileName: alphaNext.lastPathComponent,
                in: alphaSidecar,
                timeout: 5
            )
        )
        XCTAssertNil(
            reviewEntryIfPresent(
                fileName: alphaOldest.lastPathComponent,
                from: alphaSidecar
            )
        )

        let history = app.buttons["stream.history"]
        XCTAssertTrue(history.waitForExistence(timeout: 3))
        history.click()
        XCTAssertTrue(
            app.buttons["stream.detail.0004-alpha-newest.png"]
                .waitForExistence(timeout: 3)
        )
        XCTAssertFalse(app.buttons["stream.detail.0002-beta.png"].exists)
        XCTAssertEqual(
            app.buttons.matching(identifier: "stream.group.seen").count,
            0
        )
        history.click()
        XCTAssertTrue(
            app.buttons["stream.detail.0002-beta.png"]
                .waitForExistence(timeout: 3)
        )

        let markStreamSeen = app.buttons["stream.seen.all"]
        XCTAssertTrue(markStreamSeen.waitForExistence(timeout: 3))
        markStreamSeen.click()
        XCTAssertTrue(
            app.descendants(matching: .any)["stream.seen.empty"]
                .waitForExistence(timeout: 5)
        )
        XCTAssertTrue(
            waitForDecision(
                "seen",
                fileName: alphaOldest.lastPathComponent,
                in: alphaSidecar,
                timeout: 5
            )
        )
        XCTAssertTrue(
            waitForDecision(
                "seen",
                fileName: beta.lastPathComponent,
                in: betaFeature.appendingPathComponent("review.json"),
                timeout: 5
            )
        )

        XCTAssertTrue(history.waitForExistence(timeout: 3))
        history.click()
        XCTAssertTrue(
            app.buttons["stream.detail.0004-alpha-newest.png"]
                .waitForExistence(timeout: 3)
        )
        XCTAssertTrue(app.buttons["stream.detail.0002-beta.png"].exists)
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
        waitForDecision(
            expected,
            fileName: imageURL.lastPathComponent,
            in: sidecar,
            timeout: timeout
        )
    }

    private func waitForDecision(
        _ expected: String,
        fileName: String,
        in sidecar: URL,
        timeout: TimeInterval
    ) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if let review = reviewEntryIfPresent(
                fileName: fileName,
                from: sidecar
            ),
               review["decision"] as? String == expected {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        return false
    }

    private func fixturePNG(
        width: Int,
        height: Int,
        showsEdgeMarkers: Bool = false
    ) throws -> Data {
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
        context.setFillColor(red: 0.04, green: 0.04, blue: 0.05, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        if showsEdgeMarkers {
            context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
            context.fill(CGRect(x: 0, y: 0, width: 100, height: height))
            context.setFillColor(red: 0, green: 1, blue: 0, alpha: 1)
            context.fill(CGRect(x: width - 100, y: 0, width: 100, height: height))
        }

        let image = try XCTUnwrap(context.makeImage())
        let bitmap = NSBitmapImageRep(cgImage: image)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }

    @MainActor
    private func screenshot(
        _ screenshot: XCUIScreenshot,
        contains predicate: (_ red: CGFloat, _ green: CGFloat, _ blue: CGFloat) -> Bool
    ) -> Bool {
        guard let bitmap = NSBitmapImageRep(data: screenshot.pngRepresentation) else {
            return false
        }

        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 2) {
                guard let color = bitmap.colorAt(x: x, y: y)?
                    .usingColorSpace(.deviceRGB)
                else { continue }
                if predicate(color.redComponent, color.greenComponent, color.blueComponent) {
                    return true
                }
            }
        }
        return false
    }

    private func reviewEntry(from sidecar: URL) throws -> [String: Any] {
        try reviewEntry(
            fileName: imageURL.lastPathComponent,
            from: sidecar
        )
    }

    private func reviewEntry(
        fileName: String,
        from sidecar: URL
    ) throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(contentsOf: sidecar))
        let document = try XCTUnwrap(object as? [String: Any])
        let reviews = try XCTUnwrap(document["reviews"] as? [String: Any])
        return try XCTUnwrap(reviews[fileName] as? [String: Any])
    }

    private func reviewEntryIfPresent(
        fileName: String,
        from sidecar: URL
    ) -> [String: Any]? {
        guard let data = try? Data(contentsOf: sidecar),
              let object = try? JSONSerialization.jsonObject(with: data),
              let document = object as? [String: Any],
              let reviews = document["reviews"] as? [String: Any]
        else {
            return nil
        }
        return reviews[fileName] as? [String: Any]
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
