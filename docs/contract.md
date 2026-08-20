# The on-disk contract

Astroshots has no API, no account, and no cloud. Everything is files under
`.astroshot/` inside a folder you asked the app to watch. Any language or
harness can participate by writing this layout.

- **Harness or agent writes:** images, movie poster + video pairs, and the
  optional `manifest.json` execution state.
- **Astroshots writes:** `review.json` — human Seen state and feedback.

---

## Write layout

```text
<project>/.astroshot/<feature>/
  manifest.json          # optional execution state, written by the harness
  review.json            # optional human feedback, written by Astroshots
  0001-signed-in.png
  0002-configure.png
  0003-onboarding.png     # movie poster shown in the stream/overlay
  0003-onboarding.webm    # movie played by Astroshots
```

Project name is inferred from the folder that contains `.astroshot`. Feature is
the directory name under it.

---

## `manifest.json`

```json
{
  "version": 1,
  "feature": "install-wizard",
  "run_id": "install-wizard-…",
  "status": "running",
  "shots": [
    {
      "id": "0002",
      "file": "0002-configure.png",
      "slug": "configure",
      "title": "Configure",
      "description": "Configuration screen for the resource.",
      "url": "/solutions · dialog"
    }
  ]
}
```

Movie entries extend the same manifest shot shape; `file` remains the poster
used by the stream and review state:

```json
{
  "kind": "movie",
  "file": "0003-onboarding.png",
  "video": "0003-onboarding.webm",
  "duration_ms": 4200,
  "source": "browser",
  "chapters": [
    { "slug": "signed-in", "t_ms": 900 },
    { "slug": "configured", "t_ms": 2800 }
  ]
}
```

`manifest.json.status` reports only whether the capture journey is running,
passed, or failed. **It is never a human acknowledgement.**

---

## `review.json`

Seen state and feedback live separately from the manifest, keyed by the exact
image filename:

```json
{
  "version": 1,
  "run_id": "install-wizard-…",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0002-configure.png": {
      "decision": "seen",
      "reviewed_at": "2026-07-26T17:42:00Z",
      "image_sha256": "a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1",
      "comments": [
        {
          "id": "A1B2C3D4-E5F6-47A8-9000-111122223333",
          "body": "The primary action is clipped at this width.",
          "created_at": "2026-07-26T17:41:32Z"
        }
      ]
    }
  }
}
```

An absent decision means unseen; comment-only entries may also omit
`reviewed_at` and `image_sha256`.

### Hash scoping

`seen` applies only while `image_sha256` matches the current file bytes.
Replacing an image at the same path makes it unseen again; existing comments
remain readable so an agent can act on them.

### Run-id scoping

Feedback is scoped to `run_id`. When the manifest has a run id, a missing or
different `review.json.run_id` makes the current run unseen; the prior run's
acknowledgement and comments do not carry forward, even when the bytes match.

---

## Agent obligations

1. Never infer Seen from `manifest.json`.
2. Never manufacture an acknowledgement on a human's behalf.
3. Never rewrite or delete human feedback in `review.json`.
4. Finalizing capture (`--finalize`) closes execution state only; it marks
   nothing Seen.

The **astroshots-review** skill teaches this contract end to end — see
[`docs/skills.md`](skills.md).

---

## Reading Astroshots' own settings

A tool that needs to know **which folders Astroshots watches** must read the
app's preferences domain, and must read both watch-root keys in the right order
— reading only the current key silently reports "not configured" for an upgraded
install, and guessing the wrong domain prefix fails the same silent way. The
canonical domain and the full read contract are in
[`PREFERENCES.md`](PREFERENCES.md), enforced by
[`scripts/verify-preferences-contract.mjs`](../scripts/verify-preferences-contract.mjs).

---

## Proofs

| Script | What it guards |
|--------|----------------|
| [`scripts/verify-skills.sh`](../scripts/verify-skills.sh) | Canonical documentation and skill proof; CI runs its integration mode, which captures React, Ink, and PTY images, streams them through `astroshot-capture`, and asserts the resulting manifest |
| [`scripts/verify-packages.mjs`](../scripts/verify-packages.mjs) | Packs all four npm workspaces, installs the tarballs into a clean temporary npm project, and executes the public still and movie `npx` commands |
| [`scripts/verify-preferences-contract.mjs`](../scripts/verify-preferences-contract.mjs) | The preferences domain and watch-root read contract in [`PREFERENCES.md`](PREFERENCES.md) |
