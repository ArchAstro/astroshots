# Friction log on-disk contract

## Layout

```text
<worktree>/.astroshot/friction-logs/<slug>/
  prompt.md
  meta.json                 # optional
  runs/<run-id>/
    log.jsonl
    meta.json               # optional per-run status
    NNNN-slug.png
```

### Flat run (also accepted)

If there are no nested `runs/` directories, the app accepts:

```text
.astroshot/friction-logs/<slug>/
  prompt.md
  log.jsonl
  NNNN-slug.png
```

and treats it as a single run with id `latest`. Prefer nested `runs/` for
history.

## Reserved namespace

`friction-logs` is not a shot feature. Images under this tree **never** appear
in the Astroshots **Shots** stream. Only the **Friction Logs** tab lists them.

## meta.json

```json
{
  "version": 1,
  "slug": "checkout-as-new-user",
  "title": "Checkout as new user",
  "description": "Fresh account, empty cart → paid order",
  "status": "ready",
  "updated_at": "2026-08-07T14:30:22Z"
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `version` | number | no | Use `1` |
| `slug` | string | no | Should match directory name |
| `title` | string | no | Tray title; default humanized slug |
| `description` | string | no | One-line summary |
| `status` | string | no | `draft` \| `ready` \| `running` \| `complete` \| `failed` |
| `updated_at` | string | no | ISO-8601 UTC |

## log.jsonl

One UTF-8 JSON object per line. Order by `step` ascending. Blank lines and
`#` comment lines are ignored.

```json
{
  "step": 1,
  "id": "land-home",
  "title": "Land on homepage",
  "description": "Opened / with a clean session.",
  "transcript": "I land on the marketing home with a clean session. The hero and primary CTA are above the fold and easy to read, though Sign in still competes with the main action. Next I follow the CTA into pricing.",
  "screenshots": ["0001-land-home.png"],
  "good": ["Primary CTA is obvious"],
  "improve": ["Trust strip is below the fold"],
  "url": "/",
  "captured_at": "2026-08-07T14:30:22Z"
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `step` | number | recommended | 1-based; defaults to line order |
| `id` | string | recommended | Stable kebab-case step id |
| `title` | string | recommended | Short label |
| `description` | string | recommended | What was done / seen (short factual note) |
| `transcript` | string | **required** (new runs) | Spoken-word narrative for this step; see skill for voice + transition rules. Older logs may omit it. |
| `screenshots` | string[] | recommended | Basenames relative to the run directory |
| `screenshot` | string | no | Singular alias if only one image |
| `good` | string[] | recommended | Empty array allowed |
| `improve` | string[] | recommended | Empty array allowed; aliases: `can_improve`, `improvements` |
| `looks_good` | string[] | no | Alias for `good` |
| `url` | string | no | Route or UI context |
| `captured_at` | string | no | ISO-8601 UTC |

### `transcript` (narration)

- Plain prose only — no markdown, bullets, or step numbers.
- One short spoken paragraph (typically 2–5 sentences).
- Must cover **actions taken**, what **worked**, and what **did not**.
- Across a run, concatenated transcripts must read as one continuous voiceover
  (step *n* bridges into step *n+1*; do not restart cold each line).
- The app loads missing `transcript` as empty for backward compatibility.

Screenshots are resolved by basename under the run directory. Missing files are
dropped from the loaded path list; the step still appears.

## App behavior

- Lists every slug with a `prompt.md` and/or at least one run.
- Loads **all** nested `runs/<run-id>/` directories that have at least one
  parsed JSONL step **or** at least one image. Empty stubs (empty `log.jsonl`,
  no PNGs) are omitted.
- Runs are ordered newest-first by **run directory mtime**.
- Default selection is the newest run; the tray detail shows a run picker for
  every loaded run so history is switchable.
- List rows show the run count and the latest run id.
- Step table shows `step`, title, screenshot presence, good/improve counts.
- Selecting a step shows screenshots, transcript (if present), and note lists.
- Prompt body is available as text in the detail view.

## How new runs appear

1. Agent creates `runs/<new-unique-id>/` (never reuses an old id).
2. Writes `log.jsonl` lines + PNGs into that directory.
3. FSEvents on anything under `.astroshot/friction-logs/` schedules a full
   friction-log rescan; the loader re-reads every slug’s `runs/*`.
4. The tray updates the scenario’s run list; open the log and pick a run chip.
