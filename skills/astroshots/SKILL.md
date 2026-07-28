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

Canonical repo: [ArchAstro/astroshots](https://github.com/ArchAstro/astroshots).

### Install this skill

**Global** (user-level, all projects):

```bash
npx skills add ArchAstro/astroshots --skill astroshots -g -y
```

**This git project only** (from the project root — omit `-g`):

```bash
cd /path/to/your/project
npx skills add ArchAstro/astroshots --skill astroshots -y
```

Optional agents: `-a claude-code -a cursor` or `-a '*'`.  
Update: `npx skills update astroshots -g -y` (global) or `npx skills update astroshots -y` (project).

| Surface | Job |
|---------|-----|
| Desktop overlay | New frame flashes above all windows |
| Menu-bar tray | Unified newest-first stream across projects under all watched folders |
| Detail | Click a row; back returns to stream |
| Settings (gear) | Watched folders, overlay on/off |

No project picker — project is a badge on each row. Configure one or more watched folders in-app.

---

## Choose the capture tool

| What the frame must prove | Tool |
|---------------------------|------|
| Fixed React component state | `@archastro/astroshot react` from npmjs and the **react-shot** skill |
| Fixed Ink terminal state | `@archastro/astroshot tui` from npmjs and the **tui-shot** skill |
| Routing, auth, live data, or full browser shell | `agent-browser` and the **agent-browser** skill |
| Live human review of any resulting PNG | `astroshot-capture` from this skill |

The two npm modes generate deterministic images. This skill describes how to
stream any generated image into the macOS app.

---

## When to use this skill

| Situation | Do this |
|-----------|---------|
| Manual or agent browser walk of a feature | Capture into `.astroshot/<feature>/` for live review |
| Harness / smoke journey | Write each step with `astroshot-capture` (or the contract below) |
| “Is Astroshots running / empty?” | Check app + watched folders + that files are under `.astroshot/` |
| Shipping docs site PNGs | Use the **screenshot** skill; use Astroshots for its visual review stream |

**Astroshots is for live test/review streams**, not catalog asset production.

---

## Write contract (required)

From the **worktree root** (parent of `.astroshot`):

```text
.astroshot/<feature>/
  manifest.json
  review.json
  0001-signed-in.png
  0002-configure.png
  …
```

Rules:

1. **Feature** = kebab-case journey name (`install-wizard`, `billing-home`).
2. **Files** = `NNNN-slug.png` (zero-padded sequence + slug). Also accept
   `.jpg` / `.jpeg` / `.webp` / `.gif`.
3. **manifest.json** is live-updated by the harness as shots land (optional but
   strongly preferred).
4. **review.json** is human feedback written by Astroshots. Agents read it; they
   do not use `manifest.status` as approval.
5. Worktree is **inferred** as the directory that contains `.astroshot` — do not
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

| `status` | Execution meaning |
|----------|-------------------|
| `running` | Capture journey in progress |
| `pass` | Capture journey finished successfully |
| `fail` | Capture journey finished with a failure |
| `idle` | Capture journey is not active |

Update `status` to `pass` / `fail` when the case ends. Overlay and tray pick up
new PNGs via FSEvents (brief settle delay for partial writes).

`manifest.status` is never human approval. A run may be `pass` while one or
more screenshots have `changes_requested` in `review.json`.

### review.json shape

Path: `.astroshot/<feature>/review.json`

```json
{
  "version": 1,
  "run_id": "install-wizard-20260726T174200Z-48291",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0002-configure.png": {
      "decision": "changes_requested",
      "reviewed_at": "2026-07-26T17:42:00Z",
      "image_sha256": "a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1",
      "comments": [
        {
          "id": "A1B2C3D4-E5F6-47A8-9000-111122223333",
          "body": "The primary action is clipped at this width.",
          "created_at": "2026-07-26T17:41:32Z"
        }
      ]
    }
  }
}
```

Rules:

1. `reviews` is keyed by the exact image basename, not the manifest shot id.
2. `decision` is optional. When present it is `approved` or
   `changes_requested`; only a human reviewer sets it.
3. `reviewed_at` records the latest human decision. It is written with a
   decision and may be absent from comment-only pending feedback.
4. `image_sha256` is the lowercase SHA-256 of the bytes the human reviewed. It
   is written with a decision and may be absent from comment-only pending
   feedback.
5. `comments` is an ordered array of `{id, body, created_at}`. Comments remain
   readable even when a decision becomes stale.
6. An absent `decision` means pending review, including when comments exist.
   If a stored decision's hash differs from the current file, treat it as
   pending/stale—not approved—and preserve the comments as revision guidance.
7. Review feedback is scoped to its capture run. When the manifest has a run
   id and `review.json.run_id` is missing or different, treat the current run
   as pending and do not carry forward the prior run's comments or decision.
8. A missing `review.json` or missing filename entry also means pending review.

### Read human feedback as an agent

Do not edit `review.json` to approve your own work. Read the entry, compare its
hash with the current file, and report every comment:

```bash
FEATURE="install-wizard"
FILE="0002-configure.png"
DIR=".astroshot/$FEATURE"
MANIFEST="$DIR/manifest.json"
REVIEW="$DIR/review.json"

if command -v sha256sum >/dev/null 2>&1; then
  CURRENT_SHA="$(sha256sum "$DIR/$FILE" | awk '{print $1}')"
else
  CURRENT_SHA="$(shasum -a 256 "$DIR/$FILE" | awk '{print $1}')"
fi

MANIFEST_RUN_ID="$(jq -r '.run_id // empty' "$MANIFEST" 2>/dev/null || true)"

if [[ ! -f "$REVIEW" ]]; then
  jq -n --arg file "$FILE" \
    '{file: $file, review_state: "pending", comments: []}'
else
  jq --arg file "$FILE" --arg sha "$CURRENT_SHA" \
    --arg manifest_run_id "$MANIFEST_RUN_ID" '
    if $manifest_run_id != "" and (.run_id // "") != $manifest_run_id then
      {file: $file, review_state: "pending", comments: []}
    else
      (.reviews[$file] // null) as $review
      | {
          file: $file,
          review_state: (
            if $review == null then "pending"
            elif $review.decision == null then "pending"
            elif $review.image_sha256 != $sha then "stale"
            else $review.decision
            end
          ),
          comments: ($review.comments // [])
        }
    end
  ' "$REVIEW"
fi
```

For `changes_requested`, address the comments and capture a new frame or replace
the intended file. The old decision cannot carry across changed bytes; wait for
the human to review the new hash. Never claim approval based on an automated
test or a `pass` execution manifest.

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
# Generate the source with react-shot, tui-shot, or any image tool.
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

This finalizes harness execution only. It does not update `review.json` or
approve a screenshot.

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

Prefer `astroshot-capture` so the manifest stays in sync.

---

## Running the Mac app

```bash
# From a clone of https://github.com/ArchAstro/astroshots
cd macos
./scripts/bootstrap.sh          # needs xcodegen
open Astroshots.xcodeproj       # ⌘R
```

Or install a signed build from [Releases](https://github.com/ArchAstro/astroshots/releases).

- Add or remove **watched folders** under tray → gear if needed.
- App is menu bar only (no Dock icon).

### Sanity check that a write will appear

```bash
# From a project under any watched folder:
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
5. On success/fail: `--finalize --status pass|fail` for execution state.
6. Check `review.json` for exact-filename comments and decisions. Ignore
   feedback from a different manifest run; treat current-run hash mismatches as
   stale and their comments as revision instructions.
7. Tell the human: feature name, worktree, execution result, and review state;
   frames are under
   `.astroshot/<feature>/` — they can leave the tray closed and still see overlays.

Do **not** dump screenshots only under `/tmp/...` if the goal is live review —
Astroshots will never see them.

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Empty tray | Files not under `**/.astroshot/**`, or no watched folder includes this worktree |
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
- **screenshot** skill — plan and review documentation image sets
  `npx skills add ArchAstro/astroshots --skill screenshot -g -y`
- **react-shot** skill — deterministic React component PNGs
  `npx skills add ArchAstro/astroshots --skill react-shot -g -y`
- **tui-shot** skill — deterministic Ink terminal PNGs
  `npx skills add ArchAstro/astroshots --skill tui-shot -g -y`
- **agent-browser** skill — drive the browser CLI  
  `npx skills add ArchAstro/astroshots --skill agent-browser -g -y`
- **browser-ui-harness** skill — Bash harness layout & UI testing practices  
  `npx skills add ArchAstro/astroshots --skill browser-ui-harness -g -y`
