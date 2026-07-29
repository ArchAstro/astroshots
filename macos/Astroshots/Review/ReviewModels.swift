import Foundation

enum ReviewDecision: String, Codable, Sendable, Hashable {
    case seen
    // Kept for decoding review files written by Astroshots 0.1.x.
    case approved
    case changesRequested = "changes_requested"
}

enum ReviewState: Sendable, Hashable {
    case pending
    case seen
}

struct ReviewComment: Codable, Identifiable, Sendable, Hashable {
    let id: String
    let body: String
    let createdAt: String

    enum CodingKeys: String, CodingKey {
        case id, body
        case createdAt = "created_at"
    }
}

struct ShotReview: Codable, Sendable, Hashable {
    var decision: ReviewDecision?
    var comments: [ReviewComment]
    var reviewedAt: String?
    var imageSHA256: String?

    init(
        decision: ReviewDecision? = nil,
        comments: [ReviewComment] = [],
        reviewedAt: String? = nil,
        imageSHA256: String? = nil
    ) {
        self.decision = decision
        self.comments = comments
        self.reviewedAt = reviewedAt
        self.imageSHA256 = imageSHA256
    }

    enum CodingKeys: String, CodingKey {
        case decision, comments
        case reviewedAt = "reviewed_at"
        case imageSHA256 = "image_sha256"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        decision = try container.decodeIfPresent(ReviewDecision.self, forKey: .decision)
        comments = try container.decodeIfPresent([ReviewComment].self, forKey: .comments) ?? []
        reviewedAt = try container.decodeIfPresent(String.self, forKey: .reviewedAt)
        imageSHA256 = try container.decodeIfPresent(String.self, forKey: .imageSHA256)
    }
}

struct ReviewSnapshot: Sendable, Hashable {
    let state: ReviewState
    let comments: [ReviewComment]
    let isStale: Bool
    let review: ShotReview?

    init(review: ShotReview?, currentImageSHA256: String) {
        let hashMatches = review?.imageSHA256 == currentImageSHA256
        let effectiveDecision = hashMatches ? review?.decision : nil

        comments = review?.comments ?? []
        isStale = review?.decision != nil && !hashMatches
        self.review = review

        switch effectiveDecision {
        case .seen, .approved:
            state = .seen
        case .changesRequested:
            state = .pending
        case nil:
            state = .pending
        }
    }
}

struct ReviewDocument: Codable, Sendable, Hashable {
    static let currentVersion = 1

    var version: Int
    var runID: String?
    var updatedAt: String?
    var reviews: [String: ShotReview]

    init(
        version: Int = currentVersion,
        runID: String? = nil,
        updatedAt: String? = nil,
        reviews: [String: ShotReview] = [:]
    ) {
        self.version = version
        self.runID = runID
        self.updatedAt = updatedAt
        self.reviews = reviews
    }

    enum CodingKeys: String, CodingKey {
        case version, reviews
        case runID = "run_id"
        case updatedAt = "updated_at"
    }
}
