import AVKit
import Foundation
import Testing
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

    @Test func bindsMovieFieldsFromManifest() throws {
        let json = """
        {
          "version": 1,
          "feature": "tray-movie",
          "run_id": "run-movie",
          "status": "pass",
          "shots": [
            {
              "id": "0001",
              "file": "0001-tray-journey.png",
              "slug": "tray-journey",
              "title": "Tray Journey",
              "description": "Native tray walkthrough",
              "kind": "movie",
              "video": "0001-tray-journey.webm",
              "duration_ms": 2750,
              "source": "desktop.window"
            }
          ]
        }
        """
        let manifest = try JSONDecoder().decode(FeatureManifest.self, from: Data(json.utf8))
        let meta = ManifestParser.shotMetadata(
            fileName: "0001-tray-journey.png",
            manifest: manifest
        )
        #expect(meta.kind == "movie")
        #expect(meta.video == "0001-tray-journey.webm")
        #expect(meta.durationMs == 2750)
        #expect(meta.source == "desktop.window")
        #expect(meta.title == "Tray Journey")
    }
}

struct ShotMovieMetadataTests {
    @MainActor
    @Test func nativePlayerExposesInlinePlaybackControls() {
        let playerView = MoviePlayerView.configuredAVPlayerView()

        #expect(playerView.controlsStyle == .inline)
        #expect(playerView.showsFullScreenToggleButton)
        #expect(playerView.showsFrameSteppingButtons)
        #expect(MoviePlayerView.backend(for: "/tmp/movie.mp4") == .avKit)
        #expect(MoviePlayerView.backend(for: "/tmp/movie.webm") == .webKit)
    }

    @Test func isMovieFromKindAndDurationLabel() {
        let movie = Shot(
            path: "/tmp/wt/.astroshot/f/0001-journey.png",
            worktree: "wt",
            worktreePath: "/tmp/wt",
            feature: "f",
            fileName: "0001-journey.png",
            sequence: "0001",
            slug: "journey",
            title: "Journey",
            description: "desc",
            url: nil,
            runID: "r1",
            status: .pass,
            capturedAt: Date(),
            kind: "movie",
            videoFileName: "0001-journey.webm",
            durationMs: 2750,
            movieSource: "browser"
        )
        #expect(movie.isMovie)
        #expect(movie.durationLabel == "2.8s")

        let still = Shot(
            path: "/tmp/wt/.astroshot/f/0002-still.png",
            worktree: "wt",
            worktreePath: "/tmp/wt",
            feature: "f",
            fileName: "0002-still.png",
            sequence: "0002",
            slug: "still",
            title: "Still",
            description: "",
            url: nil,
            runID: nil,
            status: nil,
            capturedAt: Date()
        )
        #expect(!still.isMovie)
        #expect(still.durationLabel == nil)
    }
}
