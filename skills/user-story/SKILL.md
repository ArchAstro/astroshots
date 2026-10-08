---
name: user-story
description: >
  Author, list, and run Astroshots user stories — agentic user-perspective UX
  scenarios that write JSONL steps with screenshots, good/improve notes, and a
  spoken transcript per step under .astroshot/stories/. Use when the user wants
  a user story, a friction log (the earlier name), a UX walkthrough from a clean
  user, a scenario prompt for the Astroshots user stories tab, narrated-video-ready
  step transcripts, or to run/list existing user stories in a worktree.
---

# User stories

A **user story** is a local agent run that walks a product the way a real user
would: explicit steps, clean environment, screenshots, and honest UX notes.

This feature was previously called **friction logs**. New stories go under
`.astroshot/stories/`. The old `.astroshot/friction-logs/` tree is still read,
and existing content there is never moved or deleted. If a slug exists in both
trees, the copy under `stories/` is the one that is read.

The review tray lists user stories in its second tab, separate from one-off
**Shots**. The terminal tray (`astroshot review`) will label that tab `User stories`.
The macOS app may still label it Friction Logs until it is updated. Prompts and
run output live under the worktree:

```text
.astroshot/stories/<slug>/
  prompt.md              # authored scenario (viewable in the tray)
  meta.json              # optional title / description / status
  runs/<run-id>/
    log.jsonl            # one JSON object per step
    meta.json            # optional per-run status
    0001-land-home.png
    0002-open-cart.png
```

One-off captures still use `.astroshot/<feature>/`; never put user-story runs
there. `stories` and `friction-logs` are both reserved names and never appear
in the Shots stream.

## Native `archdev shots` support

When `archdev shots` is available and enabled, use its `stories` commands
instead of the shell steps below. They read and write the same on-disk layout.
Detect it once per session:

```bash
if command -v archdev >/dev/null 2>&1 && archdev shots doctor >/dev/null 2>&1; then
  NATIVE=1   # archdev shots is installed and enabled
else
  NATIVE=0   # use the manual shell steps in this skill
fi
```

`archdev shots doctor` exits 0 only when the feature is installed and enabled
(`archdev settings set shots on`). Any other result means: use the manual steps.

| Task | Native command | Manual fallback |
|---|---|---|
| Create a story | `archdev shots stories new <slug> --title "<title>"` | `mkdir -p .astroshot/stories/<slug>` and write `prompt.md` |
| List stories | `archdev shots stories list [--json]` | the `find` loop under **List** |
| Show a story and its runs | `archdev shots stories show <slug> [--run <id>] [--json]` | read `prompt.md` and `runs/*/log.jsonl` |
| Start a new run directory | `archdev shots stories run-dir <slug>` (creates and prints it) | the shell block under **Run** |
| Upload for web review | `archdev shots stories upload <slug>` | none; review in the tray |

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

Create the story directory, then write its `prompt.md`:

```bash
# native (when archdev shots is available)
archdev shots stories new <slug> --title "<title>"

# manual fallback
mkdir -p .astroshot/stories/<slug>
```

Write `.astroshot/stories/<slug>/prompt.md` with:

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
   under `runs/<run-id>/`, with a **`transcript` on every step** (spoken narrative
   for later narrated video).

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
Write `runs/<run-id>/log.jsonl` and screenshots per the user-story run contract.
Every JSONL step must include a `transcript` (spoken narrative with flowing
transitions across steps).
```

After authoring, tell the human the path and that Astroshots will list the
prompt under **User stories** once the tray rescans (or on next FSEvent). The
macOS app may still show the tab as Friction Logs.

---

## List

From the worktree root (or any watched parent). With native support:

```bash
archdev shots stories list --json
archdev shots stories show <slug> --json
```

Manual fallback. It lists `stories/` and the legacy `friction-logs/` tree; when
a slug is in both, skip the `friction-logs/` copy:

```bash
# All user stories under this worktree
find .astroshot/stories .astroshot/friction-logs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort

# Prompt + every run per slug (flag empty stubs the app will hide)
for d in .astroshot/stories/*/ .astroshot/friction-logs/*/; do
  [ -d "$d" ] || continue
  slug=$(basename "$d")
  case "$d" in .astroshot/friction-logs/*) [ -d ".astroshot/stories/$slug" ] && continue ;; esac
  prompt="missing"; [ -f "$d/prompt.md" ] && prompt="prompt.md"
  echo "$slug  prompt=$prompt"
  if [ -d "$d/runs" ]; then
    for r in "$d/runs"/*/; do
      [ -d "$r" ] || continue
      rid=$(basename "$r")
      lines=$(grep -cve '^\s*$' "$r/log.jsonl" 2>/dev/null || echo 0)
      imgs=$(find "$r" -maxdepth 1 \( -name '*.png' -o -name '*.jpg' \) 2>/dev/null | wc -l | tr -d ' ')
      if [ "${lines:-0}" -eq 0 ] && [ "${imgs:-0}" -eq 0 ]; then
        echo "  run $rid  EMPTY (hidden in tray)"
      else
        echo "  run $rid  steps≈$lines  imgs=$imgs"
      fi
    done
  else
    echo "  runs=none"
  fi
done
```

Summarize for the human: slug, title (from meta or humanized slug), **all run
ids** (not only latest), which are empty stubs, and whether `log.jsonl` has steps. Note which stories exist only under the
legacy `friction-logs/` tree.

---

## Run

Execute an existing `prompt.md` (or author first if missing).

### 1. Prepare the run directory

**Every attempt is a new nested run.** Never append a second attempt into an
existing `runs/<id>/` folder, and never write `log.jsonl` only at the slug root
when you already have nested runs (the app prefers nested history).

With native support, this creates the run directory and prints its path:

```bash
SLUG="<slug>"                     # e.g. checkout-as-new-user
RUN_DIR="$(archdev shots stories run-dir "$SLUG")"
RUN_ID="$(basename "$RUN_DIR")"
```

Manual fallback. A story that exists only under `friction-logs/` keeps its
runs there; use `ROOT=".astroshot/friction-logs/$SLUG"` for it. New stories use
`stories/`:

```bash
SLUG="<slug>"                     # e.g. checkout-as-new-user
# Unique id — second resolution. Never reuse an existing runs/<id>/.
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
ROOT=".astroshot/stories/$SLUG"
RUN_DIR="$ROOT/runs/$RUN_ID"
i=0
while [ -e "$RUN_DIR" ]; do
  i=$((i + 1))
  RUN_DIR="$ROOT/runs/${RUN_ID}-$i"
done
RUN_ID="$(basename "$RUN_DIR")"
mkdir -p "$RUN_DIR"
: > "$RUN_DIR/log.jsonl"
echo "User story run dir: $RUN_DIR"
```

Set slug `meta.json` status to `running` while active; `complete` or `failed`
when finished. Optional per-run `runs/<id>/meta.json` with the same `status`
field is fine.

Astroshots lists **every** non-empty `runs/<id>/` under the slug (newest by
directory mtime first). The tray User stories row shows the run count; open the
log to switch between runs.

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
   (user stories do not use `astroshot-capture` feature folders).

4. **Append one JSON line** to `$RUN_DIR/log.jsonl` (never rewrite earlier lines
   mid-run unless correcting a failed partial write):

```json
{
  "step": 1,
  "id": "land-home",
  "title": "Land on homepage",
  "description": "Opened / with a clean session. Marketing hero and primary CTA are above the fold.",
  "transcript": "I land on the marketing home with a clean session. The hero and primary CTA sit above the fold and read clearly — the CTA even names the user outcome — but Sign in still fights the main action for attention, and the trust strip drops below the fold at this width. From here I follow the primary CTA into pricing.",
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
| `description` | yes | Short factual note: what you did + what you saw |
| `transcript` | **yes** | Spoken narrative for this step (see **Transcript rules** below) |
| `screenshots` | yes | Basename(s) next to `log.jsonl`; array may be empty only if capture failed (say so in description) |
| `good` | yes | Array of strings; use `[]` if nothing is praiseworthy |
| `improve` | yes | Array of strings; concrete UX friction, not vague “polish” |
| `url` | no | Route or context |
| `captured_at` | no | ISO-8601 UTC |

Honesty bar:

- Prefer specific, screenshot-grounded notes (“label truncates at this width”).
- Separate product bugs from taste when obvious.
- Empty `improve` is fine when the step is clean; do not invent issues.

### Transcript rules (required every step)

`transcript` is the **spoken voiceover** for this step. It will be stitched into a
narrated video of the user story. Write it for the ear, not as a bullet dump.

**What every transcript must cover (in prose):**

1. **Actions** — what you actually did (clicked, typed, navigated, waited).
2. **What was good** — strengths visible in the screenshot / flow.
3. **What was bad** — friction, confusion, bugs, missing feedback (or explicitly
   that the step was clean if `improve` is empty).

**Length & form**

- One short paragraph: typically **2–5 sentences**, ~40–90 words.
- Plain prose only — no markdown, bullets, step numbers, or “Step 3:” prefixes.
- First-person present (“I open…”, “I click…”) or close observational present.
- Weave `good` / `improve` into sentences; do not restate them as a list.

**Transitions (non-negotiable)**

Transcripts must **flow across steps** as one continuous narration when read in
order. Treat the run as a single script, cut into chapters:

| Position | How to open / close |
|----------|---------------------|
| First step | Orient: goal, persona, starting surface. End by pointing at the next action. |
| Middle steps | Open with a bridge from the previous step’s last beat (“From pricing I…”, “That lands me on…”). Close by teeing up what you do next. |
| Last step | Resolve the goal (or the blocker). No dangling “next I…” unless the run truly continues elsewhere. |

Anti-patterns (rewrite these):

- Cold restarts: “I am on the homepage.” after you already described arriving there.
- Isolated report cards: “Good: CTA. Bad: contrast.” with no action narrative.
- Cliffhangers mid-run with no handoff: ending a middle step with no forward link.
- Reading the `title` field aloud or saying “in this step.”

**Work the whole script**

After the last step is written, re-read every `transcript` in order. If the join
between two steps stumbles, edit those lines in `log.jsonl` before you finish the
run. Prefer light connective tissue over repeating setup each time.

**Example sequence (abridged)**

```text
Step 1: I land on marketing home with a clean session. The hero and primary CTA
are clear, though Sign in still competes for attention. I take the CTA into pricing.

Step 2: Pricing loads with three plans and readable CTAs. The free tier is easy
to scan, but annual billing toggles without confirming what changes. I start signup
from the middle plan.

Step 3: The signup form labels fields well and validates on blur. Submit stays
disabled with no explanation when the password is short — I fix that, create the
account, and land in an empty workspace ready for first project.
```

### 4. Finish

1. Update `meta.json` status to `complete` or `failed`.
2. Confirm the Astroshots tray **User stories** tab shows the slug (the macOS
   app may still label it Friction Logs); open it and click through steps.
3. Report to the human: path to `RUN_DIR`, step count, top 3 improve items
   across the run.

### Failure handling

- If the environment cannot be prepared, write a single JSONL step explaining
  the blocker (include a `transcript` that narrates the stop), set status
  `failed`, and stop.
- If one step fails, still append a step with screenshots of the failure state
  and a transcript that says what broke, and continue only if later steps
  remain meaningful.
- **Do not leave empty run stubs.** If you abort before any step is written
  (`log.jsonl` still empty / no PNGs), delete that run directory:

  ```bash
  # Only when the run never produced a step:
  if [ ! -s "$RUN_DIR/log.jsonl" ]; then rm -rf "$RUN_DIR"; fi
  ```

  Empty `runs/<id>/` dirs are **hidden** by the app (no steps and no images),
  which makes it look like only one run exists even when several attempts were
  started.

---

## Relationship to other skills

| Need | Skill |
|------|--------|
| Isolated React / Ink / PTY frame | **astroshot** |
| Live browser routing / auth | **agent-browser** |
| Reusable multi-case browser harness | **browser-ui-harness** |
| Stream one-off harness frames for human review | **astroshots-review** |
| User-story author / list / run | **user-story** (this skill) |

User stories are **agent-authored UX narrative**, not harness pass/fail.
`manifest.json` / `review.json` remain the one-off shot contract.
Do not replace required per-step screenshots with one `astroshot movie`
recording. The screenshots ground each note; the optional narrated MP4 is a
derived presentation generated by Astroshots after the run.

## Narrated videos (opt-in, Astroshots app)

When the human enables **Settings → Narration**, Astroshots can render an MP4
from a run: each step’s screenshot held for the spoken `transcript` via
**mlx-audio-swift** (MLX Swift + Qwen3-TTS on Apple Silicon). The app downloads
the model in the background and serializes render jobs in a queue.

Agents should still write high-quality `transcript` fields; they do **not**
invoke TTS themselves. Point humans at the run detail **Make narrated video**
control once narration status is Ready.

With native support, `archdev shots stories upload <slug>` uploads a story for
review in the ArchCode web UI.

Full JSONL and directory notes: [references/contract.md](references/contract.md).
