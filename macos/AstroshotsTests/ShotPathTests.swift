import Testing
import Foundation
@testable import Astroshots

struct ShotPathTests {
    @Test func parsesStandardAstroshotPath() {
        let path = "/Users/calvin/archastro/firstlanding-wt4/.astroshot/install-wizard/0004-configure.png"
        let parts = ShotPath.parse(imagePath: path)
        #expect(parts?.worktree == "firstlanding-wt4")
        #expect(parts?.feature == "install-wizard")
        #expect(parts?.fileName == "0004-configure.png")
        #expect(parts?.worktreePath.hasSuffix("firstlanding-wt4") == true)
    }

    @Test func rejectsNonImage() {
        let path = "/tmp/proj/.astroshot/feat/readme.txt"
        #expect(ShotPath.parse(imagePath: path) == nil)
    }

    @Test func rejectsMissingAstroshotSegment() {
        let path = "/tmp/proj/screenshots/0001.png"
        #expect(ShotPath.parse(imagePath: path) == nil)
    }

    @Test func sequenceAndSlug() {
        let (seq, slug) = ShotPath.sequenceAndSlug(fileName: "0004-new-network-with-org-access.png")
        #expect(seq == "0004")
        #expect(slug == "new-network-with-org-access")
    }

    @Test func sequenceAndSlugWithoutNumber() {
        let (seq, slug) = ShotPath.sequenceAndSlug(fileName: "hero.png")
        #expect(seq == nil)
        #expect(slug == "hero")
    }

    @Test func worktreeShortExtractsWt() {
        let shot = Shot(
            path: "/x/firstlanding-wt4/.astroshot/f/a.png",
            worktree: "firstlanding-wt4",
            worktreePath: "/x/firstlanding-wt4",
            feature: "f",
            fileName: "a.png",
            sequence: nil,
            slug: "a",
            title: "A",
            description: "",
            url: nil,
            runID: nil,
            status: nil,
            capturedAt: Date()
        )
        #expect(shot.worktreeShort == "wt4")
    }
}
