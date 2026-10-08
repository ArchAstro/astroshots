---
name: astroshots-review
description: >
  Stream stills and journey movies into live human review through Astroshots,
  the macOS menu-bar app that watches .astroshot/ across worktrees, and read
  its hash- and run-scoped feedback. Use when wiring a harness to the
  .astroshot contract, choosing a movie source, operating movie playback,
  separating one-off Shots from the reserved user-story tree, or debugging
  overlays and review state. This is review transport, not the underlying
  React, terminal, browser, user-story, or documentation capture workflow.
---

# Astroshots live review

Astroshots watches `.astroshot/` trees, flashes new frames as desktop overlays,
and keeps one menu-bar stream across watched worktrees. Capture tools produce
images; Astroshots transports them to a human and writes feedback.

## Native `archdev shots` support

When `archdev shots` is available and enabled, the Astroshots engine is also a
command group of `archdev`. Detect it once per session:

```bash
if command -v archdev >/dev/null 2>&1 && archdev shots doctor >/dev/null 2>&1; then
  NATIVE=1   # archdev shots is installed and enabled
else
  NATIVE=0   # use the standalone astroshot commands in this skill
fi
```

`archdev shots doctor` exits 0 only when the feature is installed and enabled
(`archdev settings set shots on`). Any other result means: use the standalone
path. The rest of this skill stays correct as the fallback.

| Standalone | Native |
|---|---|
| `astroshot-capture --feature … --source …` | `archdev shots capture --feature … --source …` |
| `astroshot-capture … --from-agent-browser "$SESSION"` | `archdev shots capture … --from-agent-browser "$SESSION"` |
| `astroshot-capture … --status pass --finalize` | `archdev shots capture … --status pass --finalize` |
| `astroshot react\|ink\|pty\|movie\|review\|doctor\|init\|demo` | `archdev shots react\|ink\|pty\|movie\|review\|doctor\|init\|demo` |
| read `manifest.json` and `review.json` by hand | `archdev shots status [--feature <f>] --json` |

`archdev shots capture` takes the same flags as the bash helper (`--feature`,
`--slug`, `--source` or `--from-agent-browser`, `--title`, `--description`,
`--url`, `--viewport`, `--status`, `--run-id`, `--root`, `--finalize`) and prints
the destination path on stdout. `archdev shots react|ink|pty <fixture>
--feature <f> --slug <s>` renders and publishes into `.astroshot/<f>/` in one
step.

To bring review into the ArchCode web UI, `archdev shots upload --feature <f>`
uploads a run, and `archdev shots status --remote` or `archdev shots pull`
bring web review comments back into `review.json`.

## Choose the capture source

| Frame must prove | Capture skill |
|---|---|
| Nothing yet — only that the pipe works | **`astroshot demo`** (zero prerequisites) |
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
- Do **not** put one-off harness frames under `.astroshot/stories/` or
  `.astroshot/friction-logs/`. Both are reserved names for agentic user-story
  scenarios (see the **user-story** skill; `friction-logs` is the earlier name
  and is still read). Those runs appear in the tray's user stories tab, not the
  Shots stream. The terminal tray will label that tab `User stories`; the macOS app
  may still label it Friction Logs until it is updated.

Read [the manifest and review contract](references/manifest.md) whenever
writing a custom integration or interpreting feedback. Read
[harness integration](references/harness-integration.md) when dual-writing
from a Bash or agent-browser harness.

## Use the capture helper

With native support (see above), run `archdev shots capture` with the same
flags and skip the lookup below. Otherwise resolve `astroshot-capture` from an explicit override, a project install, a
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

When `archdev shots` is available, read review state with:

```bash
archdev shots status --feature <feature> --json
```

It prints `{ root, features: [{ feature, run_id, status, shots: [{ file, slug,
title, kind, state, comments: [{id, body, created_at}] }] }], totals }`, where
`state` is `seen`, `stale`, or `unseen`, and it applies the hash and run
scoping below for you. Report every comment it returns. Without `--feature` it
covers every feature under the root.

Fallback when `archdev shots` is not available: apply the rules by hand. For
every current-run frame:

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

Never edit `review.json` to mark your own work Seen, with or without
`archdev shots`. Address feedback, capture new bytes, and let the human review
the new hash.

## Operate and troubleshoot the app

Install a signed build from GitHub Releases, or from a clone:

```bash
cd macos
./scripts/bootstrap.sh
open Astroshots.xcodeproj
```

`npx astroshot review` opens the same stream in a terminal (Ghostty, kitty, or
WezTerm draw the pictures; ffmpeg plays movies). It reads and writes the same
`review.json`, so a human can review from a shell session and agents read the
feedback the same way. See `docs/review-tui.md`.

The app is menu-bar only. Configure watched folders and overlay visibility
from its gear menu.

Start every investigation with the diagnostics instead of guessing:

```bash
astroshot doctor          # required/optional checks plus the exact fix command
astroshot doctor --json   # same report, machine readable
astroshot demo            # seed real frames to prove the path end to end
```

`doctor` reads the app's live watched-folder configuration, so "empty tray"
resolves into three distinct answers with different fixes: first-launch setup
never completed, setup completed but this project is outside every watched
folder, or the project is watched and the problem is elsewhere. It is read-only
— it never installs anything or changes app state.

| Symptom | Check |
|---|---|
| Empty tray | `astroshot doctor` — it names the watch-root state and the fix |
| Unsure the contract works at all | `astroshot demo`, then open **Shots** |
| No overlay | App is running, overlay is enabled, and the file has settled |
| Wrong project badge | `.astroshot` is at the intended worktree root |
| Manifest metadata missing | JSON parses and `file` matches the image basename |
| Corrupt preview | Producer writes atomically; the helper does |

Report the feature, worktree, execution result, current review state, every
applicable comment, and the `.astroshot/<feature>/` path.
