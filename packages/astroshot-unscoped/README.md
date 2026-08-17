# `astroshot`

One command for deterministic React, Ink, and arbitrary terminal screenshots —
plus **journey movies** into `.astroshot/`.

```bash
npx astroshot --help
npx astroshot init react
npx astroshot react ./react.shot.tsx -o ./react.png
npx astroshot ink ./ink.shot.tsx -o ./ink.png
npx astroshot pty ./pty.shot.yaml -o ./terminal.png
npx astroshot movie which-source "ratatui truecolor dashboard"
```

No registry flags, no scope configuration, no `--registry` override.

## What this package is

`astroshot` is the public, unscoped entry point for
[`@archastro/astroshot`](https://www.npmjs.com/package/@archastro/astroshot).
It ships the unified CLI and all three capture engines
(`@archastro/react-shot`, `@archastro/tui-shot`, `@archastro/movie-harness`)
*inside its own tarball* using npm `bundleDependencies`.

That matters because many developers have a `~/.npmrc` that maps the whole
`@archastro` scope to a private registry:

```
@archastro:registry=https://npm.pkg.github.com
```

With that config, `npx @archastro/astroshot` fails with **E404** — npm never
asks the public registry for the package at all. Because this package bundles
everything scoped, nothing scoped is fetched at install time, so a plain
`npx astroshot` works regardless of scope configuration.

The scoped packages remain fully supported and unchanged; use them directly if
you prefer, or if you already have the scope pointed at the public registry.

## Install

```bash
# one-off
npx astroshot --help

# project dev dependency
npm install --save-dev astroshot
```

Install the shared Chromium runtime once before browser-backed captures:

```bash
npx astroshot install-browser
```

On Linux CI images that also need Chromium's system libraries, add
`--with-deps`.

## Peer dependencies you supply

This package deliberately does **not** pin your framework versions. Install the
peers for the modes you use:

| Mode | Peers |
|------|-------|
| `astroshot react` | `react`, `react-dom` (>=18) |
| `astroshot ink` | `ink@^7.1`, `react@^19` |
| `astroshot pty`, `astroshot movie` | none |

```bash
npm install --save-dev ink@^7.1 react@^19
```

`node-pty` is an **optional** dependency. PTY and movie-PTY capture need it;
every other mode works without it, so an environment that cannot build the
native addon still installs successfully.

## Documentation

Full fixture APIs, the PTY action contract, manifest formats, and movie sources
are documented in the
[repository README](https://github.com/ArchAstro/astroshots#readme) and in
[`@archastro/astroshot`](https://www.npmjs.com/package/@archastro/astroshot).

Fixtures, imported modules, and PTY commands execute with the current user's
permissions. Review untrusted files before capture and never include
credentials or private customer data in fixtures or generated images.
