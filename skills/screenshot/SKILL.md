---
name: screenshot
description: >
  Orchestrate reproducible product screenshot sets for documentation: audit
  the docs contract, plan assets and alt text, choose astroshot or
  agent-browser, visually review the output, embed it, and verify the rendered
  docs. Use for documentation image production or maintenance across one or
  more capture tools. For a single non-documentation capture use the matching
  capture skill; for live test-review streams use astroshots-review.
---

# Documentation screenshot workflow

Own the documentation outcome, not merely PNG generation.

## Choose the capture boundary

| Documentation state | Skill and mode |
|---|---|
| React component with fixed props | **astroshot** `react` |
| Fixed Ink state | **astroshot** `ink` |
| Real terminal executable or keyboard flow | **astroshot** `pty` |
| Authenticated page, routing, live data, or full browser shell | **agent-browser** |
| Optional live human review of any result | **astroshots-review** |

Use a fixture when fixed inputs can express the state. Use a real browser when
the image must prove application boundaries.

## Audit the documentation contract

Before capturing:

1. Find the content root and public asset directory; do not assume a framework.
2. Read two nearby pages for image syntax, naming, dimensions, and voice.
3. Find the screenshot inventory or playbook.
4. Find existing fixtures or browser journeys that already prove the state.
5. Find the documented docs build or preview command.

If no inventory exists, keep a small manifest beside the fixture sources or
documentation. A committed PNG without a reproducible source will go stale.

## Plan the image set

For each image record:

- the reader question it answers;
- stable asset filename and destination;
- capture skill, mode, fixture, manifest, or route;
- viewport or terminal dimensions;
- readiness selector or expected text;
- synthetic data;
- instructional alt text;
- owning documentation page and regeneration command.

Prefer the smallest set that explains the workflow. Do not capture every click
when one image can establish the state.

## Produce and inspect

Read the selected **astroshot** or **agent-browser** skill and follow its
current command contract. Keep fixture sources and batch manifests in version
control. Capture directly to the documentation asset directory when practical.

Open every generated image with the available image inspection tool. Check:

- the instructional state is visible without unrelated chrome;
- text is readable at the rendered documentation width;
- layout is not clipped, unexpectedly wrapped, or still loading;
- terminal colors and line drawing remain legible;
- synthetic names and values are consistent across the page;
- no credentials, customer data, internal hostnames, or worktree identifiers
  appear;
- transparent corners and focused crops render cleanly.

Regenerate a failing image from its source; do not hand-edit the PNG.

## Optional Astroshots review

Read the **astroshots-review** skill and stream the documentation assets under a
run-unique feature so old frames cannot be mistaken for the current set.
Astroshots is optional review transport, not proof that a human saw the files.

Only report `seen` when the current run and image hash match a human
acknowledgement in `review.json`. Address every applicable comment and
regenerate the source. Seen is not approval or sign-off; follow the repository's
normal policy for commit or publication.

On non-macOS systems or when Astroshots is unavailable, visual inspection of
every PNG is still required.

## Embed and render

1. Add or update the image reference on the intended page.
2. Write alt text describing the state, choice, or action the reader should
   understand—not “screenshot of.”
3. Update the inventory with source, regeneration command, dimensions, asset,
   and owning page.
4. Run the repository's documented docs build or preview.
5. Open the rendered page at desktop and narrow widths.
6. Confirm exact path and filename case, legibility, crop, and surrounding
   prose.

## Trust boundary

React and Ink fixtures, configuration, imports, PTY commands, and browser pages
can execute code with the current user's permissions. Review untrusted changes
before capture. Pin npm versions in CI and review package provenance before
first use.

## Report

Tell the human:

- which source and mode produced each asset;
- the asset and owning page paths;
- what each frame proves;
- the visual and rendered-page checks performed;
- the inventory and regeneration command;
- Astroshots feature and human review state, if used;
- every applicable review comment;
- anything that could not be reproduced or inspected.
