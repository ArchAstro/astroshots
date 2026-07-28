---
name: screenshot
description: >
  Plan, generate, review, and maintain product screenshots for documentation.
  Use react-shot for isolated React UI, tui-shot for Ink or arbitrary PTY
  terminal UI, agent-browser for real application journeys, and Astroshots to
  review the resulting documentation image set.
---

# Screenshot documentation

Create documentation images that are reproducible, synthetic, focused, and
reviewed. Choose the capture boundary before writing fixtures or starting an
application.

## Choose the capture tool

| Documentation image | Tool |
|---|---|
| React dialog, form, panel, empty state, or component with fixed props | **react-shot** |
| Ink screen, wizard step, command center, or terminal state | **tui-shot** |
| Ratatui, Bubble Tea, Textual, curses, or another executable terminal program | **tui-shot** with `astroshot pty` |
| Complete application shell, authenticated page, routing, or live data | **agent-browser** |
| Multi-image review journey for any of the above | **astroshots** |

Use component fixtures when props can express the state. Use a real browser
when the screenshot must prove application boundaries. Do not rebuild an
entire application shell inside a component fixture merely to avoid starting
the application.

Read the matching **react-shot**, **tui-shot**, **agent-browser**, and
**astroshots** skills for their complete command and fixture contracts.

## Audit the documentation contract

Before capturing:

1. Find the documentation content root and public asset directory. Do not
   assume a framework-specific path.
2. Read two nearby pages to learn image syntax, naming, dimensions, and voice.
3. Find an existing screenshot inventory or playbook. If none exists, keep a
   small manifest beside the fixtures or documentation.
4. List the exact reader question answered by each planned image.
5. Reuse an existing fixture or capture journey when it already proves the
   intended state.
6. Find the documented command that builds or previews the target docs site.

Keep fixture sources and batch manifests in version control. A committed PNG
without a reproducible source becomes stale documentation.

## Plan the image set

For every image, record:

- stable asset filename;
- destination under the documentation site's public asset directory;
- capture tool and fixture, manifest, or route;
- viewport or terminal dimensions;
- meaningful selector or expected text;
- synthetic data required;
- alt text describing what the reader learns.

Prefer the smallest set that explains the workflow. Do not capture every click
when one image can establish the state.

## Capture isolated React documentation

Install Chromium once:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot install-browser
```

Capture a fixture directly into the documentation asset directory:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot react ./fixtures/account-dialog.tsx \
  -o ./docs/public/screenshots/account-dialog.png
```

Use `batch <manifest.yaml|json>` for a maintained image set. Fixtures should
use stable props, local providers, mocked server modules, a focused selector,
and a content-based readiness condition. Crop dialogs and panels to the
meaningful element; avoid empty canvas and opaque squares around rounded
corners.

## Capture Ink terminal documentation

Install Ink's project-local peers and Chromium once:

```bash
npm install --save-dev ink@^7.1 react@^19
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot install-browser
```

Generate and capture a fixed Ink state:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init ink ./fixtures/install-confirmation.tsx
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot ink ./fixtures/install-confirmation.tsx \
  -o ./docs/public/screenshots/install-confirmation.png
```

Use stable `cols`, `rows`, and `expectText` values. Prefer a fixture for a
known visual state.

For Ratatui or any executable terminal UI, use a real PTY fixture:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init pty ./fixtures/install-confirmation.yaml
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot pty ./fixtures/install-confirmation.yaml \
  -o ./docs/public/screenshots/install-confirmation.png
```

Prefer `waitFor` actions over sleeps, script the minimum meaningful input, and
assert distinctive final `expectText`. Keep the PTY fixture with the docs so
the screenshot remains reproducible.

## Capture a real application journey

Discover the application URL from its documentation or environment and
health-check the real service; do not invent a port. Use a named agent-browser
session so cookies and page state remain isolated:

```bash
test -n "${URL:-}" || {
  echo "Set URL from the application's documented local or staging address" >&2
  exit 1
}
curl -fsS -o /dev/null "${HEALTH_URL:-$URL}"
SESSION="docs-$(basename "$(git rev-parse --show-toplevel)")"
cleanup_browser() {
  agent-browser --session "$SESSION" close >/dev/null 2>&1 || true
}
trap cleanup_browser EXIT

agent-browser --session "$SESSION" set viewport 1280 900
agent-browser --session "$SESSION" open "$URL"
agent-browser --session "$SESSION" wait --load networkidle
agent-browser --session "$SESSION" snapshot -i
# Wait for and assert a product-specific selector or text from the snapshot.
agent-browser --session "$SESSION" screenshot \
  '[role=dialog]' ./docs/public/screenshots/account-dialog.png
agent-browser --session "$SESSION" close
trap - EXIT
```

Inspect the interactive snapshot before acting. Wait for the final product
state rather than treating network idle as sufficient. Prefer an element crop
when the application shell is not instructional, and never ship loading
skeletons or harness credentials. Close the session after capture, or report
explicitly that it remains open for debugging.

## Review the set with Astroshots

On macOS, confirm Astroshots is running and one of its watched folders includes
the current project. Astroshots is an optional review surface, not proof that a
human saw the images. On non-macOS systems or when the viewer is unavailable, open every
PNG with the agent's image inspection tool or a local image viewer instead.

Resolve a caller override first, then check project-local, global, and clone
locations for the capture helper:

```bash
CAPTURE="${CAPTURE:-}"
if [[ ! -x "$CAPTURE" ]]; then
  for candidate in \
    "./bin/astroshot-capture" \
    "./.agents/skills/astroshots/scripts/astroshot-capture" \
    "./.claude/skills/astroshots/scripts/astroshot-capture" \
    "./.codex/skills/astroshots/scripts/astroshot-capture" \
    "$HOME/.agents/skills/astroshots/scripts/astroshot-capture" \
    "$HOME/.claude/skills/astroshots/scripts/astroshot-capture" \
    "$HOME/.codex/skills/astroshots/scripts/astroshot-capture"; do
    if [[ -x "$candidate" ]]; then
      CAPTURE="$candidate"
      break
    fi
  done
fi
if [[ ! -x "$CAPTURE" ]]; then
  echo "Install the astroshots skill or set CAPTURE to its executable" >&2
  exit 1
fi
```

Use a unique feature for each review run so stale frames cannot be mistaken for
the current image set. Add each documentation asset to that journey:

```bash
FEATURE="docs-account-guide-$(date -u +%Y%m%dT%H%M%SZ)-$$"

"$CAPTURE" \
  --feature "$FEATURE" \
  --slug account-dialog \
  --title "Edit account details" \
  --description "The form fields and save action used by this step." \
  --source ./docs/public/screenshots/account-dialog.png

# This status describes successful screenshot generation, not human approval.
"$CAPTURE" \
  --feature "$FEATURE" \
  --status pass \
  --finalize
```

For a browser session, `--from-agent-browser "$SESSION"` can capture and add
the review frame in one operation; it does not replace the documentation asset
written by the earlier screenshot command. `manifest.json.status` describes
only capture execution. It may be `pass` while human review is still pending or
has requested changes.

Astroshots writes human decisions and comments separately to
`.astroshot/<feature>/review.json`. Entries are keyed by exact image filename:

```json
{
  "version": 1,
  "run_id": "docs-account-guide-20260726T174200Z-48291",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0001-account-dialog.png": {
      "decision": "changes_requested",
      "reviewed_at": "2026-07-26T17:42:00Z",
      "image_sha256": "a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1",
      "comments": [
        {
          "id": "A1B2C3D4-E5F6-47A8-9000-111122223333",
          "body": "Increase the width so the primary action is visible.",
          "created_at": "2026-07-26T17:41:32Z"
        }
      ]
    }
  }
}
```

Only a human reviewer sets `approved` or `changes_requested`. A comment-only
entry may omit `reviewed_at` and `image_sha256` and remains pending. Before
reporting a decision, calculate the current file's SHA-256 and compare it with
`image_sha256`. Changed bytes make the decision stale/pending, but comments
remain agent-readable instructions for that run. When the manifest has a run
id, feedback with a missing or different `review.json.run_id` does not apply to
the current run: report pending and do not carry its comments forward. Read and
report the exact-filename entry with `jq`; never manufacture approval or infer
it from an automated test:

```bash
DIR=".astroshot/$FEATURE"
FILE="0001-account-dialog.png"
MANIFEST="$DIR/manifest.json"
REVIEW="$DIR/review.json"
if command -v sha256sum >/dev/null 2>&1; then
  SHA="$(sha256sum "$DIR/$FILE" | awk '{print $1}')"
else
  SHA="$(shasum -a 256 "$DIR/$FILE" | awk '{print $1}')"
fi
MANIFEST_RUN_ID="$(jq -r '.run_id // empty' "$MANIFEST" 2>/dev/null || true)"

if [[ ! -f "$REVIEW" ]]; then
  jq -n '{state: "pending", comments: []}'
else
  jq --arg file "$FILE" --arg sha "$SHA" \
    --arg manifest_run_id "$MANIFEST_RUN_ID" '
    if $manifest_run_id != "" and (.run_id // "") != $manifest_run_id then
      {state: "pending", comments: []}
    else
      (.reviews[$file] // null) as $r
      | {
          state: (
            if $r == null then "pending"
            elif $r.decision == null then "pending"
            elif $r.image_sha256 != $sha then "stale"
            else $r.decision
            end
          ),
          comments: ($r.comments // [])
        }
    end
  ' "$REVIEW"
fi
```

When changes are requested, address every comment, regenerate the documentation
asset, stream the new frame, and wait for the human to review its new hash.

## Embed and render the documentation

Capturing the asset is not the end of the documentation task:

1. Add or update the image reference on the intended documentation page.
2. Write alt text that describes the state, choice, or action the reader
   should understand—not “screenshot of.”
3. Update the screenshot inventory with the asset, source fixture or journey,
   regeneration command, dimensions, and owning page.
4. Run the repository's documented docs build or preview command.
5. Open the rendered page at desktop and narrow widths. Confirm the exact path
   and filename case resolve, the image remains legible, and its crop supports
   the surrounding prose.

## Documentation quality gate

Open every PNG and check:

- the instructional state is visible without unrelated chrome;
- text is readable at the rendered documentation width;
- layout is not clipped, wrapped unexpectedly, or still loading;
- terminal colors and line drawing remain legible;
- synthetic names and values are consistent across the page;
- no credentials, customer data, internal hostnames, test emails, or
  worktree-specific identifiers appear;
- transparent corners and crops render cleanly;
- the file path and case exactly match the documentation reference.

Update the screenshot inventory whenever the fixture, route, asset name, or
regeneration command changes.

## Trust boundary

React and Ink fixtures, configuration, imports, and browser pages can execute
code with the current user's permissions. Review untrusted changes before
capture. Pin npm package versions in CI or other security-sensitive
automation, and review package provenance before first use.

## Report

Tell the human:

- which tool and source produced each image;
- the documentation asset paths and pages that reference them;
- what each frame proves;
- the visual checks performed;
- the Astroshots feature made available for review and whether a human
  approved it, requested changes, or has not reviewed the current hash;
- every agent-readable review comment and the image filename it targets;
- the docs build or preview command and rendered pages checked;
- any images that could not be reproduced or reviewed.

Do not commit or publish documentation images until the human has reviewed the
final frames.
