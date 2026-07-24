---
name: astroshots
description: >
  Write and live-watch harness screenshots via Astroshots — the macOS menu-bar
  app that streams frames from .astroshot/ across worktrees and flashes new ones
  as desktop overlays. Use when capturing browser/UI test screenshots for review,
  wiring agent-browser (or any harness) to the .astroshot write contract, running
  or debugging the Astroshots app, or when the user mentions Astroshots, astroshot,
  screenshot stream, test screenshot overlay, or live screenshot review.
---

# Astroshots

Astroshots is a **menu-bar Mac app** that watches the filesystem for screenshots
under `.astroshot/` and shows them live (tray stream + desktop overlay). Agents
and harnesses **write frames**; humans **watch**.

Canonical repo: `~/archastro/astroshots` (this skill ships with it).

| Surface | Job |
|---------|-----|
| Desktop overlay | New frame flashes above all windows |
| Menu-bar tray | Unified newest-first stream across all worktrees |
| Detail | Click a row; back returns to stream |
| Settings (gear) | Watch root, overlay on/off |

There is **no worktree picker**. Worktree is a badge on each row. Default watch
root is `~/archastro` (recursive).

---

## When to use this skill

| Situation | Do this |
|-----------|---------|
| Manual or agent browser walk of a feature | Capture into `.astroshot/<feature>/` so Calvin can watch |
| agent-browser smoke / harness journey | Write each step with `astroshot-capture` (or the contract below) |
| “Is Astroshots running / empty?” | Check app + watch root + that files are under `.astroshot/` |
| Docs / catalog PNGs for the product site | Use the **screenshot** skill (`react-shot` / docs path) — not Astroshots |

**Astroshots is for live test/review streams.** Ship-ready docs assets still go
through `.claude/skills/screenshot/SKILL.md`.

---

## Write contract (required)

From the **worktree root** (parent of `.astroshot`):

```text
.astroshot/<feature>/
  manifest.json
  0001-signed-in.png
  0002-configure.png
  …
```

Rules:

1. **Feature** = kebab-case journey name (`install-wizard`, `billing-home`).
2. **Files** = `NNNN-slug.png` (zero-padded sequence + slug). Also accept
   `.jpg` / `.jpeg` / `.webp` / `.gif`.
3. **manifest.json** is live-updated as shots land (optional but strongly preferred).
4. Worktree is **inferred** as the directory that contains `.astroshot` — do not
   invent a project registry.

### manifest.json shape

```json
{
  "version": 1,
  "feature": "install-wizard",
  "run_id": "install-wizard-20260724T133012Z-48291",
  "status": "running",
  "description": "Optional one-line journey summary",
  "shots": [
    {
      "id": "0002",
      "file": "0002-configure.png",
      "slug": "configure",
      "title": "Configure",
      "description": "What this frame proves.",
      "captured_at": "2026-07-24T13:30:24Z",
      "url": "/solutions · dialog",
      "viewport": "1280x1100"
    }
  ]
}
```

| `status` | Meaning |
|----------|---------|
| `running` | Journey in progress |
| `pass` | Finished OK |
| `fail` | Finished with failure |
| `idle` | Not active |

Update `status` to `pass` / `fail` when the case ends. Overlay and tray pick up
new PNGs via FSEvents (brief settle delay for partial writes).

---

## Preferred capture helper

After install, the skill directory includes `scripts/astroshot-capture`. Resolve it
from the installed skill path (typical locations):

```bash
# Global install (skills CLI)
CAPTURE="$(ls -d \
  ~/.agents/skills/astroshots/scripts/astroshot-capture \
  ~/.claude/skills/astroshots/scripts/astroshot-capture \
  ~/.codex/skills/astroshots/scripts/astroshot-capture \
  2>/dev/null | head -1)"

# Or from a clone of this repo:
# CAPTURE=./bin/astroshot-capture
```

```bash
FEATURE="install-wizard"

# After agent-browser is on the right page:
"$CAPTURE" \
  --feature "$FEATURE" \
  --slug configure \
  --title "Configure" \
  --description "Configuration screen for the resource being installed." \
  --url "/solutions · dialog" \
  --status running \
  --from-agent-browser "$SESSION"
```

Without agent-browser (copy an existing PNG):

```bash
"$CAPTURE" \
  --feature "$FEATURE" \
  --slug signed-in \
  --title "Signed in" \
  --description "Session authenticated." \
  --source ./shot.png
```

Mark the run finished:

```bash
"$CAPTURE" --feature "$FEATURE" --status pass --finalize
# or --status fail
```

The helper:

- Creates `.astroshot/<feature>/` under the git/worktree root
- Writes `NNNN-slug.png`
- Merges the shot into `manifest.json`
- Prints the absolute path (so you can log it)

---

## agent-browser without the helper

```bash
REPO_ROOT="$(git rev-parse --show-toplevel)"
FEATURE="install-wizard"
DIR="$REPO_ROOT/.astroshot/$FEATURE"
mkdir -p "$DIR"
# next free index:
N=$(printf '%04d' $(( $(ls "$DIR"/*.png 2>/dev/null | wc -l) + 1 )))
SLUG="configure"
PATH_PNG="$DIR/${N}-${SLUG}.png"

agent-browser --session "$SESSION" wait --load networkidle
agent-browser --session "$SESSION" screenshot --full "$PATH_PNG"
# then update manifest.json (prefer the helper)
```

For the **agent_network smoke harness** (`services/agent_network/test-harness/agent-browser/`),
mirror `smoke_capture` so it also writes under `$REPO_ROOT/.astroshot/$case_name/`
(in addition to or instead of `/tmp/archagents-browser-smoke/...`). Feature name
= case name (`install-wizard`).

---

## Running the Mac app

```bash
# From a clone of https://github.com/ArchAstro/astroshots
cd macos
./scripts/bootstrap.sh          # needs xcodegen
open Astroshots.xcodeproj       # ⌘R
```

Or install a signed build from [Releases](https://github.com/ArchAstro/astroshots/releases).

- Set **watch root** under tray → gear if needed.
- App is menu bar only (no Dock icon).

### Sanity check that a write will appear

```bash
# From a project under the watch root:
mkdir -p .astroshot/smoke-check
cp /path/to/any.png .astroshot/smoke-check/0001-ping.png
# Overlay should flash if Astroshots is running and overlay is enabled.
```

---

## Agent workflow (browser feature review)

1. Confirm Astroshots is running (or tell the human to start it).
2. Name the **feature** (kebab-case).
3. Drive agent-browser (or the app) to each meaningful state.
4. After each state: `astroshot-capture` with slug + title + description + url.
5. On success/fail: `--finalize --status pass|fail`.
6. Tell the human: feature name, worktree, and that frames are under
   `.astroshot/<feature>/` — they can leave the tray closed and still see overlays.

Do **not** dump screenshots only under `/tmp/...` if the goal is live review —
Astroshots will never see them.

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Empty tray | Files not under `**/.astroshot/**`, or watch root does not include this worktree |
| No overlay | Gear → Show overlay; or file still settling (wait ~0.5s); or status bar app not running |
| Wrong worktree badge | `.astroshot` must sit at worktree root, not nested under `services/…` unless that *is* the intended root |
| Manifest ignored | Valid JSON; `file` must match the PNG basename |
| Partial / corrupt PNG in UI | Write atomically (helper does write-to-temp + move when possible) or wait for harness to finish the file |

---

## Related

- Repo: https://github.com/ArchAstro/astroshots
- Design mock: `docs/mocks/astroshots-menubar.html`
- App README: `macos/README.md`
- Capture script: `skills/astroshots/scripts/astroshot-capture`
