---
name: friction-log
description: >
  Author, list, and run Astroshots friction logs — agentic user-perspective UX
  scenarios that write JSONL steps with screenshots plus good/improve notes under
  .astroshot/friction-logs/. Use when the user wants a friction log, UX walkthrough
  from a clean user, scenario prompt for the Astroshots Friction Logs tab, or to
  run/list existing friction logs in a worktree.
---

# Friction logs

A **friction log** is a local agent run that walks a product the way a real user
would: explicit steps, clean environment, screenshots, and honest UX notes.

Astroshots shows them in the tray **Friction Logs** tab (separate from one-off
**Shots**). Prompts and run output live under the worktree:

```text
.astroshot/friction-logs/<slug>/
  prompt.md              # authored scenario (viewable in the app)
  meta.json              # optional title / description / status
  runs/<run-id>/
    log.jsonl            # one JSON object per step
    0001-land-home.png
    0002-open-cart.png
```

One-off captures still use `.astroshot/<feature>/` — never put friction-log
runs there. The app ignores `friction-logs` in the Shots stream.

## Choose the mode

| User intent | Do this |
|---|---|
| Design a scenario / write the prompt | **Author** |
| See what logs exist | **List** |
| Execute a scenario and produce JSONL | **Run** |

---

## Author

Write a prompt a coding agent can execute without inventing the journey.

### Prompt file

Create:

```bash
mkdir -p .astroshot/friction-logs/<slug>
```

Write `.astroshot/friction-logs/<slug>/prompt.md` with:

1. **Goal** — one sentence, user outcome (not implementation).
2. **Persona** — who the user is (new, free tier, admin, etc.).
3. **Environment** — clean user/session requirements (fresh account, empty cart,
   locale, base URL). Prefer ephemeral fixtures over reusing the operator’s login.
4. **Preconditions** — data that must exist before step 1.
5. **Steps** — numbered, each from the user’s POV:
   - action the user takes
   - what “done” looks like for that step
   - where to capture (viewport / route)
6. **Out of scope** — what not to test.
7. **Output contract** — remind the runner to append JSONL lines + PNGs only
   under `runs/<run-id>/`.

Optional `meta.json`:

```json
{
  "version": 1,
  "slug": "checkout-as-new-user",
  "title": "Checkout as new user",
  "description": "Fresh account, empty cart → paid order",
  "status": "ready"
}
```

`status`: `draft` | `ready` | `running` | `complete` | `failed`.

### Authorship rules

- Steps are **user actions**, not “call the API” or “assert in code”.
- Keep 5–12 steps; split huge journeys into multiple slugs.
- Name the slug kebab-case: `invite-teammate`, `upgrade-billing`.
- Prefer real routes the runner can open (local app, staging URL, or
  agent-browser session). For fixed components only, use **astroshot**; for
  routing/auth/live data use **agent-browser** / **browser-ui-harness**.
- Do not put secrets in `prompt.md`. Reference env vars by name.

### Example prompt skeleton

```markdown
# Checkout as new user

## Goal
Complete a paid checkout with a brand-new account and empty cart.

## Persona
First-time buyer on the free marketing site.

## Environment
- Base URL: $APP_URL (local)
- Clean browser profile / no prior cookies
- Create a throwaway user during the run; do not reuse the operator account

## Preconditions
- Catalog has at least one purchasable SKU
- Stripe test mode (or mock checkout) enabled

## Steps
1. Land on `/` — marketing home is visible; capture hero + primary CTA.
2. Open pricing — plans and CTAs are readable.
3. Start signup from the primary CTA — form validation is clear.
…

## Out of scope
Admin dashboards, refunds, mobile native shells.

## Output
Write `runs/<run-id>/log.jsonl` and screenshots per the friction-log run contract.
```

After authoring, tell the human the path and that Astroshots will list the
prompt under **Friction Logs** once the tray rescans (or on next FSEvent).

---

## List

From the worktree root (or any watched parent):

```bash
# All friction logs under this worktree
find .astroshot/friction-logs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort

# Prompt + latest run per slug
for d in .astroshot/friction-logs/*/; do
  [ -d "$d" ] || continue
  slug=$(basename "$d")
  prompt="missing"; [ -f "$d/prompt.md" ] && prompt="prompt.md"
  runs=$(find "$d/runs" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
  latest=$(ls -1 "$d/runs" 2>/dev/null | sort | tail -1)
  echo "$slug  prompt=$prompt  runs=$runs  latest=${latest:-none}"
done
```

Summarize for the human: slug, title (from meta or humanized slug), run count,
latest run id, whether `log.jsonl` exists.

---

## Run

Execute an existing `prompt.md` (or author first if missing).

### 1. Prepare the run directory

```bash
SLUG="<slug>"                     # e.g. checkout-as-new-user
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
ROOT=".astroshot/friction-logs/$SLUG"
RUN_DIR="$ROOT/runs/$RUN_ID"
mkdir -p "$RUN_DIR"
: > "$RUN_DIR/log.jsonl"
```

Set `meta.json` status to `running` while the run is active; `complete` or
`failed` when finished.

### 2. Clean environment

Before step 1:

- New browser profile / session (agent-browser named session is fine).
- Create or use a dedicated throwaway user when the product requires auth.
- Reset app state the prompt requires (empty cart, no onboarding flags).
- Do **not** use the developer’s personal logged-in session unless the prompt
  explicitly says so.

### 3. For each step

1. Perform the user action.
2. Wait until the “done” signal for that step is visible.
3. Capture a screenshot into `$RUN_DIR` with a zero-padded name:

   ```text
   0001-land-home.png
   0002-open-pricing.png
   ```

   Prefer the project capture helper / agent-browser / astroshot skill that
   already matches the surface. Copy or write the PNG **into the run dir**
   (friction logs do not use `astroshot-capture` feature folders).

4. **Append one JSON line** to `$RUN_DIR/log.jsonl` (never rewrite earlier lines
   mid-run unless correcting a failed partial write):

```json
{
  "step": 1,
  "id": "land-home",
  "title": "Land on homepage",
  "description": "Opened / with a clean session. Marketing hero and primary CTA are above the fold.",
  "screenshots": ["0001-land-home.png"],
  "good": [
    "Primary CTA is high contrast and labeled with the user outcome",
    "Hero copy states the product value in one glance"
  ],
  "improve": [
    "Secondary “Sign in” competes visually with the primary CTA",
    "Trust strip is below the fold on 1280×800"
  ],
  "url": "/",
  "captured_at": "2026-08-07T14:30:22Z"
}
```

Field rules:

| Field | Required | Notes |
|-------|----------|-------|
| `step` | yes | 1-based order |
| `id` | yes | kebab-case, stable within the run |
| `title` | yes | Short step label |
| `description` | yes | What you did + what you saw |
| `screenshots` | yes | Basename(s) next to `log.jsonl`; array may be empty only if capture failed (say so in description) |
| `good` | yes | Array of strings; use `[]` if nothing is praiseworthy |
| `improve` | yes | Array of strings; concrete UX friction, not vague “polish” |
| `url` | no | Route or context |
| `captured_at` | no | ISO-8601 UTC |

Honesty bar:

- Prefer specific, screenshot-grounded notes (“label truncates at this width”).
- Separate product bugs from taste when obvious.
- Empty `improve` is fine when the step is clean; do not invent issues.

### 4. Finish

1. Update `meta.json` status to `complete` or `failed`.
2. Confirm Astroshots tray → **Friction Logs** shows the slug; open it and
   click through steps.
3. Report to the human: path to `RUN_DIR`, step count, top 3 improve items
   across the run.

### Failure handling

- If the environment cannot be prepared, write a single JSONL step explaining
  the blocker, set status `failed`, and stop.
- If one step fails, still append a step with screenshots of the failure state
  and continue only if later steps remain meaningful.

---

## Relationship to other skills

| Need | Skill |
|------|--------|
| Isolated React / Ink / PTY frame | **astroshot** |
| Live browser routing / auth | **agent-browser** |
| Reusable multi-case browser harness | **browser-ui-harness** |
| Stream one-off harness frames for human review | **astroshots-review** |
| Friction-log author / list / run | **friction-log** (this skill) |

Friction logs are **agent-authored UX narrative**, not harness pass/fail.
`manifest.json` / `review.json` remain the one-off shot contract.

Full JSONL and directory notes: [references/contract.md](references/contract.md).
