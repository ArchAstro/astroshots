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
| `description` | string | recommended | What was done / seen |
| `screenshots` | string[] | recommended | Basenames relative to the run directory |
| `screenshot` | string | no | Singular alias if only one image |
| `good` | string[] | recommended | Empty array allowed |
| `improve` | string[] | recommended | Empty array allowed; aliases: `can_improve`, `improvements` |
| `looks_good` | string[] | no | Alias for `good` |
| `url` | string | no | Route or UI context |
| `captured_at` | string | no | ISO-8601 UTC |

Screenshots are resolved by basename under the run directory. Missing files are
dropped from the loaded path list; the step still appears.

## App behavior

- Lists every slug with a `prompt.md` and/or at least one run.
- Default view uses the newest run (by directory mtime).
- Step table shows `step`, title, screenshot presence, good/improve counts.
- Selecting a step shows screenshots and the note lists.
- Prompt body is available as text in the detail view.
