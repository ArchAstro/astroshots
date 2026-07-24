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

- **Watch root** (default `~/archastro`): recursive scan + FSEvents for
  `**/.astroshot/<feature>/*.{png,jpg,…}` and `manifest.json`.
- **No worktree picker**: every tree streams into one newest-first list.
  Worktree is a badge on each row / overlay.
- **Desktop overlay**: new frames float above all windows; Open jumps to detail.
- **Tray**: stream → click for detail → gear for settings. Pin keeps a floating window.

## Harness layout

```
<worktree>/.astroshot/<feature>/
  manifest.json          # optional but recommended
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

## Architecture

```
Astroshots/
  AstroshotsApp.swift       MenuBarExtra + pinned Window
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
