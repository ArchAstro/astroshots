# Astroshots manifest reference

Path: `<worktree>/.astroshot/<feature>/manifest.json`

## Top level

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `version` | number | no | Use `1` |
| `feature` | string | yes | Same as directory name |
| `run_id` | string | recommended | Stable for one harness run |
| `status` | string | recommended | `running` \| `pass` \| `fail` \| `idle` |
| `description` | string | no | Journey one-liner |
| `shots` | array | yes | Append-only during a run |

## Shot entry

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `id` | string | recommended | Matches `NNNN` in filename |
| `file` | string | yes | Basename only, e.g. `0002-configure.png` |
| `slug` | string | recommended | From filename |
| `title` | string | recommended | Short label in tray |
| `description` | string | recommended | What the frame proves |
| `captured_at` | string | no | ISO-8601 UTC |
| `url` | string | no | Route or UI context |
| `viewport` | string | no | e.g. `1280x1100` |

## Matching

The app matches a PNG to a manifest entry by, in order:

1. `file` equals basename
2. `id` equals sequence prefix (`0004`)
3. `slug` equals filename slug

If none match, title falls back to a humanized slug from the filename.
