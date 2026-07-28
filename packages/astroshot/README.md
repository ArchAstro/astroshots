# `@archastro/astroshot`

One command for deterministic React, Ink, and arbitrary terminal screenshots.

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init react
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init ink
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot init pty

npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot react ./react.shot.tsx -o ./react.png
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot ink ./ink.shot.tsx -o ./ink.png
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot pty ./pty.shot.yaml -o ./terminal.png
```

Use `react` for isolated browser components, `ink` for in-process Ink fixture
trees, and `pty` for executable terminal applications such as Ratatui, Bubble
Tea, Textual, and curses programs. `tui` remains an alias for `ink` for
compatibility.

Ink fixtures require project-local peers:

```bash
npm install --save-dev ink@^7.1 react@^19
```

Install the shared Chromium runtime once:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot install-browser
```

Use `react batch <manifest>` or `ink batch <manifest>` for maintained fixture
sets. Run `<mode> --help` for mode-specific options.

Fixture types are available from the unified package:

```tsx
import type { ReactShotFixture } from "@archastro/astroshot/react";
import type { InkShotFixture } from "@archastro/astroshot/ink";
```

PTY YAML and JSON files launch `command` directly, without a shell. Their
`actions` can wait for visible text, send named keys or literal text, and
pause for bounded durations. Paths in `cwd` are relative to the fixture.

Fixtures, imported modules, and PTY commands execute with the current user's
permissions. Review untrusted files before capture and never include
credentials or private customer data in fixtures or generated images.
