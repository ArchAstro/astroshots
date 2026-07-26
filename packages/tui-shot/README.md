# @archastro/tui-shot

Create deterministic PNG screenshots of real
[Ink](https://github.com/vadimdemedes/ink) components from typed TSX fixtures.
`tui-shot` renders the actual Ink tree, interprets its ANSI output with xterm,
and captures the styled terminal grid in Chromium.

## Quick start

Run these commands from an Ink project using Node.js 22.14 or newer:

```bash
npx --@archastro:registry=https://registry.npmjs.org @archastro/tui-shot install-browser
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot shot ./screenshots/welcome.tsx \
  -o ./screenshots/welcome.png
```

The first command installs the Chromium build pinned by this package. If capture
reports that Chromium is missing, run it again in the same environment and user
account as the capture command.

On Linux hosts that do not already have Chromium's system libraries, install
both the browser and OS dependencies:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot install-browser --with-deps
```

## Fixture contract

```tsx
import { Box, Text } from "ink";
import React from "react";
import type { TuiShotFixture } from "@archastro/tui-shot";

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
} satisfies TuiShotFixture;
```

`expectText` is recommended: capture fails unless every listed string is
present in the rendered terminal frame. Fixture values control terminal
columns, rows, colors, font, spacing, border radius, and PNG scale. The
`--cols`, `--rows`, and `--scale` options override their fixture equivalents.

Fixture modules are executable code loaded into the `tui-shot` process. Only
capture fixtures you trust; they have the same filesystem and environment
access as any script you run locally.

## Batch capture

Paths are resolved relative to the YAML or JSON manifest:

```yaml
shots:
  - fixture: fixtures/01-select.tsx
    out: output/01-select.png
  - fixture: fixtures/02-configure.tsx
    out: output/02-configure.png
```

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot batch ./screenshots/journey.yaml
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot batch ./screenshots/journey.yaml \
  --out-dir ./artifacts
```

With `--out-dir`, each manifest output keeps its relative subdirectory beneath
the selected directory. Absolute paths, parent traversal, and duplicate
destinations are rejected before capture begins.

Use `--headed` with either capture command to show Chromium while debugging.
Run
`npx --@archastro:registry=https://registry.npmjs.org @archastro/tui-shot --help`
for all options.

## Reproducibility

Playwright is pinned so each package release selects one Chromium version.
PNG bytes can still differ across operating systems because the default stage
uses the host's monospace fonts. Set `fontFamily` to an installed font and run
capture in a consistent OS/container when pixel-level baselines matter.

For stable state, fixtures should render exported production components with
realistic deterministic data. Use a PTY-driven test instead when the behavior
under test is process startup, keystrokes, signals, or terminal lifecycle.

## Programmatic API

```ts
import { closeSharedBrowser, takeTuiShot } from "@archastro/tui-shot";

await takeTuiShot({
  fixturePath: "./screenshots/welcome.tsx",
  outPath: "./screenshots/welcome.png",
});
await closeSharedBrowser();
```

The package supports Ink 7 and React 19 as peer dependencies.
