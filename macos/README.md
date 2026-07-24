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

## DMG packaging

Local (same script CI uses):

```bash
./scripts/bootstrap.sh
./scripts/package-dmg.sh
# → build/Astroshots.dmg  and  build/Astroshots-<version>-<build>.dmg
```

### CI

| Workflow | When | Output |
|----------|------|--------|
| `.github/workflows/ci.yml` | PRs / pushes | **Astroshots-dmg** artifact (14 days) |
| `.github/workflows/release-dmg.yml` | tags `v*` | DMG attached to the GitHub Release |

Default signing is **ad-hoc** (`CODE_SIGN_IDENTITY=-`) so no Apple secrets are
required. First launch of an ad-hoc build on another Mac may need
right-click → **Open** (Gatekeeper).

### Optional Developer ID (later)

Repo secrets (when you have a Developer ID Application cert):

| Secret | Example |
|--------|---------|
| `MACOS_CODE_SIGN_IDENTITY` | `Developer ID Application: ArchAstro Inc (TEAMID)` |
| `MACOS_DEVELOPMENT_TEAM` | `TEAMID` |

Notarization (staple + `notarytool`) is not wired yet; ad-hoc DMGs are enough
for internal review.
