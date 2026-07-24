import Testing
import Foundation
@testable import Astroshots

struct ManifestParserTests {
    @Test func bindsShotFromManifestFileName() throws {
        let json = """
        {
          "version": 1,
          "feature": "install-wizard",
          "run_id": "run-1",
          "status": "running",
          "shots": [
            {
              "id": "0004",
              "file": "0004-new-network.png",
              "slug": "new-network",
              "title": "New network + ACL",
              "description": "ACL selected.",
              "url": "/solutions"
            }
          ]
        }
        """
        let manifest = try JSONDecoder().decode(FeatureManifest.self, from: Data(json.utf8))
        let meta = ManifestParser.shotMetadata(fileName: "0004-new-network.png", manifest: manifest)
        #expect(meta.sequence == "0004")
        #expect(meta.slug == "new-network")
        #expect(meta.title == "New network + ACL")
        #expect(meta.description == "ACL selected.")
        #expect(meta.url == "/solutions")
        #expect(meta.runID == "run-1")
        #expect(meta.status == .running)
    }

    @Test func fallsBackWhenNoManifest() {
        let meta = ManifestParser.shotMetadata(fileName: "0002-configure.png", manifest: nil)
        #expect(meta.sequence == "0002")
        #expect(meta.slug == "configure")
        #expect(meta.title == "Configure")
        #expect(meta.description.isEmpty)
    }

    @Test func statusAliases() {
        #expect(FeatureStatus(raw: "PASS") == .pass)
        #expect(FeatureStatus(raw: "failed") == .fail)
        #expect(FeatureStatus(raw: "in_progress") == .running)
    }
}
