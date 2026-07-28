---
name: tui-shot
description: >
  Capture deterministic PNG screenshots of terminal interfaces with the
  unified @archastro/astroshot CLI. Use Ink fixtures for synthetic component
  states and PTY fixtures for arbitrary interactive executables including
  Ratatui, Bubble Tea, Textual, and curses programs.
---

# Capture terminal screenshots

Choose the boundary deliberately:

| Need | Mode |
|---|---|
| Fixed state expressible as an Ink/React tree | `astroshot ink` |
| Real executable, raw mode, alternate screen, or keyboard interaction | `astroshot pty` |
| Browser React component | Read the **react-shot** skill |
| Full browser journey | Read the **agent-browser** skill |

`pty` is the correct mode for Ratatui. It launches the program in a real
pseudoterminal, sends its VT stream through xterm, scripts input, and captures
the final grid in Chromium. Do not attempt to import a Ratatui program into an
Ink fixture.

## Set up

No global Astroshot install is required. For Ink capture, install its
project-local peers; PTY-only projects do not need them:

```bash
npm install --save-dev ink@^7.1 react@^19
npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot --help
npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot install-browser
```

Pin an exact package version in CI. On Linux images missing browser libraries,
use `install-browser --with-deps`.

## Capture Ink

Generate a starting fixture:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init ink ./fixtures/install-wizard.tsx
```

Edit the generated TSX to render the intended production component and state:

```tsx
import React from "react";
import { Box, Text } from "ink";
import type { InkShotFixture } from "@archastro/astroshot/ink";

export default {
  cols: 72,
  rows: 18,
  expectText: ["Install agent", "Continue"],
  component: (
    <Box flexDirection="column">
      <Text bold>Install agent</Text>
      <Text color="cyan">Continue</Text>
    </Box>
  ),
} satisfies InkShotFixture;
```

Capture it:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot ink ./fixtures/install-wizard.tsx \
  -o ./screenshots/install-wizard.png
```

Use `ink batch <manifest.yaml|json>` for maintained fixture sets.

## Capture an arbitrary PTY program

Build the program first, then generate and edit a declarative fixture:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init pty ./fixtures/install-wizard.yaml
```

```yaml
version: 1
command: ./target/debug/install-wizard
args: []
cwd: ..
cols: 100
rows: 30
timeoutMs: 15000
actions:
  - waitFor: Choose an option
  - key: down
  - key: enter
  - waitFor: Ready
expectText:
  - Ready
```

`cwd` resolves relative to the fixture. The command launches directly without
a shell. On Windows, select the underlying `.exe`; `.cmd` and `.bat` scripts
are rejected because they require a shell. Actions may use:

- `waitFor` and optional `timeoutMs`;
- `waitForExit: true` and optional `timeoutMs` when the program must terminate
  before capture;
- `key`: `enter`, arrows, `tab`, `escape`, `backspace`, `space`, `ctrl-c`, or
  `ctrl-d`;
- `text` for literal input;
- `pauseMs` only when no visible readiness signal exists.

Prefer `waitFor` over timing. End with distinctive `expectText` so a wrong
screen fails instead of becoming documentation.

Use this path for text and ANSI/VT applications. It does not currently model
mouse input, mid-run resize actions, Sixel, or Kitty graphics.

Nonzero child exit fails capture by default. Use `allowNonZeroExit: true` only
when the documentation intentionally demonstrates a failure state. If
completion is part of the intended state, finish with `waitForExit: true` so
capture waits for the authoritative child status.

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot pty ./fixtures/install-wizard.yaml \
  -o ./screenshots/install-wizard.png
```

## Make documentation reproducible

- Commit fixture sources beside the documentation.
- Use synthetic data and fixed terminal dimensions.
- Avoid clocks, spinners, random IDs, and live services unless they are the
  behavior being documented.
- Inspect wrapping, clipping, colors, and private data in the PNG.
- Record the fixture, command, output path, and terminal dimensions.

## Send the PNG to Astroshots review

Read the **astroshots** skill, locate its `astroshot-capture` helper, then:

```bash
"$CAPTURE" \
  --feature terminal-install \
  --slug install-wizard \
  --title "Install wizard" \
  --description "The confirmation step is ready." \
  --source ./screenshots/install-wizard.png
```

Finalize the run only after all documentation states are captured:

```bash
"$CAPTURE" --feature terminal-install --status pass --finalize
```

Do not manufacture approval or rewrite human review feedback. Read the
Astroshots review state and regenerate the source fixture when comments request
changes.

## Trust boundary

Ink fixtures, imports, and PTY commands execute with the current user's
permissions. A pseudoterminal is not a security sandbox. Review untrusted
changes before capture and use a container for untrusted programs.

## Report

Tell the human which mode, fixture, and command produced the image; the output
path and terminal dimensions; the final text assertion; and whether it was
added to an Astroshots review stream.
