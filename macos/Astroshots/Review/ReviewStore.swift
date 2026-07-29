import CryptoKit
import Darwin
import Foundation

actor ReviewStore {
    static let sidecarFileName = "review.json"

    enum StoreError: Error, Equatable {
        case unsupportedVersion(Int)
        case emptyComment
        case invalidImageFileName
    }

    private let fileManager: FileManager

    init(fileManager: FileManager = .default) {
        self.fileManager = fileManager
    }

    func loadDocument(featureDirectory: URL) throws -> ReviewDocument {
        let url = sidecarURL(in: featureDirectory)
        guard fileManager.fileExists(atPath: url.path) else {
            return ReviewDocument()
        }

        let data = try Data(contentsOf: url)
        let document = try Self.decoder.decode(ReviewDocument.self, from: data)
        guard document.version == ReviewDocument.currentVersion else {
            throw StoreError.unsupportedVersion(document.version)
        }
        return document
    }

    func review(
        forImage imageURL: URL,
        featureDirectory: URL,
        expectedRunID: String? = nil
    ) throws -> ReviewSnapshot {
        let fileName = try validatedFileName(for: imageURL)
        let document = try loadDocument(featureDirectory: featureDirectory)
        let hash = try Self.imageSHA256(at: imageURL)
        let review = expectedRunID == nil || document.runID == expectedRunID
            ? document.reviews[fileName]
            : nil
        return ReviewSnapshot(
            review: review,
            currentImageSHA256: hash
        )
    }

    static func readReview(
        forImage imageURL: URL,
        featureDirectory: URL,
        expectedRunID: String? = nil
    ) throws -> ReviewSnapshot {
        let sidecar = featureDirectory.appendingPathComponent(sidecarFileName)
        let document: ReviewDocument
        if FileManager.default.fileExists(atPath: sidecar.path) {
            document = try JSONDecoder().decode(
                ReviewDocument.self,
                from: Data(contentsOf: sidecar)
            )
            guard document.version == ReviewDocument.currentVersion else {
                throw StoreError.unsupportedVersion(document.version)
            }
        } else {
            document = ReviewDocument()
        }

        let hash = try imageSHA256(at: imageURL)
        let review = expectedRunID == nil || document.runID == expectedRunID
            ? document.reviews[imageURL.lastPathComponent]
            : nil
        return ReviewSnapshot(
            review: review,
            currentImageSHA256: hash
        )
    }

    @discardableResult
    func markSeen(
        note: String?,
        for shot: Shot
    ) throws -> ReviewSnapshot {
        let imageURL = URL(fileURLWithPath: shot.path)
        return try markSeen(
            forImage: imageURL,
            featureDirectory: imageURL.deletingLastPathComponent(),
            runID: shot.runID,
            commentBody: note
        )
    }

    @discardableResult
    func markSeen(
        forImage imageURL: URL,
        featureDirectory: URL,
        runID: String?,
        commentBody: String? = nil
    ) throws -> ReviewSnapshot {
        let fileName = try validatedFileName(for: imageURL)
        let imageHash = try Self.imageSHA256(at: imageURL)
        var document = try loadDocument(featureDirectory: featureDirectory)
        resetReviewsIfNeeded(in: &document, for: runID)
        var review = document.reviews[fileName] ?? ShotReview()

        if let commentBody {
            review.comments.append(try makeComment(body: commentBody))
        }
        review.decision = .seen
        review.reviewedAt = Self.timestamp()
        review.imageSHA256 = imageHash
        document.updatedAt = Self.timestamp()
        document.reviews[fileName] = review

        try writeDocument(document, featureDirectory: featureDirectory)
        return ReviewSnapshot(review: review, currentImageSHA256: imageHash)
    }

    @discardableResult
    func addComment(
        _ body: String,
        forFileName fileName: String,
        featureDirectory: URL,
        runID: String?
    ) throws -> ReviewComment {
        guard !fileName.isEmpty, fileName == URL(fileURLWithPath: fileName).lastPathComponent else {
            throw StoreError.invalidImageFileName
        }

        let comment = try makeComment(body: body)
        var document = try loadDocument(featureDirectory: featureDirectory)
        resetReviewsIfNeeded(in: &document, for: runID)
        var review = document.reviews[fileName] ?? ShotReview()
        review.comments.append(comment)
        document.updatedAt = Self.timestamp()
        document.reviews[fileName] = review
        try writeDocument(document, featureDirectory: featureDirectory)
        return comment
    }

    @discardableResult
    func addComment(
        _ body: String,
        to shot: Shot
    ) throws -> ReviewSnapshot {
        let imageURL = URL(fileURLWithPath: shot.path)
        let featureDirectory = imageURL.deletingLastPathComponent()
        _ = try addComment(
            body,
            forFileName: shot.fileName,
            featureDirectory: featureDirectory,
            runID: shot.runID
        )
        return try review(
            forImage: imageURL,
            featureDirectory: featureDirectory,
            expectedRunID: shot.runID
        )
    }

    func clearDecision(
        forFileName fileName: String,
        featureDirectory: URL,
        runID: String?
    ) throws {
        var document = try loadDocument(featureDirectory: featureDirectory)
        resetReviewsIfNeeded(in: &document, for: runID)
        guard var review = document.reviews[fileName] else { return }
        review.decision = nil
        review.reviewedAt = nil
        review.imageSHA256 = nil
        document.updatedAt = Self.timestamp()
        document.reviews[fileName] = review
        try writeDocument(document, featureDirectory: featureDirectory)
    }

    static func imageSHA256(at url: URL) throws -> String {
        let digest = SHA256.hash(data: try Data(contentsOf: url))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    private func writeDocument(
        _ document: ReviewDocument,
        featureDirectory: URL
    ) throws {
        try fileManager.createDirectory(
            at: featureDirectory,
            withIntermediateDirectories: true
        )
        let destination = sidecarURL(in: featureDirectory)
        let temporary = featureDirectory.appendingPathComponent(
            ".review.tmp.\(UUID().uuidString)"
        )
        let data = try Self.encoder.encode(document)
        try data.write(to: temporary, options: [.withoutOverwriting])
        defer { try? fileManager.removeItem(at: temporary) }

        guard rename(temporary.path, destination.path) == 0 else {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
    }

    private func sidecarURL(in featureDirectory: URL) -> URL {
        featureDirectory.appendingPathComponent(Self.sidecarFileName)
    }

    private func validatedFileName(for imageURL: URL) throws -> String {
        let fileName = imageURL.lastPathComponent
        guard !fileName.isEmpty else {
            throw StoreError.invalidImageFileName
        }
        return fileName
    }

    private func makeComment(body: String) throws -> ReviewComment {
        let normalized = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { throw StoreError.emptyComment }
        return ReviewComment(
            id: UUID().uuidString,
            body: normalized,
            createdAt: Self.timestamp()
        )
    }

    private func resetReviewsIfNeeded(
        in document: inout ReviewDocument,
        for runID: String?
    ) {
        guard let runID else { return }
        if document.runID != runID {
            document.runID = runID
            document.reviews = [:]
        }
    }

    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return encoder
    }()

    private static let decoder = JSONDecoder()

    private static func timestamp() -> String {
        Date().formatted(.iso8601)
    }
}
