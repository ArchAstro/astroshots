# Astroshots (macOS)

Menu-bar app that live-watches harness screenshots under `.astroshot/`, flashes
new frames as desktop overlays, and streams them from every worktree in one list.

Design source: [`../docs/mocks/astroshots-menubar.html`](../docs/mocks/astroshots-menubar.html).

Requires macOS 14+, Xcode 16+, [XcodeGen](https://github.com/yonaskolb/XcodeGen)
(`brew install xcodegen`).

## Getting started

```bash
./scripts/bootstrap.sh      # xcodegen generate → Astroshots.xcodeproj
open Astroshots.xcodeproj
```

Or from the CLI:

```bash
xcodegen generate
xcodebuild -project Astroshots.xcodeproj -scheme Astroshots -destination 'platform=macOS' build
xcodebuild -project Astroshots.xcodeproj -scheme Astroshots -destination 'platform=macOS' test
```

## Behavior

- **Watch root** (configurable; default `~/archastro`): recursive scan + FSEvents
  for `**/.astroshot/<feature>/*.{png,jpg,…}` and `manifest.json`.
- **No project picker**: every project under the watch root streams into one
  newest-first list. Project name is a badge on each row / overlay.
- **Desktop overlay**: new frames float above all windows; Open jumps to detail.
- **Tray**: stream → click for detail → gear for settings. Pin keeps a floating window.
- **Review**: clicking a screenshot opens a chromeless, screen-sized takeover
  with a dim gray stage, close control, comment history, and approve/request
  changes actions. Feedback is saved beside the execution manifest so an agent
  can read it without an app-specific API.

The unsigned `Astroshots` scheme contains the focused model and storage tests.
The real-window review proof is isolated in `AstroshotsReviewUITests` because
macOS UI automation requires a local development signing team:

```bash
xcodebuild \
  -project Astroshots.xcodeproj \
  -scheme AstroshotsReviewUITests \
  -destination 'platform=macOS' \
  DEVELOPMENT_TEAM=YOUR_TEAM_ID \
  test
```

## Harness layout

```
<worktree>/.astroshot/<feature>/
  manifest.json          # optional harness execution state
  review.json            # optional human review state
  0001-signed-in.png
  0002-configure.png
```

Example `manifest.json`:

```json
{
  "version": 1,
  "feature": "install-wizard",
  "run_id": "install-wizard-…",
  "status": "running",
  "shots": [
    {
      "id": "0002",
      "file": "0002-configure.png",
      "slug": "configure",
      "title": "Configure",
      "description": "Configuration screen for the resource.",
      "url": "/solutions · dialog"
    }
  ]
}
```

`manifest.json.status` belongs to the harness: `running`, `pass`, `fail`, or
`idle` describes execution, not human acceptance. Astroshots stores human
feedback separately in `review.json`:

```json
{
  "version": 1,
  "run_id": "install-wizard-…",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0002-configure.png": {
      "decision": "approved",
      "reviewed_at": "2026-07-26T17:42:00Z",
      "image_sha256": "a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1",
      "comments": [
        {
          "id": "A1B2C3D4-E5F6-47A8-9000-111122223333",
          "body": "The final action is now fully visible.",
          "created_at": "2026-07-26T17:41:32Z"
        }
      ]
    }
  }
}
```

Review entries use exact image filenames. A decision is current only when its
`image_sha256` matches the file's SHA-256. If a harness or agent replaces the
image, Astroshots treats the prior decision as stale/pending while preserving
comments as agent-readable feedback. A comment may exist without a decision;
such pending entries may omit `reviewed_at` and `image_sha256`.

When a manifest supplies `run_id`, Astroshots only applies feedback from a
`review.json` with the same run id. A missing or different review run is
pending, and its prior decision and comments are not shown for the current run.

## Architecture

```
Astroshots/
  AstroshotsApp.swift       App entry (status item via AppDelegate)
  App/StatusItemController  Left-click popover, right-click Quit
  Watch/ShotIndexCache      Persisted .astroshot dir index for fast warm start
  App/                      AppState, Preferences, Theme
  Models/                   Shot, Manifest
  Watch/                    AstroshotWatcher (scan + FSEvents)
  Features/
    Tray/                   Stream, Detail, Settings, thumbnails
    Overlay/                NSPanel stack above all windows
```

App sandbox is off so FSEvents can watch developer worktrees without a
bookmark dance. This is intentional for a local tooling app.

## DMG packaging (Gatekeeper-clean)

CI and local releases **Developer ID sign + notarize** the DMG so users can
double-click install without right-click → Open.

**Setup secrets once:** see [`docs/SIGNING.md`](../docs/SIGNING.md).

```bash
export CODE_SIGN_IDENTITY="Developer ID Application: … (TEAMID)"
export DEVELOPMENT_TEAM=TEAMID
export APPLE_API_KEY_PATH=~/Downloads/AuthKey_XXX.p8
export APPLE_API_KEY_ID=…
export APPLE_API_ISSUER_ID=…

./scripts/bootstrap.sh
./scripts/package-dmg.sh
# → build/Astroshots.dmg  (signed, notarized, stapled)
```

Dev-only (Gatekeeper will warn): `./scripts/package-dmg.sh --adhoc`

| Workflow | When | Output |
|----------|------|--------|
| `.github/workflows/ci.yml` | PRs / pushes | Tests only (no DMG) |
| `.github/workflows/cut-release.yml` | Manual: patch / minor / major | `release/vX.Y.Z` branch + tag + PR to main |
| `.github/workflows/release-dmg.yml` | tags `v*` | Signed + notarized DMG on the GitHub Release |

```bash
gh workflow run "Cut release" --repo ArchAstro/astroshots -f bump=patch
```
