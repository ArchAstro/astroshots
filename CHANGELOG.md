# Changelog

All notable changes to Astroshots are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/).

macOS app releases use headers like `## [0.1.14] (macos) - YYYY-MM-DD`.
npm package releases use headers like `## [0.1.0] (npm) - YYYY-MM-DD`.
The two tracks are versioned independently.

## [Unreleased]

### Added

- Release automation: **Cut npm release** workflow, changelog roll-forward,
  and automatic **Release DMG** dispatch from **Cut release** (macOS).

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
