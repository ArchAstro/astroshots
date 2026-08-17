# `@archastro/astroshot`

One command for deterministic React, Ink, and arbitrary terminal screenshots —
plus **journey movies** into `.astroshot/`.

## Start here (no prerequisites)

```bash
# Write a real .astroshot/ set: stills + movie poster/video + manifest.json.
# No Chromium download, no assets of your own; works with the app closed.
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot demo

# Check Node, watched folders, app install/run state, Chromium, and macOS
# Screen Recording. Each failure prints the exact fix command. Read-only.
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot doctor
```

`demo` accepts `--feature <name>`, `--root <dir>`, and `--json`. `doctor`
accepts `--root <dir>`, `--json`, and `--skip-screen`, and exits non-zero when a
required check fails. Watched-folder coverage comes from the Astroshots app's
own live configuration, so an "empty tray" resolves to one of: first-launch
setup never completed, this project is outside every watched folder, or the
project is watched and the problem is elsewhere.

## Capture

```bash
npx astroshot init react
npx astroshot init ink
npx astroshot init pty

npx astroshot react ./react.shot.tsx -o ./react.png
npx astroshot ink ./ink.shot.tsx -o ./ink.png
npx astroshot pty ./pty.shot.yaml -o ./terminal.png
```

## Movies (`astroshot movie`)

Same binary. Agents: **run `astroshot movie which-source "…"` first**.

| You need to record… | `--source` |
|---------------------|------------|
| Web / SPA / agent-browser | `browser` |
| TUI / CLI / truecolor terminal | `pty` (never desktop of Terminal.app) |
| Native macOS app window | `desktop.window` |
| Your own PNG sequence | `frames` |

```bash
astroshot movie --help
astroshot movie which-source "ratatui truecolor dashboard"
astroshot movie run --source browser --feature web --slug home --url https://example.com
astroshot movie run --source desktop.window --feature app --slug onboard \
  --bundle-id com.example.App --duration-ms 4000
astroshot movie list-windows   # macOS
```

`desktop.window` uses macOS `/usr/sbin/screencapture` (already on the system)
plus a Swift window list shipped in the package — no separate capture binary
download. Requires Screen Recording permission for your terminal/IDE.

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
npx astroshot install-browser
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
On Windows, point `command` at an `.exe`; `.cmd` and `.bat` files are rejected
because running them would introduce an implicit shell.

Fixtures, imported modules, and PTY commands execute with the current user's
permissions. Review untrusted files before capture and never include
credentials or private customer data in fixtures or generated images.
