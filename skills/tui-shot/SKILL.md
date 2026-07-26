---
name: tui-shot
description: >
  Capture deterministic PNG screenshots of Ink terminal components with the
  @archastro/tui-shot CLI. Use for terminal UI documentation, wizard states,
  command-center views, and repeatable fixture-driven TUI images.
---

# tui-shot

`@archastro/tui-shot` renders a real Ink tree, interprets its ANSI output with
a terminal model, and screenshots the styled terminal grid in Chromium. Prefer
it for fixed terminal states that should not depend on a PTY, timing, or a
running service.

Use **react-shot** for browser React components. Use a real PTY or end-to-end
harness when the image must prove keyboard interaction, process lifecycle, or
live network behavior.

## Run with npx

No global install is required:

```bash
npx --@archastro:registry=https://registry.npmjs.org @archastro/tui-shot --help
```

Install the Playwright browser once on a new machine:

```bash
npx --@archastro:registry=https://registry.npmjs.org @archastro/tui-shot install-browser
```

`npx` downloads and executes the package. Pin an exact package version in CI
or other security-sensitive automation, and review its npm provenance before
first use. Browser installation downloads a Chromium build; on Linux CI,
`npx --@archastro:registry=https://registry.npmjs.org @archastro/tui-shot install-browser --with-deps`
may also install
operating-system packages and should run only in an expected build environment.

Capture one fixture:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot shot ./fixtures/install-wizard.tsx \
  -o ./screenshots/install-wizard.png
```

Capture a manifest:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot batch ./shots.yaml
```

## Fixture design

A fixture default-exports the Ink element and terminal metadata. Import the
public type from the package:

```tsx
import React from "react";
import { Box, Text } from "ink";
import type { TuiShotFixture } from "@archastro/tui-shot";

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
} satisfies TuiShotFixture;
```

Keep fixtures deterministic:

- render a specific state instead of replaying input;
- use synthetic values and stable terminal dimensions;
- assert distinctive `expectText` so the wrong state fails loudly;
- avoid clocks, spinners, random IDs, and live network calls;
- inspect the PNG for wrapping, clipping, color contrast, and private data.

See the package README for the complete fixture and batch manifest contracts.

## Trust boundary

Fixtures and every module they import are executable code with the current
user's permissions; Chromium does not sandbox the Node-side fixture load.
Review untrusted pull requests and downloaded manifests before running them,
and never put credentials or private customer data in a fixture or shipped
PNG.

## Send the PNG to Astroshots

```bash
# Install the astroshots skill first:
# npx skills add ArchAstro/astroshots --skill astroshots -g -y
CAPTURE="$(ls -d \
  ~/.agents/skills/astroshots/scripts/astroshot-capture \
  ~/.claude/skills/astroshots/scripts/astroshot-capture \
  ~/.codex/skills/astroshots/scripts/astroshot-capture \
  2>/dev/null | head -1)"
test -x "$CAPTURE" || {
  echo "Install the astroshots skill or set CAPTURE to ./bin/astroshot-capture" >&2
  exit 1
}

npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot shot ./fixtures/install-wizard.tsx \
  -o /tmp/install-wizard.png

"$CAPTURE" \
  --feature terminal-install \
  --slug install-wizard \
  --title "Install wizard" \
  --description "The confirmation step is ready." \
  --source /tmp/install-wizard.png
```

Finalize a review journey with:

```bash
"$CAPTURE" --feature terminal-install --status pass --finalize
```

## Report

Tell the human which fixture and command produced the image, its output path,
what terminal dimensions were used, and whether it was added to an Astroshots
stream.

## Related skills

- **screenshot** — documentation image planning, review, and asset conventions
- **astroshots** — live `.astroshot/` streams and manifests
- **react-shot** — deterministic browser component fixtures
- **agent-browser** — real browser journeys against a running app
- **browser-ui-harness** — repeatable end-to-end browser harnesses
