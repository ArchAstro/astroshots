# @archastro/react-shot

Capture deterministic PNG screenshots from small React or TSX fixtures. The
CLI starts an isolated Vite page, renders the fixture in Chromium with
Playwright, and captures either a CSS selector or the full page.

This package is the React rendering engine. Use the unified
`@archastro/astroshot` package for command-line capture.

Use this for component documentation, release assets, and repeatable visual
fixtures. Use browser automation against your application when the screenshot
needs real authentication, routing, server data, or a complete product flow.

## Quick start

Node.js 22.14 or newer is required. Install the package's compatible Chromium
build once:

```bash
npx astroshot install-browser
```

On a Linux machine that also needs Chromium's operating-system libraries, run
`npx astroshot install-browser --with-deps`.

Then create `example.shot.tsx`:

```tsx
import type { ReactShotFixture } from "@archastro/astroshot/react";

function WelcomeCard() {
  return (
    <main
      data-card
      style={{ width: 480, padding: 32, background: "#f8fafc" }}
    >
      <h1>Hello from React</h1>
    </main>
  );
}

export default {
  width: 800,
  height: 600,
  selector: "[data-card]",
  component: <WelcomeCard />,
} satisfies ReactShotFixture;
```

Capture it without a permanent install:

```bash
npx astroshot react example.shot.tsx --out welcome.png
```

Or install the unified package and use `astroshot` in project scripts:

```bash
npm install --save-dev @archastro/astroshot
npx astroshot react example.shot.tsx -o welcome.png
```

## Fixture API

The fixture's default export accepts:

| Field | Purpose |
| --- | --- |
| `component` | React tree to render; required |
| `width`, `height` | Viewport in CSS pixels; defaults to 1280 by 800 |
| `background` | Page background; defaults to transparent |
| `selector` | Element to capture; defaults to the fixture root |
| `waitFor` | CSS selector or `text=...` gate to await |
| `settleMs` | Extra layout settling time; defaults to 150 ms |
| `fullPage` | Capture the full page instead of one element |
| `stripOverlay` | Isolate the target from a full-screen parent overlay |
| `omitBackground` | Preserve transparent PNG pixels |

Selectors containing `[role=dialog]` automatically enable `stripOverlay` and
`omitBackground`. This avoids including a modal's full-screen dimmer and keeps
rounded corners transparent. Set either option explicitly to override it.

CLI `--width` and `--height` values override fixture dimensions. Values must be
whole CSS pixels between 1 and 10,000, and output paths must end in `.png`.

## Project configuration

Place `react-shot.config.ts` beside your application package. It is discovered
by walking upward from the fixture, or can be selected with `--config`.
Filesystem paths are resolved relative to the config file.

Without a config or `--root`, react-shot uses the closest directory containing
a `package.json`, so fixtures can import sibling application source files.

```ts
export default {
  root: ".",
  alias: {
    "@": "./src",
  },
  styles: ["./src/global.css"],
  postcssConfig: "./postcss.config.mjs",
  dedupe: ["some-react-library"],
  stubModules: ["server-only"],
};
```

Common imports from `next/navigation`, `next/link`, `next/image`,
`next/dynamic`, and `server-only` receive lightweight browser stubs. These
stubs are conveniences for presentation fixtures, not substitutes for testing
Next.js behavior.

If `@tailwindcss/vite` is installed in the target project, react-shot loads it
automatically. Otherwise Vite uses the configured PostCSS pipeline.

## Batch captures

Create a YAML or JSON manifest:

```yaml
root: .
shots:
  - fixture: fixtures/welcome.shot.tsx
    out: screenshots/welcome.png
    width: 900
    height: 700
  - fixture: fixtures/settings.shot.tsx
    out: screenshots/settings.png
```

Paths in a manifest, including per-shot `root` and `config`, are relative to
the manifest file. Destinations are normalized and checked as a complete batch
before capture; duplicate and case-only colliding output paths are rejected:

```bash
npx astroshot react batch shots.yaml
```

## Programmatic API

```ts
import { closeSharedBrowser, takeShot } from "@archastro/react-shot";

try {
  await takeShot({
    fixturePath: "fixtures/welcome.shot.tsx",
    outPath: "screenshots/welcome.png",
  });
} finally {
  await closeSharedBrowser();
}
```

Batch CLI runs reuse one browser process. Programmatic callers should close
the shared browser during shutdown.

## Security

Fixtures and config files are executable code. Vite loads application modules
and `react-shot.config.*`; they run with the permissions of the current user.
Only capture trusted repositories and fixtures. Do not run the CLI on an
untrusted pull request, package, or downloaded manifest without reviewing it.

The Vite server binds only to `127.0.0.1` and restricts file serving to the
package, fixture, configured alias/style paths, and this tool's runtime files.
It does not provide a sandbox for fixture code.

## Troubleshooting

- `Executable doesn't exist`: run
  `npx astroshot install-browser`.
- Import failures: pass `--root` or define `root` and `alias` in the config.
- Missing styles: add their entry files to `styles`; CSS is not inferred.
- A modal includes a dimmer: target `[role=dialog]` or set
  `stripOverlay: true`.
- A fixture depends on application providers or network state: wrap it with
  deterministic providers, or capture the running application instead.

Run
`npx astroshot react --help`
for all CLI options.
