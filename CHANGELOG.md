# Changelog

All notable changes to Astroshots are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/).

macOS app releases use headers like `## [0.1.14] (macos) - YYYY-MM-DD`.
npm package releases use headers like `## [0.1.0] (npm) - YYYY-MM-DD`.
The two tracks are versioned independently.

## [Unreleased]

### Added

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
