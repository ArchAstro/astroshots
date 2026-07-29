# Ink and PTY modes

Choose the terminal boundary deliberately:

| Need | Mode |
|---|---|
| Fixed state expressible as an Ink component tree | `astroshot ink` |
| Real executable, raw mode, alternate screen, or keyboard interaction | `astroshot pty` |

Do not import a Ratatui, Bubble Tea, Textual, or curses program into an Ink
fixture. Run it through a real pseudoterminal.

## Ink

Ink capture requires project-local peers:

```bash
npm install --save-dev ink@^7.1 react@^19
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init ink ./fixtures/install-wizard.tsx
```

Use a typed fixture:

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

## PTY

Build the target program first, then generate a declarative fixture:

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
a shell. On Windows, select the underlying `.exe`; `.cmd` and `.bat` require a
shell and are rejected.

Actions support:

- `waitFor` with optional `timeoutMs`;
- `waitForExit: true` with optional `timeoutMs`;
- `key`: `enter`, arrows, `tab`, `escape`, `backspace`, `space`, `ctrl-c`, or
  `ctrl-d`;
- `text` for literal input;
- `pauseMs` only when no visible readiness signal exists.

Prefer `waitFor` over sleeps and finish with distinctive `expectText`.
Nonzero exit fails by default; use `allowNonZeroExit: true` only for an
intentional failure-state capture.

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot pty ./fixtures/install-wizard.yaml \
  -o ./screenshots/install-wizard.png
```

PTY mode does not currently model mouse input, mid-run resize actions, Sixel,
or Kitty graphics. A pseudoterminal is not a security sandbox.
