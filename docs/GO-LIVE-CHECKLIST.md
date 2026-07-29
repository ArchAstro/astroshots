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

### Phase 3: Bootstrap the three public packages — 3–5 minutes

This phase happens **once**. npm requires each package to exist before GitHub
trusted publishing can be authorized.

- [ ] Publish the React engine:

  ```bash
  npm --@archastro:registry=https://registry.npmjs.org \
    publish --workspace @archastro/react-shot \
    --access public --provenance=false
  ```

- [ ] Publish the terminal engine:

  ```bash
  npm --@archastro:registry=https://registry.npmjs.org \
    publish --workspace @archastro/tui-shot \
    --access public --provenance=false
  ```

- [ ] Publish the unified CLI **last**:

  ```bash
  npm --@archastro:registry=https://registry.npmjs.org \
    publish --workspace @archastro/astroshot \
    --access public --provenance=false
  ```

The registry override is required. ArchAstro development machines may map the
`@archastro` scope to GitHub Packages instead of npmjs.

Using `--provenance=false` is correct **only for these first three manual
publishes**. Future releases use GitHub OIDC and provenance.

**STOP if:** any publish fails. Do not keep moving down the list. Fix that
package first; npm versions cannot be overwritten.

### Phase 4: Enable trusted publishing — 3 minutes

- [ ] Confirm npmjs can see all three packages:

  ```bash
  for PACKAGE in \
    @archastro/react-shot \
    @archastro/tui-shot \
    @archastro/astroshot
  do
    env 'npm_config_@archastro:registry=https://registry.npmjs.org' \
      npm view "$PACKAGE@0.1.0" name version \
      --registry=https://registry.npmjs.org
  done
  ```

- [ ] Authorize this repository’s GitHub workflow for each package:

  ```bash
  env 'npm_config_@archastro:registry=https://registry.npmjs.org' \
    npm trust github @archastro/react-shot \
    --repo ArchAstro/astroshots \
    --file publish-npm.yml \
    --allow-publish \
    --yes

  env 'npm_config_@archastro:registry=https://registry.npmjs.org' \
    npm trust github @archastro/tui-shot \
    --repo ArchAstro/astroshots \
    --file publish-npm.yml \
    --allow-publish \
    --yes

  env 'npm_config_@archastro:registry=https://registry.npmjs.org' \
    npm trust github @archastro/astroshot \
    --repo ArchAstro/astroshots \
    --file publish-npm.yml \
    --allow-publish \
    --yes
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

- [ ] The three npm package pages show version `0.1.0`
- [ ] `Publish npm package` is green for tag `astroshot-v0.1.0`
- [ ] The public `npx` command prints help
- [ ] A project-local install discovers all five skills
- [x] The signed macOS v0.1.7 DMG is downloadable

---

## Not today: the next npm release

Use this only after `0.1.0` is live.

### 1. Create the version change — 2 minutes

Replace `0.1.1` below with the next version:

```bash
git switch main
git pull --ff-only
git switch -c release/astroshot-v0.1.1
npm run version:packages -- 0.1.1
```

### 2. Run the release gate — 5–10 minutes

```bash
npm ci
npm run build --workspaces --if-present
node packages/astroshot/bin/astroshot.mjs install-browser
npm run check
ASTROSHOTS_VERIFY_PACKAGES_CAPTURE=1 npm run pack:check
git diff --check
```

### 3. Commit, push, and merge — 5 minutes plus CI

```bash
git add \
  package-lock.json \
  packages/astroshot/package.json \
  packages/react-shot/package.json \
  packages/tui-shot/package.json
git commit -m "release: Astroshot npm v0.1.1"
git push -u origin release/astroshot-v0.1.1
gh pr create --fill
```

Wait for CI. Merge only when every required check is green.

### 4. Tag the merged `main` commit — 1 minute

```bash
git switch main
git pull --ff-only
test -z "$(git status --porcelain)"
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)"

VERSION="$(node -p "require('./packages/astroshot/package.json').version")"
TAG="astroshot-v$VERSION"
git tag -a "$TAG" -m "Release @archastro/astroshot $VERSION"
git push origin "$TAG"
```

### 5. Verify publication — 3 minutes plus CI

- [ ] `Publish npm package` is green
- [ ] All three packages show the new version
- [ ] The package pages show provenance
- [ ] The registry-pinned `npx @archastro/astroshot@<version> --help` works

---

## Not today: the next macOS release

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

In GitHub, run:

**Actions → Cut release → Run workflow → patch/minor/major**

### 4. Confirm the DMG workflow actually starts

GitHub does not trigger a second workflow when `Cut release` pushes its tag
with `GITHUB_TOKEN`. Until that workflow is redesigned, manually run:

**Actions → Release DMG → Run workflow → tag `vX.Y.Z`**

### 5. Verify the release

- [ ] The release PR is merged
- [ ] `main` contains the new marketing and build versions
- [ ] `Release DMG` is green
- [ ] The GitHub Release is public
- [ ] `Astroshots.dmg` is attached, signed, and notarized
