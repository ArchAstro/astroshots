---
name: astroshots-review
description: >
  Stream stills and journey movies into live human review through Astroshots,
  the macOS menu-bar app that watches .astroshot/ across worktrees, and read
  its hash- and run-scoped feedback. Use when wiring a harness to the
  .astroshot contract, choosing a movie source, operating movie playback,
  separating one-off Shots from the reserved friction-log tree, or debugging
  overlays and review state. This is review transport, not the underlying
  React, terminal, browser, friction-log, or documentation capture workflow.
---

# Astroshots live review

Astroshots watches `.astroshot/` trees, flashes new frames as desktop overlays,
and keeps one menu-bar stream across watched worktrees. Capture tools produce
images; Astroshots transports them to a human and writes feedback.

## Choose the capture source

| Frame must prove | Capture skill |
|---|---|
| Fixed React, Ink, or terminal executable state | **astroshot** (still PNG) |
| Journey **movie** (web / TUI / native window / frames) | **`astroshot movie`** |
| Routing, auth, live data, or browser shell | **agent-browser** |
| Reusable end-to-end browser journey | **browser-ui-harness** |
| Documentation image production and rendered page | **screenshot** |

### Movies — pick `--source` before recording

Agents must run (or follow) the decision table:

```bash
astroshot movie which-source "<what you need to record>"
astroshot movie --help
```

| Intent | `--source` | Do not |
|--------|------------|--------|
| Web / SPA / Playwright | `browser` | Desktop-grab Chrome |
| TUI / CLI / truecolor | `pty` | `desktop.window` on Terminal.app |
| Native Mac app window | `desktop.window` | Headless browser for AppKit chrome |
| Custom PNG sequence | `frames` | Invent a new sink path |

`desktop.window` is macOS-only and uses OS `screencapture` (already installed)
plus the package’s Swift window list — no extra binary download. Needs Screen
Recording TCC. Movies write **poster PNG + video** under `.astroshot/`; the
tray shows duration and a play overlay, filters Movies, and plays the video in
tray or full-screen review with scrubbing, volume, and full-screen controls.
Manifest metadata can also surface source and chapters. Review remains keyed
to the poster filename/hash; replacing the poster makes Seen stale.

This skill starts after a capture source exists.

## Write contract

Write from the worktree root:

```text
.astroshot/<feature>/
  manifest.json
  review.json
  0001-signed-in.png
  0002-configure.png
  0003-journey.png       # movie poster
  0003-journey.webm      # movie payload
```

- Feature names are kebab-case.
- Image names start with a zero-padded sequence and slug.
- `manifest.json` is harness execution state.
- `review.json` is human feedback written by the app.
- The directory containing `.astroshot` defines the worktree/project.
- Do **not** put one-off harness frames under `.astroshot/friction-logs/` —
  that reserved tree is for agentic friction-log scenarios (see the
  **friction-log** skill). Those runs appear in the tray **Friction Logs**
  tab, not the Shots stream.

Read [the manifest and review contract](references/manifest.md) whenever
writing a custom integration or interpreting feedback. Read
[harness integration](references/harness-integration.md) when dual-writing
from a Bash or agent-browser harness.

## Use the capture helper

Resolve `astroshot-capture` from an explicit override, a project install, a
global skill install, or this repository:

```bash
CAPTURE="${CAPTURE:-}"
if [[ ! -x "$CAPTURE" ]]; then
  for candidate in \
    "./bin/astroshot-capture" \
    "./.agents/skills/astroshots-review/scripts/astroshot-capture" \
    "./.claude/skills/astroshots-review/scripts/astroshot-capture" \
    "./.codex/skills/astroshots-review/scripts/astroshot-capture" \
    "$HOME/.agents/skills/astroshots-review/scripts/astroshot-capture" \
    "$HOME/.claude/skills/astroshots-review/scripts/astroshot-capture" \
    "$HOME/.codex/skills/astroshots-review/scripts/astroshot-capture"; do
    if [[ -x "$candidate" ]]; then
      CAPTURE="$candidate"
      break
    fi
  done
fi
test -x "$CAPTURE" || {
  echo "Install the astroshots-review skill or set CAPTURE to its helper" >&2
  exit 1
}
```

Stream an existing image:

```bash
"$CAPTURE" --feature account-settings \
  --slug account-dialog \
  --title "Account dialog" \
  --description "The editable account fields are visible." \
  --source ./account-dialog.png
```

Capture a live browser session:

```bash
"$CAPTURE" --feature account-settings \
  --slug saved \
  --title "Saved" \
  --description "The success state is visible." \
  --url "/account" \
  --from-agent-browser "$SESSION"
```

For a reusable harness, pass one stable `--run-id` to every capture and the
finalize call. Finalize execution state when the journey ends:

```bash
"$CAPTURE" --feature account-settings --status pass --finalize
# use --status fail when the journey failed
```

`pass` means capture execution succeeded. It never means a human has seen the image.

## Read review feedback

For every current-run frame:

1. Find the exact filename entry in `review.json`.
2. Confirm `review.json.run_id` matches `manifest.json.run_id`.
3. If a decision exists, compare `image_sha256` with the current file bytes.
4. Report every applicable comment.

The resulting state is:

| Condition | State |
|---|---|
| Missing file, review file, entry, or decision | `unseen` |
| Review run differs from manifest run | `unseen`; suppress old comments |
| Decision hash differs from current bytes | `stale`; comments remain guidance |
| Matching current-run `seen` decision and hash | `seen` |

Never edit `review.json` to mark your own work Seen. Address feedback, capture
new bytes, and let the human review the new hash.

## Operate and troubleshoot the app

Install a signed build from GitHub Releases, or from a clone:

```bash
cd macos
./scripts/bootstrap.sh
open Astroshots.xcodeproj
```

The app is menu-bar only. Configure watched folders and overlay visibility
from its gear menu.

| Symptom | Check |
|---|---|
| Empty tray | The file is under `.astroshot/` and a watched folder contains the worktree |
| No overlay | App is running, overlay is enabled, and the file has settled |
| Wrong project badge | `.astroshot` is at the intended worktree root |
| Manifest metadata missing | JSON parses and `file` matches the image basename |
| Corrupt preview | Producer writes atomically; the helper does |

Report the feature, worktree, execution result, current review state, every
applicable comment, and the `.astroshot/<feature>/` path.
