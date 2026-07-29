# Astroshots manifest reference

Path: `<worktree>/.astroshot/<feature>/manifest.json`

`manifest.json` is written by a harness or capture helper. It describes
execution and image metadata. Human feedback is a separate sibling file:
`review.json`.

## Top level

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `version` | number | no | Use `1` |
| `feature` | string | yes | Same as directory name |
| `run_id` | string | recommended | Stable for one harness run |
| `status` | string | recommended | Harness execution only: `running` \| `pass` \| `fail` \| `idle` |
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

## Human review file

Path: `<worktree>/.astroshot/<feature>/review.json`

```json
{
  "version": 1,
  "run_id": "install-wizard-20260726T174200Z-48291",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0002-configure.png": {
      "decision": "approved",
      "reviewed_at": "2026-07-26T17:42:00Z",
      "image_sha256": "a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1",
      "comments": [
        {
          "id": "A1B2C3D4-E5F6-47A8-9000-111122223333",
          "body": "Ready to ship at this width.",
          "created_at": "2026-07-26T17:41:32Z"
        }
      ]
    }
  }
}
```

### Top level

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `version` | number | yes | `1` |
| `run_id` | string | no | Store-populated capture run association when known |
| `updated_at` | string | no | Store-populated ISO-8601 time after a review write |
| `reviews` | object | yes | Keys are exact image basenames |

### Review entry

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `decision` | string | no | Human decision: `approved` or `changes_requested` |
| `reviewed_at` | string | conditional | Required with `decision`; absent for comment-only pending feedback |
| `image_sha256` | string | conditional | Required with `decision`; lowercase SHA-256 of the reviewed file bytes |
| `comments` | array | yes | Ordered agent-readable feedback |

Each comment has required string fields `id`, `body`, and `created_at`.

Review validity is content-addressed. Consumers first check whether `decision`
exists. Without one, the entry is pending even if it contains comments and
omits `image_sha256`. With a decision, consumers calculate the current image
SHA-256 and compare it with `image_sha256`; a missing or mismatched hash
invalidates the decision to pending/stale while comments remain readable. A
missing file or missing review entry is also pending.

Reviews are also run-scoped. When `manifest.json` has a `run_id`, a
`review.json` with a missing or different `run_id` belongs to another run.
Consumers report pending and suppress that other run's decision and comments,
even if the image bytes are identical. This matches the app and prevents an old
approval from carrying into a new execution. `manifest.status` must never be
used as a substitute for human review.
