# Astroshots (macOS)

Menu-bar app that live-watches harness screenshots under `.astroshot/`, flashes
new frames as desktop overlays, and streams them from every worktree in one list.

Design source: [`../docs/mocks/astroshots-menubar.html`](../docs/mocks/astroshots-menubar.html).

Requires macOS 14+ (Apple Silicon for narration), Xcode 26+,
[XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).

Narration pulls **mlx-audio-swift** (→ **mlx-swift**). That stack needs:

- **Swift tools 6.2+** (Xcode 26+; GitHub Actions uses `macos-26`)
- **Metal toolchain** for mlx-swift shaders
- **`-skipPackagePluginValidation`** (mlx CUDA plugin fingerprint on macOS CLI)

```bash
xcodebuild -downloadComponent MetalToolchain   # once per machine
# every xcodebuild:
xcodebuild … -skipPackagePluginValidation
```

`./scripts/bootstrap.sh` and `scripts/xcode-env.sh` handle the Metal/flags side.

## Getting started

```bash
./scripts/bootstrap.sh      # Metal prereqs + xcodegen → Astroshots.xcodeproj
open Astroshots.xcodeproj
```

Or from the CLI:

```bash
source scripts/xcode-env.sh && ensure_mlx_build_prereqs
xcodegen generate
xcodebuild -project Astroshots.xcodeproj -scheme Astroshots \
  -destination 'platform=macOS' "${ASTROSHOTS_XCODEBUILD_FLAGS[@]}" build
xcodebuild -project Astroshots.xcodeproj -scheme Astroshots \
  -destination 'platform=macOS' "${ASTROSHOTS_XCODEBUILD_FLAGS[@]}" test
```

## Behavior

- **Watch roots** (required first-run choice; no automatic default root):
  recursive scan + FSEvents for `**/.astroshot/<feature>/*.{png,jpg,…}` and
  `manifest.json`. The folder open panel starts in `~/Projects` when present.
  Stored in the app's preferences domain under two keys with a required
  precedence; external readers must follow
  [`docs/PREFERENCES.md`](../docs/PREFERENCES.md).
- **No per-project picker**: every project under the watched roots streams into
  one newest-first list. Project name is a badge on each row / overlay.
- **Desktop overlay**: new frames float above all windows; clicking anywhere on
  a card opens its full-screen review.
- **Tray**: stream → click for detail → gear for settings. Pin keeps a floating window.
- **Review**: clicking a screenshot opens a chromeless, screen-sized takeover
  with a dim gray stage, close control, comment history, feedback composer, and
  Seen acknowledgement. Feedback is saved beside the execution manifest so an
  agent can read it without an app-specific API.

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
`idle` describes execution, not whether a human has seen the image. Astroshots
stores human acknowledgement and feedback separately in `review.json`:

```json
{
  "version": 1,
  "run_id": "install-wizard-…",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0002-configure.png": {
      "decision": "seen",
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

Review entries use exact image filenames. Seen is current only when its
`image_sha256` matches the file's SHA-256. If a harness or agent replaces the
image, Astroshots treats it as unseen while preserving
comments as agent-readable feedback. A comment may exist without a decision;
such unseen entries may omit `reviewed_at` and `image_sha256`.

When a manifest supplies `run_id`, Astroshots only applies feedback from a
`review.json` with the same run id. A missing or different review run is
unseen, and its prior acknowledgement and comments are not shown for the current run.

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

The packaged volume uses the Astroshots visor artwork, a fixed Finder window,
and positioned **Astroshots → Applications** icons. The source artwork lives in
`Design/Generated/`; packaging fails rather than silently shipping an unbranded
fallback when the background is missing. Run `./scripts/test-package-dmg.sh`
for the canonical end-to-end proof that builds, packages, mounts, and verifies
the same installer contract before release.

| Workflow | When | Output |
|----------|------|--------|
| `.github/workflows/ci.yml` | PRs / pushes | Tests + ad-hoc branded DMG proof |
| `.github/workflows/cut-release.yml` | Manual: patch / minor / major | Bump + changelog + tag `vX.Y.Z` + **dispatch Release DMG** + PR |
| `.github/workflows/cut-npm-release.yml` | Manual: patch / minor / major | Bump npm packages + changelog + tag `astroshot-v*` + publish dispatch + PR |
| `.github/workflows/release-dmg.yml` | tags `v*` or dispatch | Signed + notarized `Astroshots-X.Y.Z.dmg`, Sparkle zip, and `appcast.xml` on the GitHub Release |
| `.github/workflows/publish-npm.yml` | tags `astroshot-v*` or dispatch | Publish `@archastro/*` with OIDC trusted publishing |

### Auto-update (Sparkle)

The app embeds [Sparkle 2](https://sparkle-project.org):

- Feed: `https://github.com/ArchAstro/astroshots/releases/download/appcast/appcast.xml`
  (dedicated release — not GitHub “Latest”, which is often the npm track)
- Public key: `SUPublicEDKey` in `project.yml`
- UI: status-item **Check for Updates…** + Settings → Updates

```bash
./scripts/package-dmg.sh
# → build/Astroshots-X.Y.Z.dmg  (manual install)
# → build/Astroshots-X.Y.Z.zip  (in-app update archive)

./scripts/generate-appcast.sh \
  --zip build/Astroshots-X.Y.Z.zip \
  --version X.Y.Z \
  --out ../appcast.xml
```

CI secret `SPARKLE_ED_PRIVATE_KEY` and key rotation: [`docs/SIGNING.md`](../docs/SIGNING.md) step **10b**.

#### Watching upgrade logs (release testing)

Every Sparkle stage is written to the unified system log:

| Field | Value |
|-------|--------|
| subsystem | `ai.archastro.Astroshots` |
| category | `SoftwareUpdate` |

```bash
# Live stream while you cut/install successive versions:
log stream --style compact --predicate \
  'subsystem == "ai.archastro.Astroshots" AND category == "SoftwareUpdate"'
```

In **Console.app**: search `subsystem:ai.archastro.Astroshots category:SoftwareUpdate`
(include Info/Debug; “Include Info Messages”).

Typical lines for a successful upgrade:

1. `startup installed=… feed=…`
2. `mayPerformCheck type=user-initiated|background`
3. `appcastLoaded itemCount=…`
4. `foundUpdate 0.2.1 [18] …` **or** `noUpdate …`
5. `willDownload` → `didDownload` → `willExtract` → `didExtract`
6. `willInstall` → `willRelaunch` → `cycleFinished … ok`

Errors surface as `downloadFailed`, `aborted`, or `cycleFinished … error=`.

```bash
# macOS app
gh workflow run "Cut release" --repo ArchAstro/astroshots -f bump=patch

# npm packages (separate version track)
gh workflow run "Cut npm release" --repo ArchAstro/astroshots -f bump=patch
```

User-facing notes: put them under `## [Unreleased]` in the repo-root
[`CHANGELOG.md`](../CHANGELOG.md) before cutting either track.
