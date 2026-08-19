# Changelog

All notable changes to Astroshots are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/).

macOS app releases use headers like `## [0.1.14] (macos) - YYYY-MM-DD`.
npm package releases use headers like `## [0.1.0] (npm) - YYYY-MM-DD`.
The two tracks are versioned independently.

## [Unreleased]

## [0.2.1] (npm) - 2026-08-19

### Added

- **Unscoped `astroshot` npm package:** `npx astroshot` now works with no
  registry flags. The new package bundles `@archastro/astroshot` and its three
  capture engines via `bundleDependencies`, so nothing scoped is fetched at
  install time and a `~/.npmrc` that maps `@archastro` to a private registry can
  no longer produce an E404. `npm run pack:check` proves this by installing the
  package with `@archastro` pointed at an unreachable registry and then running
  the CLI. See [`docs/UNSCOPED-CLI-DESIGN.md`](docs/UNSCOPED-CLI-DESIGN.md).
- **`astroshot demo` — a one-command first win:** writes a complete
  `.astroshot/<feature>/` set (two stills, a movie poster + WebM pair, and a
  contract-valid `manifest.json`) from bundled fixtures. It needs no Chromium
  download, no ffmpeg, and no assets of your own, so it works immediately after
  install and even while the app is closed. This replaces the README
  quickstart's manual "copy any PNG into `.astroshot/quickstart/`" step.
- **`astroshot doctor` — one line per silent failure mode:** reports Node
  version vs `engines`, whether this project is inside a folder Astroshots
  actually watches (read from the app's live `watchRoots` / legacy `watchRoot`
  configuration, never a guessed `~/Projects`), whether the app is installed and
  running, whether the managed Chromium runtime is present, and macOS Screen
  Recording state for `desktop.window`. Every check prints pass/fail plus the
  exact remediation command, `--json` emits the same report for tooling, and the
  process exits non-zero when a required check fails. `doctor` is read-only: it
  never installs anything and never changes app state. "Empty tray" now resolves
  to three distinct answers with different fixes: first-launch setup never
  completed, this project is outside every watched folder, or the project is
  watched and the problem is elsewhere.
- **Homebrew cask install:** `brew install --cask ArchAstro/tools/astroshots`
  installs the signed, notarized app into `/Applications` — no DMG download or
  drag step. The cask declares `auto_updates true` so Sparkle keeps handling
  updates, and **Release DMG** bumps the cask version and sha256 automatically
  after each release. The DMG download remains available as a secondary path.

### Changed

- **The documented install path is now a plain command:** every
  `npx --@archastro:registry=… @archastro/astroshot …` invocation in the README,
  the package READMEs, the skills, and the go-live checklist is now
  `npx astroshot …`. Chromium setup errors from `react-shot` and `tui-shot`
  point at the same plain command. The scoped packages keep working unchanged
  for existing consumers.

## [0.2.8] (macos) - 2026-08-14

### Added

- **Non-destructive friction-log hiding:** hide individual friction logs from
  the tray while keeping their files on disk, then restore them individually or
  all at once from Settings.
- **Unseen badges and friction-log Seen:** the Shots and Friction Logs tabs
  show how many items still need review. Friction logs use the same Unseen /
  History filter and mark-seen control as shots, persisted in each run's
  `review.json`. A newer run makes the log unseen again.

### Changed

- **Narrated-video captions are now opt-in:** Settings → Narration has a
  **Burn in captions** toggle, off by default. Word-clock captions cannot stay
  locked to Qwen's actual pacing.
- **Steadier narrator across a video:** each job synthesizes one short Ryan
  (or chosen speaker) clip, then clones every later sentence from that same
  reference so timbre does not wander between chunks.
- **Setup card after the intro:** narrated videos open the friction-log Goal
  and Persona as a spoken, typeset page before the first screenshot.

### Fixed

- **Tray stays after full-screen review:** closing the large viewer restores
  the menu-bar drawer that opened it, instead of leaving the tray dismissed.
- **Narrated video cutoff and crawl:** long step transcripts are now spoken in
  sentence chunks, trailing TTS silence is trimmed before captions are timed,
  and the voice is asked to talk at a conversational pace (temperature 0.5).
- **macOS dark mode:** tray, Settings, detail views, friction logs, overlays,
  and review sheets now use appearance-aware backgrounds, text, borders, and
  accent colors instead of mixing dark system text with fixed light cards.

## [0.2.6] (macos) - 2026-08-12

### Added

- **Narration voices:** Settings now offers persistent Qwen3 voice selection
  with sample playback, and every narration job consistently uses the chosen
  voice.
- **Share-ready narrated videos:** generated MP4s include burned-in captions,
  the friction-log title in the filename, and branded opening and closing cards
  built from the Astroshots icon and DMG artwork.

### Changed

- **Feedback-first walkthroughs:** each image begins beside its rendered review
  feedback, then the feedback fades away after about 30 seconds while the image
  expands edge-to-edge. Short narration steps still receive a fade and
  full-screen hold before advancing.

## [0.2.5] (macos) - 2026-08-12

### Fixed

- **Software Update refresh:** every update check now bypasses stale appcast
  caches, shows active checking state in Settings, and ships reachable release
  notes. Release CI verifies the public feed, archive, and EdDSA signature end
  to end before completing.

## [0.2.4] (macos) - 2026-08-12

### Fixed

- **Narrated video generation:** feed audio and video to the encoder concurrently
  so longer friction-log narrations complete instead of hanging during MP4 encoding.

## [0.2.3] (macos) - 2026-08-11

### Changed

- **Friction Logs run history:** replaced the native menu chrome with an
  Astroshots-themed, scrollable run picker that clearly identifies the latest
  run and shows each run's timestamp, step count, and stable ID.
- **Product and agent guidance:** refreshed the README, bundled skills, and
  reproducible screenshot set for Movies, Friction Logs, narrated videos, and
  app + skills onboarding.

## [0.2.2] (macos) - 2026-08-10

### Added

- **Movie playback controls:** tray and full-screen review now provide play/pause,
  scrubbing, volume, full-screen controls, and a polished loading state for both
  Astroshot WebM captures and MP4/MOV movies.

### Fixed

- **Tray movie stability:** replaced the crashing SwiftUI AVKit bridge with
  lifecycle-owned native players.
- **Tray navigation:** redesigned the header and tabs around the clearer Agent
  Rooms hierarchy, with balanced tab hit areas and a compact selected pill.

## [0.2.1] (macos) - 2026-08-10

### Added

- **Software Update (Sparkle):** standard `SPUStandardUpdaterController` owned
  by the app delegate; **Check for Updates…** on the status-item menu (target/
  action on the controller); Settings → Software Update with automatic check /
  download toggles (Sparkle’s documented preferences pattern). Release CI
  publishes a signed update zip + cumulative `appcast.xml` beside the notarized
  DMG (`SPARKLE_ED_PRIVATE_KEY` — see `docs/SIGNING.md`). Upgrade stages log
  to Console under subsystem `ai.archastro.Astroshots` category
  `SoftwareUpdate` (startup, appcast load, found/no update, download, extract,
  install, relaunch, errors).
- **Narrated friction-log videos (opt-in):** Settings → Narration enables on-device
  **mlx-audio-swift** (MLX Swift) + Qwen3-TTS. The app checks Apple Silicon,
  downloads the HF model in the background with a progress bar, loads weights
  in-process, then exposes **Make narrated video** on friction-log run detail.
  Renders serialize through a job queue into `narration-<runID>.mp4` beside the
  run’s screenshots.
- CI/macOS builds: install Metal toolchain + `-skipPackagePluginValidation` for
  mlx-swift (see `macos/scripts/xcode-env.sh`).
- Friction-log steps require a **`transcript`** field: short spoken narrative
  (actions + good + bad) with cross-step transitions for narrated video.
  Skill + on-disk contract document the voice rules; the app loads and shows
  transcripts in the tray step detail and full-screen takeover (older logs
  without the field still load with an empty transcript).
- Friction Logs **run history** follows tray filter-chip patterns: list footer
  shows run count + human latest time; detail uses `StreamFilterChip`-style
  28pt pills under a `RUNS` section header (no nested card). Skill documents
  unique run dirs and cleaning empty stubs.
- Step **transcript** is a peer callout to Looks good / Can improve (same soft
  fill + accent stroke) in tray detail and full-screen takeover.

## [0.2.0] (macos) - 2026-08-09

### Added

- **Friction Logs** tray tab: list agentic UX scenarios under
  `.astroshot/friction-logs/`, step through JSONL runs with screenshots and
  Looks good / Can improve notes, improve rollup, and keyboard ← → step nav.
- **Movies as first-class review frames** in the Shots stream: Movie duration
  badges, play overlay, **Movies** filter, Play in tray / Open movie, chapters
  list, and Kind / Video / Duration / Source metadata from the manifest.
- Fixture / UI-test launches open a **pinned on-screen tray window** (seeded
  status + labeled Pin/Unpin) so agents can `desktop.window` Astroshots itself.
- Right-click **Copy Image** on stream rows, tray detail preview, and
  full-screen review so frames paste into other apps.

## [0.2.0] (npm) - 2026-08-09

### Added

- **`astroshot movie`** universal journey capture (`@archastro/movie-harness`):
  browser, truecolor PTY, raw frames, and macOS `desktop.window`, writing
  poster PNG + video under `.astroshot/`. Includes agent-facing
  `which-source` help and Screen Recording TCC guidance.
- npm release automation ships four packages at one version: `react-shot`,
  `tui-shot`, `movie-harness`, and the unified `astroshot` CLI.

### Fixed

- `astroshot movie which-source` recommends `desktop.window` for Astroshots /
  menu-bar / tray intents (no longer defaults those to browser).
- `list-windows` includes floating/popover layers; prefers on-screen windows;
  refuses nearly blank or off-screen captures by default (`--allow-blank`).
- Movie stream descriptions are human-readable instead of raw
  `desktop.window id=…` dumps.

## [0.1.15] (macos) - 2026-08-05

### Added

- CI caches Playwright Chromium under `.cache/ms-playwright` (keyed by OS,
  arch, and Playwright version) so `install-browser` is a no-op on warm
  runners; Linux still applies `--with-deps` for system libraries.
- First-run setup replaces any default watch path: a fresh install is detected
  via an explicit `hasCompletedFirstRunSetup` preference and watches nothing
  until the user chooses folders. Launch opens the tray + folder panel only
  when that flag is incomplete. The open panel starts browsing in `~/Projects`
  when that directory exists (picker start only, not a root). Upgrades that
  already have saved watch roots skip first-run.
- Release automation: **Cut npm release** workflow, changelog roll-forward,
  and automatic **Release DMG** dispatch from **Cut release** (macOS).

### Changed

- Removed the implicit `~/archastro` watch root. Existing saved roots are
  unchanged.

## [0.1.14] (macos) - 2026-08-04

### Added

- Left/right image paging in tray detail and full-screen review (keyboard
  arrows, header chevrons, stage-side arrows, position labels).

### Fixed

- Windows PTY crash e2e no longer races settle: wait for painted frame and
  process exit; re-read the ConPTY status bridge after settle.

## [0.1.13] (macos) - 2026-07-31

### Added

- Settings and version in the status-item menu.

## [0.1.12] (macos) - 2026-07-30

### Fixed

- Recover missed shot notifications / delayed filesystem events for overlays.

## [0.1.0] (npm) - 2026-08-04

### Added

- Initial public npm packages: `@archastro/react-shot`, `@archastro/tui-shot`,
  and the unified `@archastro/astroshot` CLI (React, Ink, and PTY modes).
