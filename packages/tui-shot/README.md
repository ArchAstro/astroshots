# @archastro/tui-shot

The terminal rendering engine behind `@archastro/astroshot`. It supports two
capture boundaries:

- Ink fixture trees rendered in-process and interpreted with xterm.
- Arbitrary executables launched in a real pseudoterminal, driven by scripted
  input, interpreted with the same xterm model, and captured in Chromium.

Use the unified `@archastro/astroshot` package for command-line capture.

## Quick start

Run from a Node.js 22.14 or newer project. Install the project-local peers only
when using Ink mode:

```bash
npm install --save-dev ink@^7.1 react@^19
npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot install-browser
npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot init ink
npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot init pty
```

Then capture either boundary:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot ink ./ink.shot.tsx -o ./ink.png
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot pty ./pty.shot.yaml -o ./ratatui.png
```

On Linux hosts missing Chromium's system libraries, install the browser with
`install-browser --with-deps`.

## Ink fixture contract

```tsx
import { Box, Text } from "ink";
import React from "react";
import type { InkShotFixture } from "@archastro/astroshot/ink";

export default {
  cols: 80,
  rows: 12,
  background: "#090a12",
  scale: 2,
  expectText: ["Ready to deploy"],
  component: (
    <Box borderStyle="round">
      <Text color="#b9a8ff">Ready to deploy</Text>
    </Box>
  ),
} satisfies InkShotFixture;
```

`expectText` makes capture fail unless every listed string exists in the final
terminal screen. `--cols`, `--rows`, and `--scale` override fixture values.
Ink capture also supports `batch <manifest.yaml|json>`; manifest paths resolve
relative to the manifest.

## PTY fixture contract

PTY fixtures work with any executable that behaves like a terminal program:

```yaml
version: 1
command: ./target/debug/my-ratatui-app
args: []
cwd: .
cols: 100
rows: 30
timeoutMs: 15000
settleMs: 100
actions:
  - waitFor: Choose an option
  - key: down
  - key: enter
  - waitFor: Ready
expectText:
  - Ready
```

The child receives `TERM=xterm-256color`, a fixed terminal grid, and the
fixture environment. `command` is spawned directly—not through a shell—and
`cwd` resolves relative to the fixture. On Windows, use an `.exe`; `.cmd` and
`.bat` scripts are rejected because they require a shell. Supported actions
are:

- `waitFor` with an optional per-action `timeoutMs`;
- `waitForExit: true` with an optional `timeoutMs` when the documented program
  is expected to terminate before capture;
- `key`: `enter`, arrows, `tab`, `escape`, `backspace`, `space`, `ctrl-c`, or
  `ctrl-d`;
- `text` for literal input;
- `pauseMs` for a bounded delay.

Actions run in order. The final xterm screen must contain every `expectText`
value before Chromium captures it. Astroshot terminates a still-running child
after the screenshot.

A child that exits nonzero before capture fails by default, even when expected
text rendered. Set `allowNonZeroExit: true` only to document an intentional
failure state. Use a final `waitForExit: true` action when process completion is
part of the documented state; this waits for the authoritative child status
instead of relying on a fixed settle delay.

The renderer targets text and ANSI/VT terminal interfaces. Mouse events,
mid-journey resize actions, and terminal graphics protocols such as Sixel or
Kitty images are not currently modeled.

`node-pty` is an optional, lazily loaded native dependency so React and Ink
capture still work on an unsupported PTY host. This release pins the exact
official `1.2.0-beta.14` build; PTY capture reports an actionable error if its
native addon is unavailable.

## Reproducibility and trust

Playwright is pinned so each release selects one Chromium version. PNG bytes
can still differ across operating systems because the default stage uses host
fonts. Set `fontFamily` and use a consistent OS/container for pixel baselines.

Ink fixture modules and PTY commands are executable code with the current
user's permissions. Only capture trusted repositories. A PTY is an I/O
boundary, not a security sandbox; use a container for untrusted programs.

## Programmatic API

```ts
import {
  closeSharedBrowser,
  takePtyShot,
  takeTuiShot,
} from "@archastro/tui-shot";

await takeTuiShot({
  fixturePath: "./screenshots/welcome.tsx",
  outPath: "./screenshots/welcome.png",
});
await takePtyShot({
  fixturePath: "./screenshots/ratatui.yaml",
  outPath: "./screenshots/ratatui.png",
});
await closeSharedBrowser();
```

Ink fixtures support Ink 7 and React 19 as peer dependencies.
