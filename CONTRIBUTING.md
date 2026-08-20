# Contributing to Astroshots

Thanks for helping make screenshot review easier for people and agents.

## Before opening a change

- Search existing issues and pull requests for related work.
- Keep changes focused. Separate macOS app, React capture, and terminal capture
  changes when they can be reviewed independently.
- Never include private screenshots, credentials, tokens, customer data, or
  machine-specific paths.

## Development setup

The npm workspace requires Node.js 22.14 or later:

```bash
npm ci
npm run build --workspaces --if-present
node packages/astroshot/bin/astroshot.mjs install-browser
npm run check
```

For the macOS app, install Xcode 26 or later (Swift tools 6.2, required by
mlx-audio-swift) and XcodeGen:

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

Visual review changes also have a signed, real-process UI test:

```bash
xcodebuild \
  -project Astroshots.xcodeproj \
  -scheme AstroshotsReviewUITests \
  -destination 'platform=macOS' \
  DEVELOPMENT_TEAM=YOUR_TEAM_ID \
  test
```

Run that scheme from a logged-in macOS desktop with UI testing permissions.
It is separate from the unsigned CI scheme so forks do not need signing
credentials.

Run focused tests while developing. Before requesting review, run the checks
for every surface you changed and describe the user-visible behavior you
verified.

If you touch the app's `UserDefaults` keys — especially the watch roots — or any
tool that reads them, update [`docs/PREFERENCES.md`](docs/PREFERENCES.md) in the
same change. `npm test` runs `scripts/verify-preferences-contract.mjs`, which
derives the canonical preferences domain from `macos/project.yml` and fails on a
wrong-prefix domain string or a drifted watch-root read contract.

## Pull requests

A useful pull request includes:

- why the change is needed and what approach it takes;
- focused automated test coverage;
- a real fixture or command-level proof for screenshot behavior;
- before/after images for visual changes, with only synthetic data;
- documentation updates for changed commands or contracts.

By contributing, you agree that your contribution is licensed under the
repository's [MIT License](LICENSE) and that you will follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Releases

Maintainers cut releases from GitHub Actions (see
[`docs/releasing.md`](docs/releasing.md)):

- **Cut release** — macOS app version + signed DMG
- **Cut npm release** — `@archastro/*` packages via OIDC trusted publishing

Put user-facing notes under `## [Unreleased]` in `CHANGELOG.md` before either
cut. Do not add npm tokens to repository secrets or workflow files. macOS
signing and notarization are documented in [`docs/SIGNING.md`](docs/SIGNING.md).

A macOS release that changes review-window behavior is blocked until a
maintainer runs the signed `AstroshotsReviewUITests` scheme on a logged-in
desktop and records the exact command and passing results for
`testThumbnailTapOpensReviewTakeover`,
`testOverlayCardOpensReviewFromItsThumbnail`,
`testReviewerFeedbackAndSeenAcrossRevision`, and
`testCompactDetailSupportsSeenAndFeedbackWithoutClipping` in the release pull
request. Together they prove the stream and overlay entry paths, feedback,
Seen acknowledgement, revision invalidation, and close or return behavior. If
the Xcode UI worker cannot start, that is an unverified release—not a pass.
