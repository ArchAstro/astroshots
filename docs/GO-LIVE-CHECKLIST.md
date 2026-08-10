# Astroshots go-live checklist

> **Next action:** open a terminal in this repository and complete
> [Phase 1](#phase-1-confirm-npm-access-2-minutes).

## Where things stand

Checked on **July 29, 2026**:

- [x] Astroshots for macOS is live:
      [v0.1.7](https://github.com/ArchAstro/astroshots/releases/tag/v0.1.7)
- [x] `Astroshots.dmg` is signed, notarized, and attached to that release
- [x] The five agent skills are on `main` and install directly from GitHub
- [ ] The three npm packages are **not published yet**
- [ ] npm trusted publishing is **not configured yet**

The npm release to publish now is **0.1.0**:

1. `@archastro/react-shot`
2. `@archastro/tui-shot`
3. `@archastro/astroshot`

Budget **15–25 minutes** if your npm account already belongs to the
`@archastro` organization. Stop after any failed command. Fix that failure
before continuing.

---

## Do this now: first npm publication

### Phase 1: Confirm npm access — 2 minutes

- [ ] Install the npm version this repository uses:

  ```bash
  npm install --global npm@11.17.0
  node --version
  npm --version
  ```

  Expected:

  - Node.js `22.14.0` or newer
  - npm `11.17.0`

- [ ] Sign in to the **public npm registry**:

  ```bash
  npm login --registry=https://registry.npmjs.org
  npm whoami --registry=https://registry.npmjs.org
  ```

- [ ] Open <https://www.npmjs.com/org/archastro>, confirm your account has 2FA
      enabled, and confirm it can publish packages in the `@archastro`
      organization.

**STOP if:** `npm whoami` fails, you cannot see the organization, or you do
not have package-publish permission. An `@archastro` npm owner must grant that
access before anything else will work.

### Phase 2: Prove the packages locally — 5–10 minutes

- [ ] Start from a clean, current `main`:

  ```bash
  git switch main
  git pull --ff-only
  test -z "$(git status --porcelain)" || {
    echo "STOP: the worktree is not clean"
    exit 1
  }
  ```

- [ ] Install, build, and test:

  ```bash
  npm ci
  npm run build --workspaces --if-present
  node packages/astroshot/bin/astroshot.mjs install-browser
  npm run check
  ASTROSHOTS_VERIFY_PACKAGES_CAPTURE=1 npm run pack:check
  ```

- [ ] Confirm all three manifests say `0.1.0`:

  ```bash
  node -e '
  for (const path of [
    "./packages/react-shot/package.json",
    "./packages/tui-shot/package.json",
    "./packages/astroshot/package.json",
  ]) {
    const pkg = require(path);
    console.log(pkg.name, pkg.version);
  }
  '
  ```

  Expected:

  ```text
  @archastro/react-shot 0.1.0
  @archastro/tui-shot 0.1.0
  @archastro/astroshot 0.1.0
  ```

**STOP if:** a test fails, the worktree becomes dirty, or the versions differ.
Do not publish a package you have not just proven.

### Phase 3: Bootstrap public packages — once per package

npm requires each package to exist before GitHub trusted publishing can be
authorized. Do this from the **monorepo root** after `npm ci` and build.

ArchAstro machines often map `@archastro` → GitHub Packages. Always force
the scope to the public registry:

```bash
# From repo root (directory with workspaces: packages/*)
npm publish \
  --workspace @archastro/<package> \
  --access public \
  --@archastro:registry=https://registry.npmjs.org \
  --provenance=false
```

Order for a full bootstrap:

1. `@archastro/react-shot`
2. `@archastro/tui-shot`
3. `@archastro/movie-harness` (new — required for `astroshot movie`)
4. `@archastro/astroshot` **last** (depends on the three engines)

`--provenance=false` is only for these first manual publishes. Later releases
use **Cut npm release** + OIDC provenance via `publish-npm.yml`.

**STOP if:** any publish fails. npm versions cannot be overwritten. If
`workspace not found`, you are not on a commit that contains that package
under `packages/` (pull `main`).

### Phase 4: Enable trusted publishing — 3 minutes

- [ ] Confirm npmjs can see all packages:

  ```bash
  for PACKAGE in \
    @archastro/react-shot \
    @archastro/tui-shot \
    @archastro/movie-harness \
    @archastro/astroshot
  do
    npm view "$PACKAGE" name version \
      --registry=https://registry.npmjs.org
  done
  ```

- [ ] Authorize this repository’s GitHub workflow for each package
      (after the package exists on npmjs):

  ```bash
  for PACKAGE in \
    @archastro/react-shot \
    @archastro/tui-shot \
    @archastro/movie-harness \
    @archastro/astroshot
  do
    npm trust github "$PACKAGE" \
      --repo ArchAstro/astroshots \
      --file publish-npm.yml \
      --allow-publish \
      --yes \
      --registry=https://registry.npmjs.org
  done
  ```

- [ ] Open each npm package page and confirm the GitHub trusted publisher is
      `ArchAstro/astroshots` with workflow `publish-npm.yml`.

The authorized workflow is
[`.github/workflows/publish-npm.yml`](../.github/workflows/publish-npm.yml).
Do not add an npm token to GitHub secrets.

### Phase 5: Tag and verify the public release — 3–5 minutes

- [ ] Tag the exact clean `main` commit:

  ```bash
  git fetch origin main
  test -z "$(git status --porcelain)"
  test "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)"

  git tag -a astroshot-v0.1.0 \
    -m "Release @archastro/astroshot 0.1.0"
  git push origin astroshot-v0.1.0
  ```

- [ ] Watch **Actions → Publish npm package**:
      <https://github.com/ArchAstro/astroshots/actions/workflows/publish-npm.yml>

  The bootstrap versions already exist, so the workflow should verify that
  their tarballs match the tag and safely skip republishing them.

- [ ] Prove a public user can run the CLI:

  ```bash
  npx --yes \
    --@archastro:registry=https://registry.npmjs.org \
    @archastro/astroshot@0.1.0 --help
  ```

- [ ] Prove the skills install into a project:

  ```bash
  mkdir -p /tmp/astroshots-skill-check
  cd /tmp/astroshots-skill-check
  npx skills add ArchAstro/astroshots --skill '*' -y
  npx skills list
  ```

  PromptScript does not support global skill installation. A project-local
  install is the supported path there.

## Definition of live

You are done when all five are true:

- [ ] The four npm package pages show a published version
      (`react-shot`, `tui-shot`, `movie-harness`, `astroshot`)
- [ ] `Publish npm package` is green for the latest `astroshot-v*` tag
- [ ] The public `npx` command prints help, including `astroshot movie`
- [ ] A project-local install discovers all five skills
- [x] The signed macOS v0.1.7 DMG is downloadable

---

## Next npm release (after packages are live)

1. Ensure `@archastro/movie-harness` exists on npmjs and has
   `npm trust github` for `publish-npm.yml` (one-time bootstrap if new).
2. Add notes under `## [Unreleased]` in [`CHANGELOG.md`](../CHANGELOG.md).
3. **Actions → Cut npm release → patch / minor / major** (or
   `gh workflow run "Cut npm release" -f bump=minor`).
4. Confirm **Publish npm package** is green and all **four** package pages
   show the new version with provenance.
5. Smoke:

   ```bash
   npx --yes --@archastro:registry=https://registry.npmjs.org \
     @archastro/astroshot@<version> --help
   npx --yes --@archastro:registry=https://registry.npmjs.org \
     @archastro/astroshot@<version> movie --help
   ```

The cut workflow bumps all four packages, rolls the changelog, tags
`astroshot-vX.Y.Z`, dispatches publish, and opens a PR to main.

Manual fallback (if Actions is unavailable) is still documented in the root
README history and `npm run version:packages -- X.Y.Z`.

---

## Next macOS release

The npm package version and macOS app version are independent.

### 1. Run the appropriate release gates

For any app change:

```bash
cd macos
./scripts/bootstrap.sh
xcodebuild \
  -project Astroshots.xcodeproj \
  -scheme Astroshots \
  -destination 'platform=macOS' \
  CODE_SIGNING_ALLOWED=NO \
  test
```

For review-window changes, also run the signed UI suite from a logged-in Mac:

```bash
xcodebuild \
  -project Astroshots.xcodeproj \
  -scheme AstroshotsReviewUITests \
  -destination 'platform=macOS' \
  DEVELOPMENT_TEAM=YOUR_TEAM_ID \
  test
```

### 2. Record signed UI proof when required

- [ ] Put the exact signed command in the release PR
- [ ] Record the number of tests and failures
- [ ] Confirm all four `ReviewFlowUITests` pass

### 3. Cut the release

1. Add notes under `## [Unreleased]` in [`CHANGELOG.md`](../CHANGELOG.md).
2. In GitHub, run **Actions → Cut release → patch/minor/major**
   (or `gh workflow run "Cut release" -f bump=patch`).

**Cut release** bumps `macos/project.yml`, rolls the changelog, tags `vX.Y.Z`,
**dispatches Release DMG** (so you no longer need a second manual dispatch),
and opens a PR to main.

### 4. Verify the release

- [ ] The release PR is merged
- [ ] `main` contains the new marketing and build versions
- [ ] `CHANGELOG.md` has `## [X.Y.Z] (macos)`
- [ ] `Release DMG` is green (auto-dispatched)
- [ ] The GitHub Release is public
- [ ] `Astroshots-X.Y.Z.dmg` is attached, signed, and notarized
- [ ] `Astroshots-X.Y.Z.zip` (Sparkle update archive) and `appcast.xml` are
      attached; `SPARKLE_ED_PRIVATE_KEY` is set in repo secrets
