---
name: astroshot
description: >
  Capture deterministic PNG screenshots with the unified
  @archastro/astroshot CLI. Use React mode for isolated browser components,
  Ink mode for fixed terminal component states, and PTY mode for real
  interactive terminal executables such as Ratatui, Bubble Tea, Textual, or
  curses. Use when fixed inputs or scripted terminal actions can reproduce the
  target state without a live web application. Use agent-browser for routing,
  authentication, live backend data, or a full browser shell; use
  astroshots-review to stream an existing image for human review; use
  screenshot to orchestrate a documentation image set.
---

# Astroshot capture

Use one CLI with three capture boundaries:

| Target state | Mode | Read |
|---|---|---|
| Isolated browser React component | `react` | [React mode](references/react.md) |
| Fixed Ink component tree | `ink` | [Terminal modes](references/terminal.md) |
| Real terminal executable and keyboard flow | `pty` | [Terminal modes](references/terminal.md) |

Choose the smallest boundary that proves the intended state. Do not rebuild a
full application shell in a fixture merely to avoid running the application.

## Preflight

The public command is:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot --help
```

From a clone of the Astroshots repository, use the checked-out CLI when the
package is not yet available from npm:

```bash
node packages/astroshot/bin/astroshot.mjs --help
```

Do not silently substitute the old `react-shot` or `tui-shot` executables.
They are rendering-engine packages; the supported command surface is
`astroshot react|ink|pty`.

Install Chromium once:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot install-browser
```

On Linux CI images that need browser system libraries, add `--with-deps`.
Pin an exact Astroshot version in CI.

## Workflow

1. Decide whether fixed props, a fixed Ink tree, or a real PTY process owns the
   state.
2. Read the matching mode reference above.
3. Generate a fixture with `astroshot init react|ink|pty` when starting fresh.
4. Replace generated placeholders with production imports or the real command.
5. Use synthetic data, fixed dimensions, and a content-based readiness check.
6. Capture the PNG and open it with the available image inspection tool.
7. Check clipping, wrapping, loading states, colors, transparency, and private
   data.
8. Report the mode, fixture, command, output path, dimensions, and assertion.

React and Ink also support maintained image sets:

```bash
astroshot react batch ./shots.yaml
astroshot ink batch ./terminal-shots.yaml
```

## Review and documentation

`astroshot` creates an image. It does not imply that a human reviewed it.

- To stream the PNG into `.astroshot/<feature>/`, read the
  **astroshots-review**
  skill and pass the file to `astroshot-capture --source`.
- To plan, embed, inventory, and render-check a documentation image set, read
  the **screenshot** skill.

Do not infer Seen from `manifest.json.status`; human acknowledgement and
feedback live in the Astroshots `review.json` contract and apply only to the
reviewed run and image hash.

## Trust boundary

Fixtures, configuration, imports, and PTY commands execute with the current
user's permissions. Review untrusted changes before capture. A temporary Vite
server, Chromium process, or pseudoterminal is not a security sandbox.
