---
name: browser-ui-harness
description: >
  Design and implement browser UI test harnesses (especially Bash + agent-browser):
  runner vs case split, named sessions, bounded waits, product assertions, evidence
  (screenshots, snapshots, reports), cleanup, and live review via .astroshot/.
  Use when building smoke/e2e harnesses, writing run.sh + cases/, or defining UI
  testing best practices for agents driving real browsers.
---

# Browser UI harness best practices

Generic patterns for **human-readable browser journeys** driven by a CLI
(typically **agent-browser**). Distilled from production-style smoke harnesses:
a runner owns infrastructure; cases own product steps.

Install this skill:

```bash
# Global
npx skills add ArchAstro/astroshots --skill browser-ui-harness -g -y

# This project only
cd /path/to/your/project
npx skills add ArchAstro/astroshots --skill browser-ui-harness -y
```

Pair with:

```bash
npx skills add ArchAstro/astroshots --skill agent-browser -g -y
npx skills add ArchAstro/astroshots --skill astroshots -g -y   # live screenshot stream
```

---

## Goals of a good harness

| Goal | Practice |
|------|----------|
| Prove a **product journey** | Steps a human would recognize (“signed in”, “modal open”) |
| Stay **debuggable** | Screenshots + interactive snapshots + a Markdown report |
| Stay **repeatable** | Stable fixtures + run-unique names for created data |
| Fail **loudly with evidence** | On timeout/assert fail, capture page + console + errors |
| Keep cases **thin** | No env discovery or browser lifecycle inside case files |
| Support **live review** | Write frames under `.astroshot/<feature>/` for Astroshots |

Unit/component tests (Playwright fixtures, react-shot, etc.) stay separate.
This skill is for **end-to-end journeys against a running app**.

---

## Architecture: runner vs cases

```text
test-harness/browser/          # name as you like
  run.sh                       # runner: env, session, login, helpers, cleanup
  cases/
    install-wizard.sh          # case: CASE_DESCRIPTION + run_case()
    checkout.sh
```

### Runner owns

- Discovering base URLs / ports from **this project’s** config (never hardcode)
- Health checks (“is the app up?”)
- Creating an isolated **named** agent-browser session
- Auth bootstrap if needed (prefer real UI login when that is the product)
- Helpers: wait, assert, capture, step logging
- Artifact directories + Markdown report
- Cleanup of browser session and **run-created** resources
- Optional: refuse non-loopback URLs when the harness mutates local-only data

### Case owns

```bash
#!/usr/bin/env bash
CASE_DESCRIPTION="One sentence: what this journey proves."

run_case() {
  smoke_step "Open the pricing page"
  smoke_browser open "$APP_BASE_URL/pricing"
  smoke_wait_text "Plans" 20
  smoke_capture "pricing" "Plan cards are visible."

  smoke_step "Start checkout"
  smoke_click_text "Get started"
  smoke_wait_text "Checkout" 15
  smoke_assert_url '/checkout'
  smoke_capture "checkout" "Checkout form is ready."
}
```

**Rule:** setup and evidence mechanics live in `run.sh`; product clicks and
assertions live in cases.

---

## Session and isolation

```bash
REPO="$(basename "$(git rev-parse --show-toplevel)")"
CASE_NAME="install-wizard"
RUN_ID="${CASE_NAME}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
SESSION="${SESSION:-ui-${REPO}-${RUN_ID}}"
```

- One session per run; pass it into every `agent-browser` call.
- Close the session on exit unless `HOLD=1` (leave open for human inspection).
- Prefer a **fresh** session for smoke (no leftover cookies) unless the case
  is explicitly about resume/persistence.

---

## Helper contract (implement in the runner)

Name them as you like; keep the semantics:

| Helper | Behavior |
|--------|----------|
| `smoke_browser …` | `agent-browser --session "$SESSION" "$@"` + log |
| `smoke_step "…"` | Numbered section in report + log |
| `smoke_wait_js "label" 'expr' [seconds]` | Poll until true or fail with evidence |
| `smoke_wait_text "…"` | Wait until body text includes string |
| `smoke_assert_text` / `smoke_assert_url` | Hard fail if missing |
| `smoke_click_text "…"` | Click visible button/link by text (prefer dialog scope when open) |
| `smoke_capture "slug" "what it proves"` | Screenshot + optional a11y snapshot + report lines |
| `smoke_fail "…"` | Diagnostics then non-zero exit |

### Waits

- Bound every wait (default 10–20s). Infinite waits are bugs.
- Prefer **product signals** (heading text, URL, enabled button) over fixed `sleep`.
- Short settle (`wait 300–500`) only after known animations before a screenshot.

### Assertions

- Assert **outcomes** (“All set.” visible, URL under `/networks/`) not implementation details.
- On failure: full-page screenshot, `snapshot`, `console`, `errors` into the artifact dir.

### Captures

Each capture should answer: **what does this frame prove?**

```bash
smoke_capture "destination-default" \
  "Destination and ACL choices are presented before install."
```

Also dual-write for live review when Astroshots is in play:

```bash
# Prefer astroshot-capture after install of the astroshots skill
astroshot-capture --feature "$CASE_NAME" --slug "$slug" \
  --description "$description" \
  --status running \
  --from-agent-browser "$SESSION"
# at end:
astroshot-capture --feature "$CASE_NAME" --status pass --finalize
```

Layout (see **astroshots** skill):

```text
$REPO_ROOT/.astroshot/<feature>/
  manifest.json
  0001-slug.png
```

---

## Data and repeatability

| Do | Don't |
|----|--------|
| Stable **catalog/fixture** resources the product always has | Depend on yesterday’s manual UI state |
| **Run-unique** names for anything you create (`Smoke ${RUN_ID}`) | Fixed names that collide on re-run |
| Clean up run-created rows (best effort in `trap`) | Leave infinite garbage in shared local DBs |
| Keep secrets out of argv and logs | Echo magic links, tokens, or passwords |

---

## Safety for local harnesses

If the harness hits **local-only** setup endpoints or mutates a shared DB:

- Default to **loopback** base URLs only.
- Require an explicit escape hatch for remote targets (`ALLOW_REMOTE=1`).
- Document that the harness **writes data** and what it cleans up.

---

## Report shape

Write a single `report.md` as you go:

```markdown
# Browser smoke report

- Case: `install-wizard`
- Run: `install-wizard-20260724T…`
- App: `http://localhost:3000`
- Session: `ui-…`

## 01. Sign in
- Fresh session authenticated.
- Screenshot: `screenshots/01-signed-in.png`

## Result
PASS
```

Humans should understand the run without reading the shell log.

---

## Runner skeleton (outline)

```bash
#!/usr/bin/env bash
set -euo pipefail
# 1. parse case name, resolve APP_BASE_URL from env/files
# 2. health check or exit with clear error
# 3. mkdir artifacts; start report header
# 4. trap cleanup → diagnostics on fail; close session unless HOLD=1
# 5. source cases/$case.sh; require run_case
# 6. optional login_with_real_ui
# 7. run_case
# 8. append PASS; exit 0
```

Keep `run.sh` under a few hundred lines by pushing product detail into cases.

---

## How this relates to other tools

| Tool | Role |
|------|------|
| **agent-browser** | Drive one browser session (see **agent-browser** skill) |
| **Playwright test runner** | Large suites, parallel specs, CI gates |
| `@archastro/astroshot react` from npmjs | Isolated React docs PNGs without a full stack |
| `@archastro/astroshot ink` from npmjs | Isolated Ink terminal PNGs without a PTY |
| `@archastro/astroshot pty` from npmjs | Real Ratatui or arbitrary terminal-process PNGs |
| **Astroshots** | Live human review of harness frames |

Use a Bash agent-browser harness when you want a **readable journey + evidence**
agents and humans can both follow. Use Playwright when you need matrix scale and
strict CI parallelism.

---

## Checklist before you call a harness “done”

- [ ] Runner discovers URLs; cases do not hardcode ports
- [ ] Named session; closed on exit (or documented HOLD)
- [ ] Bounded waits + product-level asserts
- [ ] Failures dump screenshot + snapshot + console
- [ ] Captures include “what this proves”
- [ ] Run-unique data + cleanup
- [ ] Report.md written
- [ ] Optional: `.astroshot/<case>/` dual-write for live review

---

## Related

- **screenshot** skill — documentation asset planning and visual review
- **agent-browser** skill — CLI loop, snapshot, screenshots  
- **astroshots** skill — `.astroshot/` layout + `astroshot-capture`  
- **react-shot** skill — deterministic React component fixtures
- **tui-shot** skill — deterministic Ink and arbitrary PTY terminal fixtures
- CLI deep dive: `agent-browser skills get core --full`
