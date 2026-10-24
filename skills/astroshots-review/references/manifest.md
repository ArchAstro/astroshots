# Astroshots manifest reference

Path: `<worktree>/.astroshot/<feature>/manifest.json`

`manifest.json` is written by a harness or capture helper. It describes
execution and image metadata. Human feedback is a separate sibling file:
`review.json`.

## Top level

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `version` | number | no | Use `1` |
| `schema_version` | number | no | Portable record contract; currently `1`. Additive; old readers ignore it. |
| `feature` | string | yes | Same as directory name |
| `scenario_id` | string | recommended | Stable scenario identity. Copies keep this value. Path, worktree, and git metadata are provenance, never this identity. |
| `run_id` | string | recommended | One attempt. Distinct attempts never share this id. |
| `status` | string | recommended | Harness execution only: `running` \| `pass` \| `fail` \| `idle` |
| `description` | string | no | Journey one-liner |
| `provenance` | object | no | `source_path`, `worktree`, `repo`, `commit`, `branch`, `dirty`, optional `task_id` / `pr`. `dirty: true` means captured bytes are not claimed to be `commit`. |
| `shots` | array | yes | Append-only during a run |
| `previous_runs` | array | no | Earlier attempts archived when a new `run_id` takes over the directory. Each item: `run_id`, `status`, optional `provenance`, `shots`. `scenario_id` stays; `provenance` above always describes the current run. A `run_id` never appears twice (current plus archived); `astroshot-capture` and the movie sink refuse to reuse an archived id, and the same `run_id` as the current run continues that attempt. |

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
| `entry_id` | string | recommended | Stable entry identity for this shot |
| `asset_id` | string | recommended | Stable identity for the still / poster bytes |
| `video_asset_id` | string | recommended | Movie video identity; omit for stills |

### Movie fields

A movie is a normal shot entry whose `file` is the poster used in the stream,
overlay, and review state:

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `kind` | string | recommended | `movie`; inferred when `video` is present |
| `video` | string | yes | Sibling WebM, MP4, or MOV basename |
| `duration_ms` | number | recommended | Duration shown in stream/detail |
| `source` | string | recommended | `browser`, `pty`, `desktop.window`, or `frames` |
| `chapters` | array | no | `{ "slug": "configured", "t_ms": 2800 }` entries |

Astroshots shows movie badges and duration in the Shots stream, supports a
Movies filter, and provides tray/full-screen playback. `file` intentionally
remains the poster filename so overlays, thumbnails, and `review.json` share
the still-image contract.

## Matching

The app matches a PNG to a manifest entry by, in order:

1. `file` equals basename
2. `id` equals sequence prefix (`0004`)
3. `slug` equals filename slug

If none match, title falls back to a humanized slug from the filename.

## Human review file

When `archdev shots` is available, `archdev shots status --json` reports the
state derived from this file (`seen`, `stale`, `unseen`) with the hash and
run-id scoping below already applied.

Path: `<worktree>/.astroshot/<feature>/review.json`

```json
{
  "version": 1,
  "run_id": "install-wizard-20260726T174200Z-48291",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0002-configure.png": {
      "decision": "seen",
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
| `decision` | string | no | Human acknowledgement: `seen` |
| `reviewed_at` | string | conditional | Required with `decision`; absent for comment-only unseen feedback |
| `image_sha256` | string | conditional | Required with `decision`; lowercase SHA-256 of the reviewed file bytes |
| `content_revision` | string | no | Optional digest over ordered title, description, transcript, url, notes, chapters and asset hashes (length-prefixed fields; `identity.ts` `contentRevision`). Never substitutes for `image_sha256`. Poster-only / log-only Seen stays valid without it. |
| `comments` | array | yes | Ordered agent-readable feedback |

Each comment has required string fields `id`, `body`, and `created_at`.

Review validity is content-addressed. Consumers first check whether `decision`
exists. Without one, the entry is unseen even if it contains comments and
omits `image_sha256`. With a decision, consumers calculate the current image
SHA-256 and compare it with `image_sha256`; a missing or mismatched hash
invalidates the decision to unseen/stale while comments remain readable. A
missing file or missing review entry is also unseen.

Reviews are also run-scoped. When `manifest.json` has a `run_id`, a
`review.json` with a missing or different `run_id` belongs to another run.
Consumers report unseen and suppress that other run's decision and comments,
even if the image bytes are identical. This matches the app and prevents an old
Seen acknowledgement from carrying into a new execution. `manifest.status` must never be
used as a substitute for human review.
