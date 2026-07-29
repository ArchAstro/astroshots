import Foundation
import Testing
@testable import Astroshots

struct ReviewStoreTests {
    @Test func seenRoundTripAndImageReplacementInvalidatesAcknowledgement() async throws {
        // Setup: create a real feature directory and valid one-pixel PNG bytes.
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-review-\(UUID().uuidString)", isDirectory: true)
        let featureDirectory = root
            .appendingPathComponent(".astroshot/login-flow", isDirectory: true)
        let imageURL = featureDirectory.appendingPathComponent("0001-signed-in.png")
        try FileManager.default.createDirectory(
            at: featureDirectory,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: root) }

        let png = try #require(Data(base64Encoded:
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
        ))
        try png.write(to: imageURL)

        // Boundary crossing: persist a human acknowledgement through the actor into
        // the app-owned sidecar, including an actionable comment.
        let store = ReviewStore()
        let seen = try await store.markSeen(
            forImage: imageURL,
            featureDirectory: featureDirectory,
            runID: "login-run-1",
            commentBody: "The signed-in state is clear."
        )

        #expect(seen.state == .seen)
        #expect(seen.comments.map(\.body) == ["The signed-in state is clear."])

        let sidecarURL = featureDirectory.appendingPathComponent("review.json")
        let document = try await store.loadDocument(featureDirectory: featureDirectory)
        #expect(FileManager.default.fileExists(atPath: sidecarURL.path))
        #expect(document.version == 1)
        #expect(document.runID == "login-run-1")
        #expect(document.updatedAt != nil)
        #expect(document.reviews["0001-signed-in.png"]?.decision == .seen)
        let json = try #require(
            JSONSerialization.jsonObject(with: Data(contentsOf: sidecarURL))
                as? [String: Any]
        )
        #expect(json["version"] as? Int == 1)
        #expect(json["run_id"] as? String == "login-run-1")
        #expect(json["updated_at"] is String)
        let reviews = try #require(json["reviews"] as? [String: Any])
        let storedReview = try #require(
            reviews["0001-signed-in.png"] as? [String: Any]
        )
        #expect(storedReview["decision"] as? String == "seen")
        #expect(storedReview["reviewed_at"] is String)
        #expect(storedReview["image_sha256"] is String)
        let comments = try #require(storedReview["comments"] as? [[String: Any]])
        #expect(UUID(uuidString: comments[0]["id"] as? String ?? "") != nil)
        #expect(comments[0]["body"] as? String == "The signed-in state is clear.")
        #expect(comments[0]["created_at"] is String)
        #expect(
            !FileManager.default.fileExists(
                atPath: featureDirectory.appendingPathComponent("manifest.json").path
            )
        )

        // Inject an image revision at the same filename. Its old Seen state must
        // no longer be effective, while the human conversation remains visible.
        try (png + Data([0x00])).write(to: imageURL)
        let revised = try await store.review(
            forImage: imageURL,
            featureDirectory: featureDirectory
        )

        #expect(revised.state == .pending)
        #expect(revised.isStale)
        #expect(revised.review?.decision == .seen)
        #expect(revised.comments.map(\.body) == ["The signed-in state is clear."])

        // The same pixels in a later execution run are a new review subject.
        // Run mismatch hides both the prior decision and its run-scoped comments.
        try png.write(to: imageURL)
        let nextRun = try ReviewStore.readReview(
            forImage: imageURL,
            featureDirectory: featureDirectory,
            expectedRunID: "login-run-2"
        )
        #expect(nextRun.state == .pending)
        #expect(!nextRun.isStale)
        #expect(nextRun.comments.isEmpty)

        // The first mutation in that run atomically replaces the old run's map.
        _ = try await store.markSeen(
            forImage: imageURL,
            featureDirectory: featureDirectory,
            runID: "login-run-2",
            commentBody: "Seen independently in the second run."
        )
        let nextDocument = try await store.loadDocument(
            featureDirectory: featureDirectory
        )
        #expect(nextDocument.runID == "login-run-2")
        #expect(
            nextDocument.reviews["0001-signed-in.png"]?.comments.map(\.body)
                == ["Seen independently in the second run."]
        )
    }

    @Test func legacyDecisionsRemainReadable() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("astroshots-review-note-\(UUID().uuidString)")
        let imageURL = root.appendingPathComponent("0001-settings.png")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try Data([0x89, 0x50, 0x4e, 0x47]).write(to: imageURL)
        defer { try? FileManager.default.removeItem(at: root) }

        let hash = try ReviewStore.imageSHA256(at: imageURL)
        let legacyApproved = """
        {"version":1,"reviews":{"0001-settings.png":{"decision":"approved","comments":[],"image_sha256":"\(hash)"}}}
        """
        try Data(legacyApproved.utf8).write(to: root.appendingPathComponent("review.json"))
        let store = ReviewStore()
        let approvedSnapshot = try await store.review(
            forImage: imageURL,
            featureDirectory: root
        )
        #expect(approvedSnapshot.state == .seen)

        let legacyChanges = """
        {"version":1,"reviews":{"0001-settings.png":{"decision":"changes_requested","comments":[],"image_sha256":"\(hash)"}}}
        """
        try Data(legacyChanges.utf8).write(to: root.appendingPathComponent("review.json"))
        let changesSnapshot = try await store.review(
            forImage: imageURL,
            featureDirectory: root
        )
        #expect(changesSnapshot.state == .pending)
    }
}
