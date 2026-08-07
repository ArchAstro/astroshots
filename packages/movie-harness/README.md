# `@archastro/movie-harness`

Universal **movie** capture harness for Astroshots. One session API, many
sources, one `.astroshot/` sink (poster PNG + video + manifest).

Also exposed as **`astroshot movie`** from `@archastro/astroshot` (same binary
agents already install for stills).

| Source | Status | Notes |
|--------|--------|-------|
| `frames` | **ready** | Push any PNG/JPEG sequence (multi-process CLI) |
| `browser` | **ready** | Playwright `recordVideo` (headless OK) |
| `pty` | **ready** | Truecolor SGR → xterm → Chromium samples |
| `pty-demo` | **ready** | Truecolor smoke without a real program |
| `desktop.window` | **ready** (macOS) | OS `screencapture -l` + shipped Swift window list |
| `desktop.display` | stub | Use `desktop.window` or `frames` for now |

## Which source? (for agents)

```bash
astroshot movie which-source "record a ratatui TUI with truecolor"
astroshot movie which-source "SwiftUI onboarding window"
astroshot movie help-sources
astroshot movie --help          # full decision table
```

Hard rules:

1. **TUI/CLI color** → `pty` (never `desktop.window` on Terminal/iTerm/Ghostty)
2. **Web** → `browser` (not a desktop grab of Chrome)
3. **Native Mac app** → `desktop.window` (`--bundle-id` / `--window-id`)
4. **Custom pixels** → `frames`

## Why

Stills already stream through `.astroshot/`. Journeys need movies. Browser and
PTY are off-screen (headless Chromium). Native desktop is a separate adapter
behind the same session contract.

**PTY color is sacred:** `TERM=xterm-256color`, `COLORTERM=truecolor`, cell paint
via xterm (RGB / 256 / 16). Never screenshot Terminal.app.

## CLI

```bash
# From monorepo after build:
node packages/movie-harness/bin/astroshot-movie.mjs --help

# Synthetic frames demo → .astroshot/demo-journey/
astroshot-movie run --source frames --feature demo-journey --slug flow

# Browser journey
astroshot-movie run --source browser --feature web --slug home \
  --url https://example.com --settle-ms 500

# Truecolor terminal smoke (no fixture program)
astroshot movie run --source pty-demo --feature tui --slug brand

# Real PTY fixture
astroshot movie run --source pty --feature tui --slug flow \
  --fixture ./fixtures/demo.pty.yaml

# Native macOS window (uses /usr/sbin/screencapture — already on the Mac)
astroshot movie list-windows
astroshot movie run --source desktop.window --feature app --slug onboard \
  --bundle-id com.example.App --duration-ms 4000 --fps 10

# Multi-process frames
astroshot movie start --feature custom --slug walk
astroshot movie push-frame --feature custom --file ./frame.png
astroshot movie mark --feature custom --slug step-2
astroshot movie stop --feature custom --status pass
```

`desktop.window` needs **Screen Recording** permission for your terminal/IDE
(System Settings → Privacy & Security). Window listing uses Swift
(`native/macos/WindowTools.swift`) shipped in the package — no extra download.

## Library

```ts
import { MovieSession, encodeSolidPng, recordBrowserMovie } from "@archastro/movie-harness";

const session = MovieSession.create({
  feature: "install-wizard",
  slug: "journey",
  source: "frames",
  size: { width: 1280, height: 720 },
  fps: 15,
});
session.pushFrame(encodeSolidPng(1280, 720, [124, 92, 255]));
session.mark("purple");
const artifact = await session.stop({ status: "pass" });
// artifact.posterPath, artifact.videoPath under .astroshot/
```

## Write contract

```text
.astroshot/<feature>/
  manifest.json
  0001-journey.png     # poster (tray / overlay today)
  0001-journey.webm    # movie
```

Manifest shot fields (extension):

```json
{
  "kind": "movie",
  "file": "0001-journey.png",
  "video": "0001-journey.webm",
  "duration_ms": 1200,
  "source": "browser",
  "chapters": [{ "slug": "mid", "t_ms": 400 }]
}
```

Phase 0: Astroshots app needs no changes (poster is a normal image).  
Phase 1: open `video` from detail when present.

## Encode

1. **ffmpeg** if on `PATH` (WebM VP9 or MP4 H.264)
2. Else **Playwright** slideshow → `recordVideo` WebM (no extra native deps)

## Desktop window (macOS)

Uses tooling already on the machine:

1. **Window list** — shipped `WindowTools.swift` via `swift` (Xcode CLT)
2. **Frames** — `/usr/sbin/screencapture -l <windowId>`
3. **Encode** — same Playwright/ffmpeg path as other sources

```bash
astroshot movie run --source desktop.window --feature app --slug flow \
  --bundle-id ai.archastro.Astroshots --duration-ms 3000
```
