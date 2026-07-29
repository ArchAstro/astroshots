---
name: agent-browser
description: >
  Drive one persistent real-browser session with the agent-browser CLI: open
  pages, inspect the accessibility tree, act through refs or selectors, debug,
  and capture screenshots. Use when the user requests agent-browser or a
  shell-driven browser journey, or when testing live routing, authentication,
  backend data, or application chrome. For reusable smoke/e2e harness
  architecture use browser-ui-harness; for isolated React or terminal fixtures
  use astroshot; for documentation image-set planning use screenshot.
---

# Agent-browser journeys

Use `agent-browser` for states that only a running application can prove. The
CLI ships its own version-matched command guide; load it instead of relying on
a copied command reference:

```bash
agent-browser --version
agent-browser skills get core --full
```

Install the CLI only when it is missing:

```bash
npm install -g agent-browser
agent-browser install
```

## Choose the boundary

| Proof | Tool |
|---|---|
| Routing, authentication, live data, or complete browser shell | `agent-browser` |
| Isolated React, Ink, or terminal executable state | **astroshot** |
| Reusable multi-case browser smoke harness | **browser-ui-harness** |
| Documentation image set and rendered-page verification | **screenshot** |
| Live review of an existing capture | **astroshots-review** |

## Journey loop

Create a session name unique to the repository and run:

```bash
REPO="$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)")"
SESSION="browse-${REPO}-$$"

agent-browser --session "$SESSION" open "https://example.com/app"
agent-browser --session "$SESSION" wait --load networkidle
agent-browser --session "$SESSION" snapshot -i
```

Then repeat:

1. Inspect with `snapshot -i` or a scoped snapshot.
2. Act through a current `@eN` ref or a stable semantic selector.
3. Wait for a product-level ready condition.
4. Snapshot again and assert the outcome.
5. Capture only meaningful states.

Do not click a stale ref or guess a selector. `find` performs an action and
defaults to clicking, so name its action explicitly:

```bash
agent-browser --session "$SESSION" find role button click --name "Submit"
agent-browser --session "$SESSION" find label "Email" fill "user@example.com"
```

Use read-only commands for inspection:

```bash
agent-browser --session "$SESSION" snapshot -s 'main'
agent-browser --session "$SESSION" is visible 'button:has-text("Save")'
agent-browser --session "$SESSION" get count 'button'
agent-browser --session "$SESSION" get url
```

`networkidle` is a useful transport settle, not proof that the product state is
ready. Wait for distinctive text, a selector, URL, or JavaScript condition
before asserting or capturing.

## Capture and diagnose

```bash
agent-browser --session "$SESSION" set viewport 1280 900
agent-browser --session "$SESSION" screenshot ./page.png
agent-browser --session "$SESSION" screenshot --full ./full-page.png
agent-browser --session "$SESSION" screenshot '[role=dialog]' ./dialog.png

agent-browser --session "$SESSION" errors
agent-browser --session "$SESSION" console
agent-browser --session "$SESSION" network requests --filter "/api/"
```

Use a focused crop when surrounding chrome is not part of the proof. Never
capture a loading skeleton, credentials, tokens, or customer data.

## Stream a frame to Astroshots

Read the **astroshots-review** skill and use its helper after the page reaches the
intended state:

```bash
"$CAPTURE" --feature my-journey --slug step-name \
  --title "Step name" \
  --description "What this frame proves." \
  --from-agent-browser "$SESSION"
```

Astroshots is optional review transport; it does not replace product
assertions and does not make a capture human-approved.

## Local applications and cleanup

Discover the base URL from the target repository's configuration or
documentation. Health-check it before opening a session; do not invent a port.
If the journey mutates data, prefer loopback targets and run-unique values.

Close the session unless the human asked to keep it for debugging:

```bash
agent-browser --session "$SESSION" close
```

Report the product outcome, unexpected URLs or diagnostics, capture paths,
Astroshots feature if used, and whether the browser session remains open.
