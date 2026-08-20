# Capture tools

Astroshots publishes one fixture-driven CLI for deterministic React, Ink, and
PTY stills plus source-aware journey movies. Install it as
[`astroshot`](../packages/astroshot-unscoped), the unscoped package, which needs
no registry configuration:

```bash
npx astroshot --help
```

---

## The four capture modes

| Mode | Use it for | Command |
|------|------------|---------|
| `react` | React components, dialogs, forms, and other isolated UI states | `astroshot react <fixture> -o <image>` |
| `ink` | Ink terminal components rendered through a real terminal model | `astroshot ink <fixture> -o <image>` |
| `pty` | Arbitrary terminal executables such as Ratatui, Bubble Tea, or curses | `astroshot pty <fixture> -o <image>` |
| `movie` | Browser, truecolor PTY, native macOS window, or custom-frame journeys | `astroshot movie run --source <source> …` |

Use a component tool when fixed props can express the state. Use a browser
journey when the capture must prove routing, authentication, live data, or the
complete application shell.

**Two package names, one CLI.** `astroshot` is the recommended install: it
bundles [`@archastro/astroshot`](../packages/astroshot) and the three capture
engines inside its own tarball, so it installs with no registry flags even when
your `~/.npmrc` maps the `@archastro` scope to a private registry. The scoped
packages remain published and fully supported — use them directly if you already
resolve `@archastro` from the public registry. See
[`UNSCOPED-CLI-DESIGN.md`](UNSCOPED-CLI-DESIGN.md).

---

## Fixtures

Generate a typed or declarative starting fixture with `astroshot init react`,
`astroshot init ink`, or `astroshot init pty`. Existing files are preserved
unless `--force` is explicit. The former `tui` command remains an alias for
`ink`.

Install Chromium once for the browser-backed modes:

```bash
npx astroshot install-browser
```

On Linux CI images that also need Chromium's system libraries, add
`--with-deps`.

Capture a React fixture:

```bash
npx astroshot react ./fixtures/account-dialog.tsx \
  -o ./screenshots/account-dialog.png
```

Capture an Ink fixture:

```bash
npm install --save-dev ink@^7.1 react@^19
npx astroshot ink ./fixtures/install-wizard.tsx \
  -o ./screenshots/install-wizard.png
```

Capture a real Ratatui or other terminal executable through a pseudoterminal:

```bash
npx astroshot pty ./fixtures/ratatui.yaml \
  -o ./screenshots/ratatui.png
```

> Fixtures and configuration are executable code with the current user's
> permissions. Only capture trusted repositories and review downloaded fixtures
> or pull-request changes before running them.

---

## Batch manifests

React and Ink modes also accept `batch <manifest.yaml|json>`. Their fixture
APIs, PTY action contract, configuration, and manifest formats are documented in
the package READMEs:

- [`packages/react-shot`](../packages/react-shot)
- [`packages/tui-shot`](../packages/tui-shot)
- [`packages/movie-harness`](../packages/movie-harness)
- [`packages/astroshot`](../packages/astroshot)

---

## Journey movies

Ask the CLI to choose the correct pixel source before recording:

```bash
astroshot movie which-source "record a Ratatui dashboard with truecolor"
astroshot movie --help
```

### Movie sources

| Journey | `--source` | Why |
|---------|------------|-----|
| Web / SPA / Playwright | `browser` | Records the browser directly; no desktop Chrome grab |
| TUI / CLI / truecolor | `pty` | Preserves terminal color without recording Terminal.app |
| Native macOS app | `desktop.window` | Targets AppKit/SwiftUI chrome through the shipped macOS window tooling |
| Custom PNG sequence | `frames` | Encodes caller-owned pixels into the shared sink |

```bash
# Browser
astroshot movie run --source browser --feature checkout --slug purchase \
  --url https://example.com/checkout

# Native macOS window (requires Screen Recording permission)
astroshot movie list-windows
astroshot movie run --source desktop.window --feature desktop-onboarding \
  --slug first-run --bundle-id com.example.App --duration-ms 4000
```

Every source writes a poster PNG, WebM/MP4 video, duration, source, and optional
chapters into the normal `.astroshot/<feature>/` manifest (see
[`contract.md`](contract.md)). Astroshots shows the poster in the stream and
overlay, then plays the video in tray or full-screen review. `desktop.window` is
macOS-only and rejects off-screen or nearly blank captures unless
`--allow-blank` is explicit.

---

## Live stream capture helper

`astroshot-capture` ships screenshots into the layout without hand-editing
manifests:

```bash
# From a clone of this repo
export PATH="$PWD/bin:$PATH"

astroshot-capture --feature install-wizard --slug configure \
  --title "Configure" \
  --description "Configuration screen visible." \
  --source ./shot.png

astroshot-capture --feature install-wizard --status pass --finalize
```

`--finalize` finalizes capture execution only. It does **not** mark any
screenshot Seen — human acknowledgement and feedback are persisted by Astroshots
in `review.json`.

To make a generated image appear in the Astroshots app, feed it to the helper:

```bash
npx astroshot react ./fixtures/account-dialog.tsx \
  -o /tmp/account-dialog.png

./bin/astroshot-capture \
  --feature account-settings \
  --slug account-dialog \
  --title "Account dialog" \
  --description "The editable account fields are visible." \
  --source /tmp/account-dialog.png
```

Or capture from an `agent-browser` CLI session:

```bash
astroshot-capture --feature install-wizard --slug configure \
  --from-agent-browser "$SESSION"
```

The helper lives at [`bin/astroshot-capture`](../bin/astroshot-capture) →
[`skills/astroshots-review/scripts/astroshot-capture`](../skills/astroshots-review/scripts/astroshot-capture).

---

## Platform support

The npm CLI runs independently of the macOS viewer. CI verifies the still modes
and cross-platform movie sources on Linux with the minimum supported Node.js
release and Node.js 24 LTS; `desktop.window` and the live viewer remain macOS
features.

The npm capture tools require Node.js 22.14 or later and a locally installed
Chromium managed by Playwright. Native-window movie capture needs macOS Screen
Recording permission.
