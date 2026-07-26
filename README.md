# Astroshots

**Watch your test screenshots as they land.**

Astroshots is a macOS menu-bar app for anyone who captures UI during browser tests, harness runs, or manual QA. Point your tools at a simple folder layout — frames stream in live, flash over your desktop, and stay in one history across every project you work in.

<p align="center">
  <img src="docs/images/hero-overlay.jpg" alt="Astroshots desktop overlays flashing new screenshots above the menu bar" width="900" />
</p>

---

## Why Astroshots

| Problem | What Astroshots does |
|---------|----------------------|
| Screenshots buried in `/tmp` or CI artifacts | Live stream as soon as a PNG is written |
| Jumping between projects to find shots | One tray, every project under your watch folder |
| “Did that step look right?” mid-run | Desktop overlay above all windows |
| Manual folder digging after a suite | Newest-first history with titles from a small manifest |

No project picker. No account. No cloud. Just files on disk and a camera icon in the menu bar.

<p align="center">
  <img src="docs/images/overlay-cards.png" alt="Stacked overlay cards from different projects" width="420" />
</p>

---

## Install

### From a release (recommended)

1. Download the latest **Astroshots.dmg** from [Releases](https://github.com/ArchAstro/astroshots/releases).
2. Open the DMG and drag **Astroshots** into **Applications**.
3. Launch Astroshots — it lives in the **menu bar** (no Dock icon). After the first run it **warms from a local index** of known `.astroshot` folders so the stream appears quickly, then finishes a full discovery walk in the background.
4. Click the camera icon → gear → set your **watch root** if you want something other than the default.
5. **Right-click** the camera icon → **Quit Astroshots** to exit (left-click opens the tray).

Builds are Developer ID signed and notarized so Gatekeeper accepts a normal open.

### From source

macOS 14+, Xcode 16+, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).

```bash
git clone https://github.com/ArchAstro/astroshots.git
cd astroshots/macos
./scripts/bootstrap.sh
open Astroshots.xcodeproj   # ⌘R
```

Details: [`macos/README.md`](macos/README.md).

---

## How it works

1. **Watch** — Astroshots recursively watches a folder you choose (default `~/archastro`) for `.astroshot/` trees.
2. **Write** — Your harness, agent, or script drops PNGs (and an optional
   execution `manifest.json`) under that layout.
3. **See** — New frames flash on the desktop; open the tray for the full
   stream. Click a row for detail.
4. **Review** — Click a screenshot for a chromeless, screen-sized review
   takeover. A human can comment, approve, or request changes; Astroshots
   writes that feedback to `review.json` for agents and harnesses to read.

### The tray

<p align="center">
  <img src="docs/images/tray-stream.png" alt="Astroshots menu-bar tray showing a live multi-project screenshot stream" width="360" />
  &nbsp;
  <img src="docs/images/tray-detail.png" alt="Astroshots detail view for a single screenshot frame" width="360" />
</p>

<p align="center">
  <img src="docs/images/hero-stream.jpg" alt="Astroshots tray open on the desktop stream view" width="900" />
</p>

### Write layout

```text
<project>/.astroshot/<feature>/
  manifest.json          # optional execution state, written by the harness
  review.json            # optional human feedback, written by Astroshots
  0001-signed-in.png
  0002-configure.png
```

Example `manifest.json`:

```json
{
  "version": 1,
  "feature": "install-wizard",
  "run_id": "install-wizard-…",
  "status": "running",
  "shots": [
    {
      "id": "0002",
      "file": "0002-configure.png",
      "slug": "configure",
      "title": "Configure",
      "description": "Configuration screen for the resource.",
      "url": "/solutions · dialog"
    }
  ]
}
```

Project name is inferred from the folder that contains `.astroshot`. Feature is the directory name under it.

`manifest.json.status` reports only whether the capture journey is running,
passed, or failed. It is never a human approval signal. Human decisions and
comments live separately in `review.json`, keyed by the exact image filename:

```json
{
  "version": 1,
  "run_id": "install-wizard-…",
  "updated_at": "2026-07-26T17:42:00Z",
  "reviews": {
    "0002-configure.png": {
      "decision": "changes_requested",
      "reviewed_at": "2026-07-26T17:42:00Z",
      "image_sha256": "a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1a3b1",
      "comments": [
        {
          "id": "A1B2C3D4-E5F6-47A8-9000-111122223333",
          "body": "The primary action is clipped at this width.",
          "created_at": "2026-07-26T17:41:32Z"
        }
      ]
    }
  }
}
```

An absent decision means pending review; comment-only entries may also omit
`reviewed_at` and `image_sha256`. `approved` and `changes_requested` apply only
while `image_sha256` matches the current file bytes. Replacing an image at the
same path invalidates the decision until a human reviews the new hash; existing
comments remain readable so an agent can act on them. Agents must not infer
approval from `manifest.json`, manufacture an approval, or rewrite human
feedback.

Feedback is scoped to `run_id`. When the manifest has a run id, a missing or
different `review.json.run_id` makes the current run pending; the prior run's
decision and comments do not carry forward, even when the image bytes match.

---

## Screenshot tools

Astroshots also publishes two fixture-driven CLI tools. They create
deterministic documentation and review images without requiring a running
application:

| Tool | Use it for | One-off command |
|------|------------|-----------------|
| [`@archastro/react-shot`](packages/react-shot) | React components, dialogs, forms, and other isolated UI states | `npx --@archastro:registry=https://registry.npmjs.org @archastro/react-shot --help` |
| [`@archastro/tui-shot`](packages/tui-shot) | Ink terminal components rendered through a real terminal model | `npx --@archastro:registry=https://registry.npmjs.org @archastro/tui-shot --help` |

Install Chromium once for either tool:

```bash
npx --@archastro:registry=https://registry.npmjs.org @archastro/react-shot install-browser
npx --@archastro:registry=https://registry.npmjs.org @archastro/tui-shot install-browser
```

On Linux CI images that also need Chromium's system libraries, add
`--with-deps` to the matching install command.

Capture a React fixture:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/react-shot shot ./fixtures/account-dialog.tsx \
  -o ./screenshots/account-dialog.png
```

Capture an Ink fixture:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/tui-shot shot ./fixtures/install-wizard.tsx \
  -o ./screenshots/install-wizard.png
```

Both tools also accept `batch <manifest.yaml|json>`. Their fixture APIs,
configuration, and manifest formats are documented in their package READMEs.
Use a component tool when fixed props can express the state. Use a browser
journey when the screenshot must prove routing, authentication, live data, or
the complete application shell.

Fixtures and configuration are executable code with the current user's
permissions. Only capture trusted repositories and review downloaded fixtures
or pull-request changes before running them.

To make a generated image appear in the Astroshots app, feed it to the capture
helper:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/react-shot shot ./fixtures/account-dialog.tsx \
  -o /tmp/account-dialog.png

./bin/astroshot-capture \
  --feature account-settings \
  --slug account-dialog \
  --title "Account dialog" \
  --description "The editable account fields are visible." \
  --source /tmp/account-dialog.png
```

The npm tools run independently of the macOS viewer; CI verifies them on
Linux with the minimum supported Node.js release and Node.js 24 LTS. The live
viewer remains a macOS application.

---

## Live stream capture helper

Ship screenshots into the layout without hand-editing manifests:

```bash
# From a clone of this repo
export PATH="$PWD/bin:$PATH"

astroshot-capture --feature install-wizard --slug configure \
  --title "Configure" \
  --description "Configuration screen visible." \
  --source ./shot.png

astroshot-capture --feature install-wizard --status pass --finalize
```

That finalizes capture execution only. It does not approve any screenshot.
Review decisions are made by a human in Astroshots and persisted in
`review.json`.

Or capture from an `agent-browser` CLI session:

```bash
astroshot-capture --feature install-wizard --slug configure \
  --from-agent-browser "$SESSION"
```

The helper lives at [`bin/astroshot-capture`](bin/astroshot-capture) → [`skills/astroshots/scripts/astroshot-capture`](skills/astroshots/scripts/astroshot-capture).

### Agent skills

This repo ships **six** skills. Install them with the [skills](https://github.com/vercel-labs/skills) CLI.

| Skill | What it teaches |
|-------|-----------------|
| **astroshots** | Write live screenshot streams under `.astroshot/` |
| **screenshot** | Plan, generate, review, and maintain documentation image sets |
| **react-shot** | Capture deterministic React component fixtures with `@archastro/react-shot` from npmjs |
| **tui-shot** | Capture deterministic Ink fixtures with `@archastro/tui-shot` from npmjs |
| **agent-browser** | Install & drive the agent-browser CLI |
| **browser-ui-harness** | Bash UI smoke harness design (runner vs cases, evidence, cleanup) |

#### Install all skills (recommended)

**Global** (user-level, every project):

```bash
npx skills add ArchAstro/astroshots --skill '*' -g -y
```

**This git project only** (from that project’s root — no `-g`):

```bash
cd /path/to/your/project
npx skills add ArchAstro/astroshots --skill '*' -y
```

`--skill '*'` installs **every** skill in this repo (`astroshots`, `screenshot`, `react-shot`, `tui-shot`, `agent-browser`, `browser-ui-harness`).
Using a single name (e.g. only `--skill astroshots`) installs just that one.

#### Install one skill

```bash
# Global
npx skills add ArchAstro/astroshots --skill astroshots -g -y
npx skills add ArchAstro/astroshots --skill screenshot -g -y
npx skills add ArchAstro/astroshots --skill react-shot -g -y
npx skills add ArchAstro/astroshots --skill tui-shot -g -y
npx skills add ArchAstro/astroshots --skill agent-browser -g -y
npx skills add ArchAstro/astroshots --skill browser-ui-harness -g -y

# This project only (from project root)
npx skills add ArchAstro/astroshots --skill agent-browser -y
```

#### Agents, list, update

```bash
# Limit which coding agents receive the skills
npx skills add ArchAstro/astroshots --skill '*' -g -y -a claude-code -a cursor -a codex
npx skills add ArchAstro/astroshots --skill '*' -g -y -a '*'

# See what’s in the package without installing
npx skills add ArchAstro/astroshots -l

# What’s installed
npx skills list -g          # global
npx skills list             # this project

# Update
npx skills update -g -y     # global
npx skills update -y        # this project
```

Skill sources: [`skills/`](skills/).

---

## Product tour

| Surface | Job |
|---------|-----|
| **Desktop overlay** | New frame above all windows; Open / dismiss |
| **Stream** | Newest-first list across all projects under the watch root |
| **Detail** | Full frame + path, feature, URL, time |
| **Settings** | Watch root, overlay toggles |

Interactive mock (open in a browser): [`docs/mocks/astroshots-menubar.html`](docs/mocks/astroshots-menubar.html).

---

## Releases

| How | What you get |
|-----|----------------|
| [GitHub Releases](https://github.com/ArchAstro/astroshots/releases) | Signed, notarized **DMG** |
| Tag `v*` | CI builds that DMG automatically |
| Tag `react-shot-vX.Y.Z` | Publishes `@archastro/react-shot` with npm trusted publishing |
| Tag `tui-shot-vX.Y.Z` | Publishes `@archastro/tui-shot` with npm trusted publishing |

Maintainers: **Actions → Cut release** only prepares the macOS app release and
DMG. It does not bump or publish either npm package. macOS signing secrets are
documented in [`docs/SIGNING.md`](docs/SIGNING.md).

The npm packages use GitHub Actions OIDC trusted publishing, so normal releases
need no long-lived npm token. npm requires a package to exist before a trusted
publisher can be configured, so a maintainer must bootstrap each package's
first public version from an authenticated, 2FA-protected npm account. From
the repository root, explicitly disable provenance for only these bootstrap
publishes because local publishes cannot create a supported provenance
attestation. The scoped registry flag is intentional: ArchAstro development
machines may map `@archastro` to GitHub Packages, while these packages publish
to npmjs.

```bash
npm login --registry=https://registry.npmjs.org
npm whoami --registry=https://registry.npmjs.org

npm --@archastro:registry=https://registry.npmjs.org \
  publish --workspace @archastro/react-shot \
  --access public --provenance=false
npm --@archastro:registry=https://registry.npmjs.org \
  publish --workspace @archastro/tui-shot \
  --access public --provenance=false
```

Confirm both public package pages and tarballs, then use the repository-pinned
npm 11.17 release to run `npm trust github` for each package:

```bash
env 'npm_config_@archastro:registry=https://registry.npmjs.org' \
  npm trust github @archastro/react-shot \
  --repo ArchAstro/astroshots --file publish-npm.yml --allow-publish \
  --yes
env 'npm_config_@archastro:registry=https://registry.npmjs.org' \
  npm trust github @archastro/tui-shot \
  --repo ArchAstro/astroshots --file publish-npm.yml --allow-publish \
  --yes
```

These commands authorize `.github/workflows/publish-npm.yml` for
`npm publish`. Do not use `--provenance=false` after the bootstrap; the tag
workflow handles later releases with provenance enabled.

For every later npm release, bump exactly one workspace in a pull request:

```bash
# Choose exactly one package.
npm version patch --workspace @archastro/react-shot --no-git-tag-version
# or:
# npm version patch --workspace @archastro/tui-shot --no-git-tag-version

npm ci
npm run build --workspaces --if-present
node packages/react-shot/bin/react-shot.mjs install-browser
node packages/tui-shot/bin/tui-shot.mjs install-browser
npm run check
ASTROSHOTS_VERIFY_PACKAGES_CAPTURE=1 npm run pack:check
git diff --check
```

The version command updates the selected `package.json` and the root
`package-lock.json`; include both in the pull request. After that pull request
is merged, tag the exact clean `origin/main` commit:

```bash
git fetch origin main
test -z "$(git status --porcelain)"
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)"

PACKAGE=react-shot # or tui-shot
VERSION="$(node -p "require('./packages/$PACKAGE/package.json').version")"
TAG="$PACKAGE-v$VERSION"
git tag -a "$TAG" -m "Release @archastro/$PACKAGE $VERSION"
git push origin "$TAG"
```

Pushing `react-shot-vX.Y.Z` or `tui-shot-vX.Y.Z` starts
`.github/workflows/publish-npm.yml`. The workflow rejects a tag whose version
does not exactly match that package's `package.json`, reruns focused
verification, and then publishes that one workspace through npm trusted
publishing.

The workflow explicitly targets `https://registry.npmjs.org`. Trusted
publishing automatically attaches provenance after this repository and the
packages are public; npm does not generate provenance for a private source
repository.

---

## Requirements

- macOS 14 or later  
- A folder of projects to watch (configure in-app)  
- Tools that can write PNG files (any language, any harness)

The npm screenshot tools require Node.js 22.14 or later and a locally installed
Chromium managed by Playwright.

The canonical documentation and skill proof is
[`scripts/verify-skills.sh`](scripts/verify-skills.sh). CI runs its integration
mode, which captures one image with each npm package, streams both through
`astroshot-capture`, and asserts the resulting manifest. The separate
[`scripts/verify-packages.mjs`](scripts/verify-packages.mjs) proof packs both
workspaces, installs the tarballs into a clean temporary npm project, and
executes the exposed `npx` commands.

---

## Contributing and security

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for development and test commands.
Please report vulnerabilities privately as described in
[`SECURITY.md`](SECURITY.md). Astroshots and its npm packages are available
under the [MIT License](LICENSE).
