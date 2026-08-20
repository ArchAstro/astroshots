# Releases

User-facing notes live in [`CHANGELOG.md`](../CHANGELOG.md) (Keep a Changelog).
macOS app and npm package versions are independent tracks
(`## [x.y.z] (macos)` vs `## [x.y.z] (npm)`).

---

## Maintainer actions

| Maintainer action | What it does |
|-------------------|--------------|
| **Actions → Cut release** | Bump macOS marketing version + build, roll changelog, tag `vX.Y.Z`, **dispatch Release DMG**, PR to main |
| **Actions → Cut npm release** | Bump all four `@archastro/*` packages, roll changelog, tag `astroshot-vX.Y.Z`, **dispatch Publish npm package**, PR to main |
| [GitHub Releases](https://github.com/ArchAstro/astroshots/releases) | Signed DMG (macOS) and npm release notes |
| [ArchAstro/homebrew-tools](https://github.com/ArchAstro/homebrew-tools) | `Casks/astroshots.rb`, bumped automatically by **Release DMG** after the DMG is published |

Both cut workflows follow the same shape as `archastro-js` / `archastro-python`
`release.yml`: GITHUB_TOKEN tag pushes do not chain other workflows, so publish
is always dispatched explicitly.

```bash
# macOS app (patch/minor/major)
gh workflow run "Cut release" --repo ArchAstro/astroshots -f bump=patch

# npm packages
gh workflow run "Cut npm release" --repo ArchAstro/astroshots -f bump=patch
```

Before cutting either track, put notes under `## [Unreleased]` in
`CHANGELOG.md`. The cut workflow promotes that section into a dated release
header and leaves a fresh empty Unreleased block.

macOS signing secrets are documented in [`SIGNING.md`](SIGNING.md).
First-time npm bootstrap and trusted publishing are documented in
[`GO-LIVE-CHECKLIST.md`](GO-LIVE-CHECKLIST.md).

---

## One-time npm bootstrap

npm trusted publishing requires packages to exist before
`npm trust github` can authorize CI. Bootstrap once from a clean `main`, with
2FA, then configure trusted publishers for
`.github/workflows/publish-npm.yml` (see the go-live checklist). After that,
use **Cut npm release** only. Do not put an npm token in repository secrets.

The publish workflow is safe to retry after a partial npm release. It skips an
existing package version only when the registry tarball has the same integrity
as the package built from the tagged commit.

Tag `astroshot-vX.Y.Z` (or **Cut npm release**) starts
`.github/workflows/publish-npm.yml`. The
workflow rejects a tag unless every package has the same version, reruns
verification, publishes both still-image engines and the movie harness, then
publishes the unified CLI last through npm trusted publishing.

The workflow explicitly targets `https://registry.npmjs.org`. Trusted
publishing automatically attaches provenance after this repository and the
packages are public; npm does not generate provenance for a private source
repository.

---

## Updates for installed copies

Builds are Developer ID signed and notarized so Gatekeeper accepts a normal
open. Installed copies can **Check for Updates…** (Sparkle) against the latest
GitHub Release appcast, however they were installed. The Homebrew cask declares
`auto_updates true`, so Sparkle keeps handling updates and Homebrew does not
fight the app's own self-update.

See [`install.md`](install.md) for every install path.
