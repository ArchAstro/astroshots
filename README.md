# Astroshots

**Watch your test screenshots as they land.**

Astroshots is a macOS menu-bar app for anyone who captures UI during browser tests, harness runs, or manual QA. Point your tools at a simple folder layout — frames stream in live, flash over your desktop, and stay in one history across every project you work in.

<p align="center">
  <img src="docs/images/hero-overlay.jpg" alt="Astroshots desktop overlays flashing new screenshots above the menu bar" width="900" />
</p>

---

## Why Astroshots

| Problem | What Astroshots does |
|---------|----------------------|
| Screenshots buried in `/tmp` or CI artifacts | Live stream as soon as a PNG is written |
| Jumping between projects to find shots | One tray, every project under your watch folder |
| “Did that step look right?” mid-run | Desktop overlay above all windows |
| Manual folder digging after a suite | Newest-first history with titles from a small manifest |

No project picker. No account. No cloud. Just files on disk and a camera icon in the menu bar.

<p align="center">
  <img src="docs/images/overlay-cards.png" alt="Stacked overlay cards from different projects" width="420" />
</p>

---

## Install

### From a release (recommended)

1. Download the latest **Astroshots.dmg** from [Releases](https://github.com/ArchAstro/astroshots/releases).
2. Open the DMG and drag **Astroshots** into **Applications**.
3. Launch Astroshots — it lives in the **menu bar** (no Dock icon).
4. Click the camera icon → gear → set your **watch root** if you want something other than the default.

Builds are Developer ID signed and notarized so Gatekeeper accepts a normal open.

### From source

macOS 14+, Xcode 16+, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).

```bash
git clone https://github.com/ArchAstro/astroshots.git
cd astroshots/macos
./scripts/bootstrap.sh
open Astroshots.xcodeproj   # ⌘R
```

Details: [`macos/README.md`](macos/README.md).

---

## How it works

1. **Watch** — Astroshots recursively watches a folder you choose (default `~/archastro`) for `.astroshot/` trees.
2. **Write** — Your harness, agent, or script drops PNGs (and an optional `manifest.json`) under that layout.
3. **See** — New frames flash on the desktop; open the tray for the full stream. Click a row for detail.

### The tray

<p align="center">
  <img src="docs/images/tray-stream.png" alt="Astroshots menu-bar tray showing a live multi-project screenshot stream" width="360" />
  &nbsp;
  <img src="docs/images/tray-detail.png" alt="Astroshots detail view for a single screenshot frame" width="360" />
</p>

<p align="center">
  <img src="docs/images/hero-stream.jpg" alt="Astroshots tray open on the desktop stream view" width="900" />
</p>

### Write layout

```text
<project>/.astroshot/<feature>/
  manifest.json          # optional, recommended
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

Project name is inferred from the folder that contains `.astroshot`. Feature is the directory name under it.

---

## Capture helper

Ship screenshots into the layout without hand-editing manifests:

```bash
# From a clone of this repo
export PATH="$PWD/bin:$PATH"

astroshot-capture --feature install-wizard --slug configure \
  --title "Configure" \
  --description "Configuration screen visible." \
  --source ./shot.png

astroshot-capture --feature install-wizard --status pass --finalize
```

Or capture from an `agent-browser` CLI session:

```bash
astroshot-capture --feature install-wizard --slug configure \
  --from-agent-browser "$SESSION"
```

The helper lives at [`bin/astroshot-capture`](bin/astroshot-capture) → [`skills/astroshots/scripts/astroshot-capture`](skills/astroshots/scripts/astroshot-capture).

### Agent skill

Coding agents can load the full contract from [`skills/astroshots/`](skills/astroshots/):

```bash
./skills/install.sh    # links into ~/.claude/skills and ~/.grok/skills
```

---

## Product tour

| Surface | Job |
|---------|-----|
| **Desktop overlay** | New frame above all windows; Open / dismiss |
| **Stream** | Newest-first list across all projects under the watch root |
| **Detail** | Full frame + path, feature, URL, time |
| **Settings** | Watch root, overlay toggles |

Interactive mock (open in a browser): [`docs/mocks/astroshots-menubar.html`](docs/mocks/astroshots-menubar.html).

---

## Releases

| How | What you get |
|-----|----------------|
| [GitHub Releases](https://github.com/ArchAstro/astroshots/releases) | Signed, notarized **DMG** |
| Tag `v*` | CI builds that DMG automatically |

Maintainers: cut a version with **Actions → Cut release** (patch / minor / major). Signing secrets: [`docs/SIGNING.md`](docs/SIGNING.md).

---

## Requirements

- macOS 14 or later  
- A folder of projects to watch (configure in-app)  
- Tools that can write PNG files (any language, any harness)
