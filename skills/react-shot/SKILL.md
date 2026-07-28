---
name: react-shot
description: >
  Capture deterministic PNG screenshots of isolated React components with the
  react mode of the unified @archastro/astroshot CLI. Use for documentation
  images, dialogs, forms, panels, empty states, and repeatable visual fixtures
  that do not need a running application.
---

# astroshot react

`@archastro/astroshot react` mounts a typed React fixture in a temporary Vite
page and captures the selected element with Chromium. Prefer it for isolated
UI whose state can be expressed with fixed props and local providers.

Use **agent-browser** instead when the image must prove authentication,
routing, live backend data, or a complete application shell. Use
**tui-shot** for Ink and arbitrary PTY terminal UI.

## Run with npx

No global install is required:

```bash
npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot react --help
```

Install the Playwright browser once on a new machine:

```bash
npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot install-browser
```

`npx` downloads and executes the package. Pin an exact package version in CI
or other security-sensitive automation, and review its npm provenance before
first use. Browser installation downloads a Chromium build; on Linux CI,
`npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot install-browser --with-deps`
may also install
operating-system packages and should run only in an expected build environment.

Capture one fixture:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot react ./fixtures/account-dialog.tsx \
  -o ./screenshots/account-dialog.png
```

Capture a manifest:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot react batch ./shots.yaml
```

Resolve fixture and output paths from the caller's working directory. Use
`--root` when the application package root cannot be discovered correctly.

## Fixture design

A fixture default-exports a component plus capture metadata. Import the type
from the public package:

```tsx
import type { ReactShotFixture } from "@archastro/astroshot/react";
import { AccountDialog } from "../src/AccountDialog";

export default {
  width: 960,
  height: 900,
  background: "transparent",
  selector: "[role=dialog]",
  waitFor: "text=Save",
  settleMs: 200,
  component: <AccountDialog accountName="Acme" onClose={() => {}} />,
} satisfies ReactShotFixture;
```

Keep fixtures deterministic:

- use synthetic names and data;
- mock network and server-only modules;
- wrap required providers locally;
- choose a selector that crops to the meaningful component;
- wait for product content, fonts, and layout before capture;
- inspect the PNG for clipping, loading states, and private data.

For application aliases, styles, stubs, and React deduplication, add a
`react-shot.config.ts` near the application package. See the package README
for the full config contract.

## Trust boundary

Fixtures, configuration, imported components, and their transitive modules are
executable code with the current user's permissions. The local Vite server is
not a sandbox. Review untrusted pull requests and downloaded fixture manifests
before running them, and never put credentials or private customer data in a
fixture or shipped PNG.

## Send the PNG to Astroshots

`astroshot react` creates the deterministic image. `astroshot-capture` adds it
to a live `.astroshot/<feature>/` stream:

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
  @archastro/astroshot react ./fixtures/account-dialog.tsx \
  -o /tmp/account-dialog.png

"$CAPTURE" \
  --feature account-settings \
  --slug account-dialog \
  --title "Account dialog" \
  --description "The editable account fields are visible." \
  --source /tmp/account-dialog.png
```

Finalize a review journey with:

```bash
"$CAPTURE" --feature account-settings --status pass --finalize
```

## Report

Tell the human which fixture and command produced the image, its output path,
what the frame proves, and whether it was added to an Astroshots stream.

## Related skills

- **screenshot** — documentation image planning, review, and asset conventions
- **astroshots** — live `.astroshot/` streams and manifests
- **tui-shot** — deterministic Ink and arbitrary PTY terminal fixtures
- **agent-browser** — real browser journeys against a running app
- **browser-ui-harness** — repeatable end-to-end browser harnesses
