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

For the macOS app, install Xcode 16 and XcodeGen:

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

Maintainers publish npm packages through the repository's trusted-publishing
workflow. Do not add npm tokens to repository secrets or workflow files.
macOS signing and notarization are documented separately in
[`docs/SIGNING.md`](docs/SIGNING.md).

A macOS release that changes review-window behavior is blocked until a
maintainer runs the signed `AstroshotsReviewUITests` scheme on a logged-in
desktop and records the exact command and passing results for
`testThumbnailTapOpensReviewTakeover` and
`testReviewerCommentsApprovesAndRequestsChangesAcrossRevision` in the release
pull request. The first proves the stream-image entry path; the second proves
feedback, decisions, revision invalidation, and close behavior. If the Xcode
UI worker cannot start, that is an unverified release—not a pass.
