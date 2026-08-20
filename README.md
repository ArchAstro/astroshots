<div align="center">

# Astroshots

[![CI](https://github.com/ArchAstro/astroshots/actions/workflows/ci.yml/badge.svg)](https://github.com/ArchAstro/astroshots/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/astroshot?label=astroshot)](https://www.npmjs.com/package/astroshot)
[![macOS 14+](https://img.shields.io/badge/macOS-14%2B-black?logo=apple)](https://github.com/ArchAstro/astroshots/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

**Menu-bar Mac app that live-streams harness screenshots from `.astroshot/` across worktrees.**

<img src="docs/images/shots-movie-stream.png" alt="Astroshots Shots tab with a movie row, duration badge, Movies filter, and review state" width="330" />
&nbsp;
<img src="docs/images/movie-detail.png" alt="Movie detail with Play in tray, Open movie, chapters, feedback, and Seen controls" width="330" />

<img src="docs/images/overlay-card.png" alt="Desktop overlay for a newly captured onboarding movie with a direct Open in Astroshots action" width="420" />

</div>

---

Your test suites, agents, and UI harnesses already produce screenshots and
recordings — they just die in `/tmp` or a CI artifact nobody opens. Astroshots
watches the folders where your projects live, flashes every new frame as a
desktop overlay, and streams stills, journey movies, and UX friction logs into
one newest-first tray.

Your feedback goes back to disk beside the capture, so the agent that took the
screenshot can read what you said about it.

- **No project picker, no account, no cloud** — just files on disk and one icon in the menu bar.
- **Every worktree in one stream** — stills and movies from every project under your watched folders, newest first.
- **Movies, not just stills** — WebM/MP4/MOV playback with scrubbing, chapters, and duration metadata; **Make narrated video** turns a walkthrough into an on-device MP4.
- **Friction Logs** — agentic UX walkthroughs with per-step evidence, transcript, and good/improve notes, plus switchable run history.
- **A contract, not an API** — write a folder, read `review.json`; any language or harness can participate.
- **Six agent skills included** — your coding agent learns to capture, stream, and act on human feedback.

---

## Quick Start

```bash
brew install --cask ArchAstro/tools/astroshots   # then pick folders to watch
open -a Astroshots
npx skills add ArchAstro/astroshots --skill '*' -g -y
npx astroshot demo                               # proof of life, no assets needed
npx astroshot doctor                             # if nothing shows up
```

`astroshot demo` writes real stills and a movie into `.astroshot/astroshot-demo/`
inside the current project. Open the menu-bar icon → **Shots**: an
**astroshot-demo** row with a movie badge means the app and the on-disk contract
are connected.

> **`No available cask`?** `Casks/astroshots.rb` lives in the separate
> [ArchAstro/homebrew-tools](https://github.com/ArchAstro/homebrew-tools) tap and
> has not merged yet. Until it does, download the signed
> **Astroshots-x.y.z.dmg** from
> [Releases](https://github.com/ArchAstro/astroshots/releases) and drag it to
> Applications — see [`docs/install.md`](docs/install.md).

Prerequisites: macOS 14+, and Node.js 22.14+ for the capture CLI.

---

## How it works

```text
   ┌──────────────────────────┐
   │  agent / harness / CI    │  writes PNGs, movie poster+video,
   │  (any language)          │  manifest.json
   └────────────┬─────────────┘
                │
                ▼
      <project>/.astroshot/<feature>/
       0002-configure.png
       0003-onboarding.png + .webm
       manifest.json                    ← execution state (running/pass/fail)
                │
                │  fsevents on your watched folders
                ▼
   ┌──────────────────────────┐
   │   Astroshots menu bar    │  overlay flash → Shots stream → review
   │   (macOS, no Dock icon)  │  Friction Logs → narrated video
   └────────────┬─────────────┘
                │  you hit Seen / leave a comment
                ▼
       review.json                      ← human state, hash + run-id scoped
                │
                ▼
   ┌──────────────────────────┐
   │  agent reads feedback    │  fixes the UI, recaptures, loop closes
   └──────────────────────────┘
```

`manifest.json` is machine state; `review.json` is human state. They never mix:
an agent may not manufacture a Seen. Replacing an image invalidates its
acknowledgement, and feedback is scoped to the run that produced it — full rules
in [`docs/contract.md`](docs/contract.md).

---

## Capture something real

```bash
npx astroshot react ./fixtures/account-dialog.tsx -o ./shots/account-dialog.png
npx astroshot movie run --source browser --feature checkout --slug purchase \
  --url https://example.com/checkout
```

One CLI, four modes — `react`, `ink`, `pty` for deterministic stills, and
`movie` for browser, truecolor PTY, native macOS window, or custom-frame
journeys. Details in [`docs/capture.md`](docs/capture.md).

---

## Docs

| Page | What's in it |
|------|--------------|
| [`docs/install.md`](docs/install.md) | Homebrew, DMG, source builds, first launch, `demo` / `doctor` |
| [`docs/contract.md`](docs/contract.md) | Write layout, `manifest.json`, `review.json`, hash + run-id scoping, agent obligations |
| [`docs/capture.md`](docs/capture.md) | The four capture modes, fixtures, batch manifests, movie sources, `astroshot-capture` |
| [`docs/friction-logs.md`](docs/friction-logs.md) | Friction-log tree and the JSONL step schema |
| [`docs/skills.md`](docs/skills.md) | All six agent skills and the full install matrix |
| [`docs/product-tour.md`](docs/product-tour.md) | Every surface, the full review loop, requirements |
| [`docs/releasing.md`](docs/releasing.md) | Cut workflows, signed DMG, npm trusted publishing and bootstrap |
| [`docs/PREFERENCES.md`](docs/PREFERENCES.md) | Reading which folders Astroshots watches |
| [`docs/UNSCOPED-CLI-DESIGN.md`](docs/UNSCOPED-CLI-DESIGN.md) | Why `astroshot` and `@archastro/astroshot` both exist |
| [`CHANGELOG.md`](CHANGELOG.md) | User-facing release notes (macOS and npm tracks) |

This repo ships **six** skills — one command installs them all, see
[`docs/skills.md`](docs/skills.md) for per-skill, per-project, and per-agent
variants.

---

## Development

macOS 14+, Node.js 22.14+, Xcode 26+, and
[XcodeGen](https://github.com/yonaskolb/XcodeGen) for the app.

```bash
npm ci
npm run check          # typecheck, tests, verify:skills, pack:check
npm run verify:skills  # canonical docs + skill proof

cd macos && ./scripts/bootstrap.sh && open Astroshots.xcodeproj
```

Regenerate the README/docs images from the native Debug app with
`bash scripts/capture-readme-screenshots.sh`; the inventory is
[`docs/screenshots.json`](docs/screenshots.json).

Full commands, review-UI test requirements, and PR expectations:
[`CONTRIBUTING.md`](CONTRIBUTING.md) and [`macos/README.md`](macos/README.md).
Report vulnerabilities privately as described in [`SECURITY.md`](SECURITY.md).

---

## License

Astroshots and its npm packages are available under the
[MIT License](LICENSE).
