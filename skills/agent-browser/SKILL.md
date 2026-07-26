---
name: agent-browser
description: >
  Drive real browsers with the agent-browser CLI — open pages, snapshot the
  accessibility tree, click/fill via @refs, wait for network idle, take
  screenshots. Use when exploring or testing a web UI, automating a browser
  journey, capturing UI states, or when the user mentions agent-browser,
  browser automation, snapshot -i, or headed browser debugging.
---

# agent-browser

`agent-browser` is a CLI for AI agents: a **persistent browser session** you
drive with short commands (navigate, snapshot, click, fill, screenshot).

## Install agent-browser (the CLI)

```bash
# npm (global)
npm install -g agent-browser

# or one-off
npx agent-browser --help
```

Confirm:

```bash
agent-browser --help
# Prefer version-matched guides shipped with the CLI:
agent-browser skills get core --full
```

Install browsers if the CLI prompts you (Playwright-backed; follow its install
instructions for Chromium).

### Install this skill

```bash
# Global (all projects)
npx skills add ArchAstro/astroshots --skill agent-browser -g -y

# This git project only
cd /path/to/your/project
npx skills add ArchAstro/astroshots --skill agent-browser -y
```

Sibling skills in the same package:

```bash
npx skills add ArchAstro/astroshots --skill astroshots -g -y
npx skills add ArchAstro/astroshots --skill browser-ui-harness -g -y
```

---

## Core model

1. **Named session** — cookies, tabs, and history persist across commands.
2. **navigate → wait → snapshot → act → verify** — never click blind.
3. **`snapshot -i`** — interactive tree with `@eN` refs; primary inspection tool.
4. **`wait --load networkidle`** before asserting or screenshotting SPAs.

### Session naming

Scope sessions so parallel agents/worktrees do not stomp each other:

```bash
REPO="$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)")"
SESSION="browse-${REPO}-$$"
# reuse SESSION for every agent-browser call in this journey
```

---

## Command loop

```bash
agent-browser --session "$SESSION" open "https://example.com/app"
agent-browser --session "$SESSION" wait --load networkidle
agent-browser --session "$SESSION" snapshot -i

# Use @refs from the snapshot
agent-browser --session "$SESSION" click @e5
agent-browser --session "$SESSION" fill @e3 "text"
agent-browser --session "$SESSION" press Enter

agent-browser --session "$SESSION" wait --load networkidle
agent-browser --session "$SESSION" snapshot -i
```

### Viewport

```bash
agent-browser --session "$SESSION" set viewport 1280 800
# taller for modals / long forms:
agent-browser --session "$SESSION" set viewport 1280 1100
```

### Screenshots

```bash
agent-browser --session "$SESSION" wait --load networkidle
agent-browser --session "$SESSION" screenshot /tmp/page.png
agent-browser --session "$SESSION" screenshot --full /tmp/full.png
agent-browser --session "$SESSION" screenshot '[role=dialog]' /tmp/modal.png
agent-browser --session "$SESSION" screenshot /tmp/annotated.png --annotate
```

Always `networkidle` (or an explicit ready condition) before capture so you do
not freeze loading skeletons.

For an isolated React component or Ink terminal state, do not start a full
application journey: use the npmjs-pinned `@archastro/react-shot` or
`@archastro/tui-shot` commands from their sibling skills. Use agent-browser
when the frame must prove the running application's routing, auth, data, or
shell.

### Live review with Astroshots

When a human is watching (or you want a durable project stream), prefer the
**astroshots** skill — write under `.astroshot/<feature>/`, not only `/tmp`:

```bash
# After installing: npx skills add ArchAstro/astroshots --skill astroshots -g -y
CAPTURE="$(ls -d ~/.agents/skills/astroshots/scripts/astroshot-capture \
  ~/.claude/skills/astroshots/scripts/astroshot-capture 2>/dev/null | head -1)"

"$CAPTURE" --feature my-journey --slug step-name \
  --title "Step name" \
  --description "What this frame proves." \
  --from-agent-browser "$SESSION"
```

### Forms and keys

```bash
agent-browser --session "$SESSION" fill 'input[name="email"]' "user@example.com"
agent-browser --session "$SESSION" type @e3 "appended"
agent-browser --session "$SESSION" click 'button[type="submit"]'
agent-browser --session "$SESSION" press Tab
agent-browser --session "$SESSION" select @e4 "value"
agent-browser --session "$SESSION" check @e6
agent-browser --session "$SESSION" scroll down 500
agent-browser --session "$SESSION" scrollintoview '.target'
```

### Read state

```bash
agent-browser --session "$SESSION" get url
agent-browser --session "$SESSION" get title
agent-browser --session "$SESSION" get text @e2
agent-browser --session "$SESSION" get attr href @e5
agent-browser --session "$SESSION" eval "document.body.innerText.includes('Ready')"
```

### Diagnostics

```bash
agent-browser --session "$SESSION" errors
agent-browser --session "$SESSION" console
agent-browser --session "$SESSION" network requests
agent-browser --session "$SESSION" network requests --filter "/api/"
```

### Headed debugging

```bash
agent-browser --session "$SESSION" --headed open "$URL"
```

### Cleanup

```bash
agent-browser --session "$SESSION" close
# or all sessions:
agent-browser --session "$SESSION" close --all
```

---

## Local apps

1. Discover base URL from the project’s env / docs — do not invent ports.
2. Health-check before opening the browser (`curl -sf "$URL/health"` or equivalent).
3. If the stack is down, tell the human how to start it; do not silently fail.
4. Prefer loopback URLs for harnesses that create or mutate data.

---

## Finding elements when snapshot is not enough

```bash
agent-browser --session "$SESSION" snapshot          # full tree
agent-browser --session "$SESSION" snapshot -s 'main'
agent-browser --session "$SESSION" is visible 'button:has-text("Save")'
agent-browser --session "$SESSION" find text "Submit"
agent-browser --session "$SESSION" find role button
```

---

## Report to the human

After a journey:

- What you verified (product outcome, not only “clicked button”)
- Screenshot / `.astroshot` paths if any
- Failures, console errors, unexpected URLs
- Whether the session is still open (`SMOKE_HOLD`-style) or closed

---

## Related

- CLI self-docs: `agent-browser skills get core --full`
- Documentation image workflow: **screenshot** skill (`npx skills add ArchAstro/astroshots --skill screenshot -g -y`)
- Live streams: **astroshots** skill (`npx skills add ArchAstro/astroshots --skill astroshots -g -y`)
- React fixtures: **react-shot** skill (`npx skills add ArchAstro/astroshots --skill react-shot -g -y`)
- Ink fixtures: **tui-shot** skill (`npx skills add ArchAstro/astroshots --skill tui-shot -g -y`)
- Bash harness design: **browser-ui-harness** skill
