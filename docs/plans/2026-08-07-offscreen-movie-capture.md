# Universal movie harness for Astroshots

**Branch / worktree:** `features/calvin-archastro-07-08-2026-movie-capture`  
**Path:** `/Users/calvin/archastro/astroshots-movie-capture`  
**Status:** implemented (v0) in `packages/movie-harness`  
**Goal:** one harness that records a **movie in any context** — browser, PTY, native desktop window/display, or any frame stream — and lands it in the same `.astroshot/` review path.

| Source | Status |
|--------|--------|
| `frames` | done — multi-process CLI + `MovieSession` |
| `browser` | done — Playwright `recordVideo` |
| `pty` / `pty-demo` | done — truecolor SGR → xterm → samples |
| `desktop.window` | done (macOS) — `screencapture -l` + shipped `WindowTools.swift` |
| `desktop.display` / `region` | stub |

Also: **`astroshot movie`** on the main `@archastro/astroshot` CLI, plus
`which-source` / decision-table help for agents.

Demo: `bash scripts/demo-movie-harness.sh`

---

## 0. Product shape (the thing you want)

```
                    ┌──────────────────────────────────────┐
  harness journey   │         MovieSession (one API)       │
  start / stop      │  start → mark → chapter → stop       │
                    └──────────────┬───────────────────────┘
                                   │ frames (timed bitmaps)
           ┌───────────────────────┼───────────────────────┐
           ▼                       ▼                       ▼
    ┌─────────────┐        ┌─────────────┐        ┌─────────────────┐
    │  browser    │        │    pty      │        │  desktop        │
    │  Playwright │        │  SGR→xterm  │        │  ScreenCapture  │
    │  / CDP      │        │  truecolor  │        │  Kit (window)   │
    └─────────────┘        └─────────────┘        └─────────────────┘
           │                       │                       │
           └───────────────────────┼───────────────────────┘
                                   ▼
                    ┌──────────────────────────────────────┐
                    │  Encoder + sink                      │
                    │  .webm/.mp4 + poster.png + manifest  │
                    │  → .astroshot/<feature>/             │
                    └──────────────────────────────────────┘
```

**One session. Many sources. One write contract.**  
Callers never care whether the pixels came from Chromium, a PTY, or a native window — only that a movie + poster land under `.astroshot/`.

This is the same idea as `astroshot-capture` for stills, generalized to **time**.

---

## 1. What we have today (stills only)

| Source | Frame origin | Output |
|--------|--------------|--------|
| `react-shot` | Headless Chromium screenshot | PNG |
| `ink` / tui-shot | ANSI → xterm → HTML → Chromium | PNG |
| `pty` | node-pty → final xterm buffer → same | PNG |
| agent-browser harness | Live session → `astroshot-capture` | PNG |
| Astroshots app | FSEvents on image extensions | overlay + tray |

No movie path yet. No native-desktop capture path yet.

---

## 2. Design principles

1. **Source-agnostic session** — `MovieSession` owns timing, chapters, encode, sink. Sources only produce frames (or attach a recorder that does).
2. **Best surface per context** — do not force everything through OS screen capture:
   - Browser → Playwright / CDP (no TCC, headless OK)
   - PTY → xterm truecolor path (color is sacred; never OS terminal theme)
   - Native → ScreenCaptureKit window/display (TCC required)
   - Escape hatch → push raw frames from anything
3. **Same review contract as stills** — always write poster PNG so today’s Astroshots app works; video is additive.
4. **Harness-first API** — Bash / Node / agent skills call one helper; modes are adapters, not separate products.
5. **Color fidelity where it matters** — PTY truecolor is non-negotiable; native captures real pixels (including app chrome).

---

## 3. Core API (conceptual)

```ts
// packages/movie-harness (name TBD) — the universal session

type MovieSource =
  | { kind: "browser"; page: Page; mode?: "recordVideo" | "screencast" }
  | { kind: "pty"; session: PtyMovieHandle }       // SGR → xterm → pixels
  | { kind: "desktop.window"; match: WindowMatch } // ScreenCaptureKit
  | { kind: "desktop.display"; displayId?: number }
  | { kind: "desktop.region"; rect: Rect; displayId?: number }
  | { kind: "frames" };                           // push yourself

interface MovieSession {
  start(opts: {
    source: MovieSource;
    feature: string;
    slug: string;
    root?: string;          // worktree → .astroshot/
    runId?: string;
    size?: { width: number; height: number };
    fps?: number;           // default 15 for review; 30 for native polish
    format?: "webm" | "mp4";
  }): Promise<void>;

  /** Optional chapter for scrubbing / manifest markers */
  mark(slug: string, note?: string): void;

  /** Only for kind: "frames" — or mid-run stills from any source */
  pushFrame(pngOrJpeg: Buffer, tsMs?: number): void;

  /** Grab a still now (also used as poster candidate) */
  snapshot(): Promise<Buffer>;

  stop(): Promise<MovieArtifact>;
}

interface MovieArtifact {
  videoPath: string;    // .astroshot/<feature>/0001-slug.webm
  posterPath: string;   // .astroshot/<feature>/0001-slug.png
  durationMs: number;
  chapters: { slug: string; tMs: number; note?: string }[];
  source: MovieSource["kind"];
}
```

### CLI / harness helper (stills-compatible)

```bash
# browser (agent-browser session)
astroshot-movie start --feature install --slug walk --source browser --session "$SESSION"
# … drive UI …
astroshot-movie stop  --feature install --run-id "$RUN_ID"

# PTY fixture
astroshot-movie run --feature tui-demo --slug flow \
  --source pty --fixture ./demo.pty.yaml

# native macOS window (bundle id or title regex)
astroshot-movie start --feature macos-app --slug onboard \
  --source desktop.window --bundle-id ai.archastro.Astroshots
# … click through app …
astroshot-movie stop

# any process that can dump frames
astroshot-movie start --feature custom --slug x --source frames --size 1280x720
while …; do astroshot-movie push-frame --file /tmp/f.png; done
astroshot-movie stop
```

Bash harness pattern (mirrors current still dual-write):

```bash
FEATURE=install-wizard
RUN_ID="${FEATURE}-$(date -u +%Y%m%dT%H%M%SZ)-$$"

astroshot-movie start --feature "$FEATURE" --slug journey \
  --source browser --from-agent-browser "$SESSION" --run-id "$RUN_ID"

# drive the journey; optional chapter marks:
astroshot-movie mark --slug "signed-in"
astroshot-movie mark --slug "configure"

astroshot-movie stop --status pass --finalize
# → .astroshot/install-wizard/0001-journey.webm
# → .astroshot/install-wizard/0001-journey.png
# → manifest entry with kind: "movie", video, poster, chapters
```

---

## 4. Source adapters

### 4.1 Browser (`browser`)

| Mechanism | When |
|-----------|------|
| Playwright `recordVideo` | Whole journey, simplest, finalize on context close |
| CDP `Page.startScreencast` | Live mid-run frames into tray; custom fps |
| `page.screenshot` loop | Fallback only |

- Headless OK; **no** Screen Recording permission.
- Works for agent-browser, react journeys, any Playwright page.
- Viewport-bound: pin size; poster = last or marked frame.

### 4.2 PTY (`pty`) — color-critical

```
node-pty (TERM=xterm-256color, COLORTERM=truecolor)
  → raw SGR (keep truecolor)
  → @xterm/headless or live xterm.js
  → colored cells → Chromium surface → encode
```

| Sub-mode | Role |
|----------|------|
| Live xterm page + recordVideo | Default product “session movie” |
| Sample `terminalToHtml` → PNG seq | Color-correct spike / high fidelity |
| Tape (raw SGR timestamps) → offline render | Deterministic CI; re-render later |

**Hard rules:** never OS-terminal capture for PTY; never strip SGR; poster from same path as still `astroshot pty`; assert truecolor fixture `#7c5cff`.

### 4.3 Native desktop (`desktop.*`)

macOS path: **ScreenCaptureKit** (`SCStream` + `SCContentFilter`).

| Sub-source | Filter | Notes |
|------------|--------|-------|
| `desktop.window` | `SCContentFilter(desktopIndependentWindow:)` | Follows window across displays; best for app review |
| `desktop.display` | display filter | Full monitor; exclude harness UI to avoid hall-of-mirrors |
| `desktop.region` | display + crop | Rare; prefer window |

**Requirements**

- Screen Recording TCC (user grants once; CI machines need pre-grant / MDM)
- Running window: match by `bundle id`, PID, title regex, or CGWindowID
- Minimized / fully occluded: capture quality varies — prefer keep window on a space (can be covered or off to the side; independent-window filter still tracks it)

**Not the same as “headless Chromium.”** Native apps need a real window server surface. Options when you want “off screen”:

| Technique | Reality |
|-----------|---------|
| Window moved off visible area | Often still capturable via window filter |
| Separate Space / full-screen space | Works; user may not see it |
| Virtual display | Possible but heavy (CI complexity) |
| Linux CI | Xvfb / Wayland virtual output + ffmpeg x11grab |
| True headless AppKit | Generally **not** available; don’t promise it |

**Color:** native = real framebuffer pixels (correct for SwiftUI/AppKit chrome). No SGR model. High bitrate encode so gradients don’t band.

**Linux/Windows later:** same `MovieSession` API; adapters = PipeWire / DXGI / ffmpeg grab. macOS first.

### 4.4 Arbitrary frames (`frames`)

For anything else (Unity, games, custom toolkits, remote VMs):

```ts
session.pushFrame(pngBuffer, performance.now());
```

Harness only timestamps + encodes + sinks. Lowest common denominator; any context that can produce bitmaps is supported.

### 4.5 Adapter comparison

| Source | Determinism | Permissions | Color model | Headless CI |
|--------|-------------|-------------|-------------|-------------|
| browser | High | None | Browser CSS | Yes |
| pty | High | None | SGR truecolor via xterm | Yes |
| desktop.window | Medium | Screen Recording | Real pixels | Hard (need display + grant) |
| desktop.display | Low | Screen Recording | Real pixels | Hard |
| frames | Depends on producer | Depends | Raw bitmaps | Yes if producer is |

---

## 5. Write contract (sink)

```text
.astroshot/<feature>/
  manifest.json
  review.json                 # human feedback (existing)
  0001-journey.png            # poster (required — tray/overlay today)
  0001-journey.webm           # or .mp4
```

Manifest shot entry extension:

```json
{
  "id": "0001",
  "file": "0001-journey.png",
  "slug": "journey",
  "title": "Install journey",
  "kind": "movie",
  "video": "0001-journey.webm",
  "duration_ms": 14200,
  "source": "browser",
  "chapters": [
    { "slug": "signed-in", "t_ms": 1200 },
    { "slug": "configure", "t_ms": 5400 }
  ]
}
```

| Phase | App |
|-------|-----|
| **0** | No app change — poster PNG streams; video is sidecar for humans/agents |
| **1** | Detail “Open movie” if `video` present |
| **2** | First-class video in tray + `AVPlayer` scrub + chapter markers |

Review hash: prefer **video bytes** when deciding the movie; poster can be re-rendered without invalidating if we key on video (decide in Phase 1).

---

## 6. Package / repo layout (proposed)

```
packages/
  movie-harness/          # NEW — session API, encode, sink, CLI
    src/
      session.ts
      sources/
        browser.ts
        pty.ts
        desktop-macos.ts  # ScreenCaptureKit bridge (native helper)
        frames.ts
      sink.ts             # .astroshot write + manifest
      encode.ts           # webm/mp4 (Playwright file | ffmpeg | AVAssetWriter)
  tui-shot/               # stills stay; pty movie source reuses terminal-html
  react-shot/             # stills stay; browser source can wrap pages
bin/
  astroshot-movie         # or extend astroshot-capture with --movie
skills/
  astroshots-review/      # document movie harness contract
macos/                    # optional: small capture helper binary for SCK
  MovieCaptureHelper/     # if Node cannot call SCK cleanly → Swift CLI
```

**Native bridge note:** ScreenCaptureKit is Swift/ObjC. Practical options:

1. Small Swift CLI `astroshot-desktop-capture` (stream or file out) invoked by Node harness  
2. Embed capture in Astroshots.app and control via XPC/socket  
3. Node native addon (painful)

Prefer **(1) Swift helper** for harness purity and CI packaging.

---

## 7. Encoder strategy

| Input path | Encode |
|------------|--------|
| Playwright `recordVideo` | Native WebM; optional ffmpeg → MP4 |
| CDP JPEG frames / PNG seq | ffmpeg pipe (`-f image2pipe`) |
| ScreenCaptureKit `CMSampleBuffer` | `AVAssetWriter` → MP4 (helper) or raw frames → ffmpeg |
| frames push | ffmpeg pipe |

**macOS tray preference:** MP4/H.264 for AVKit simplicity; WebM OK if we stay poster-only in Phase 0.  
**PTY:** high bitrate; visual QA on saturated colors before aggressive chroma subsampling.

---

## 8. Harness integration patterns

### A. Agent / browser smoke (extend today’s dual-write)

```bash
# start movie once per case
astroshot-movie start --feature "$case" --slug journey \
  --source browser --from-agent-browser "$SESSION" --run-id "$RUN_ID"

# still optional: also capture key stills with astroshot-capture
# stop at end
astroshot-movie stop --status pass --finalize
```

### B. PTY / TUI journey

```bash
astroshot-movie run --source pty --fixture ./flows/onboard.pty.yaml \
  --feature tui-onboard --slug flow
```

### C. Native app (Astroshots itself, or any Mac app)

```bash
open -n /path/to/MyApp.app
astroshot-movie start --source desktop.window \
  --bundle-id com.example.MyApp --feature native-onboard --slug flow
# agent-browser-style click automation OR human OR AppleScript/XCUITest
astroshot-movie stop
```

### D. Mixed context (one feature, multiple sources)

One run can produce multiple movies:

```text
0001-browser-signup.webm + .png    source: browser
0002-cli-configure.webm + .png     source: pty
0003-desktop-finish.webm + .png    source: desktop.window
```

Same feature dir, same `run_id`, sequential ids — Astroshots already groups by feature.

---

## 9. Spike plan (this worktree)

Ordered to prove the **universal** shape early:

| # | Spike | Proves |
|---|-------|--------|
| 1 | `MovieSession` + `frames` source + ffmpeg/webm + `.astroshot` sink | API + contract without any special source |
| 2 | `browser` source via Playwright `recordVideo` | Headless web movies |
| 3 | `pty` truecolor baseline + live/sample encode | Color-critical terminal movies |
| 4 | Swift `desktop-capture` helper: one window → MP4 | Native path + TCC story |
| 5 | Wire `desktop.window` adapter to session | Full three-context demo |
| 6 | Bash harness example + skill docs | “any context” is usable |
| 7 | App Phase 1 (open video from poster row) | Review UX |

**Success demo (north star):** one script records (a) a browser flow, (b) a colorful PTY TUI, (c) a native window, all three land as poster+movie under `.astroshot/`, tray shows posters without app changes.

---

## 10. Non-goals (near term)

- Perfect headless native AppKit on CI without a display
- Audio tracks (add later as optional)
- Replacing still fixtures — movies are for **journeys**
- Cross-platform desktop capture on day one (macOS first)
- 60fps game capture quality (review default 10–15fps; native can opt up)

---

## 11. Decision matrix

| Need | Source |
|------|--------|
| Web journey / agent-browser | `browser` |
| Colorful TUI / CLI | `pty` (never desktop of Terminal.app) |
| SwiftUI / Electron / any Mac window | `desktop.window` |
| Whole monitor demo | `desktop.display` |
| Custom engine / remote / unknown | `frames` |
| Deterministic CI terminal | `pty` tape → offline |
| Deterministic CI web | `browser` headless |
| CI native | hard — display + TCC; treat as secondary |

---

## 12. Open questions

1. Package name: `@archastro/movie-harness` vs extend `@archastro/astroshot`?
2. CLI: `astroshot-movie` vs `astroshot-capture --movie`?
3. Default container: WebM vs MP4?
4. Native helper: ship Swift binary in repo / brew / with app?
5. Review hash: video, poster, or both?
6. Should Astroshots.app itself host SCK (always-on capture) or only the CLI helper?

---

## 13. One-sentence answer

**Build a `MovieSession` harness with pluggable sources (browser, truecolor PTY, ScreenCaptureKit desktop, raw frames) that all encode to the same `.astroshot` movie+poster contract — so any context can record a reviewable journey without each tool inventing its own recorder.**
