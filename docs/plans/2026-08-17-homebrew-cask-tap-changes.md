# Homebrew cask: tap-side changes to apply

`brew install --cask ArchAstro/tools/astroshots` is now the primary install path
in this repo's README, but the cask itself must live in
[ArchAstro/homebrew-tools](https://github.com/ArchAstro/homebrew-tools) — a
**different repository**. The worker that made this change could not push there,
so the files are reproduced in full below and still need to land in the tap.

Until they do, `brew install --cask ArchAstro/tools/astroshots` reports
`No available cask`. The README's Homebrew section carries that caveat and points
readers at the DMG path, which is unchanged.

## Status

| Piece | Repo | State |
|-------|------|-------|
| `Casks/astroshots.rb` | ArchAstro/homebrew-tools | **not pushed** — contents below |
| `scripts/update-astroshots-cask.sh` | ArchAstro/homebrew-tools | **not pushed** — contents below |
| `validate.yml` cask audit job | ArchAstro/homebrew-tools | **not pushed** — diff below |
| `Bump Homebrew cask` release step | this repo | landed in `.github/workflows/release-dmg.yml` |
| README brew-first install | this repo | landed |
| `ARCHASTRO_RELEASE_GITHUB_TOKEN` | this repo's secrets | **missing** — one-time setup, see `docs/SIGNING.md` step 10c |

## Verified locally against the real v0.2.8 release

- `brew audit --cask ArchAstro/tools/astroshots` → clean
- `brew style --cask ArchAstro/tools/astroshots` → clean
- `brew install --cask` → app installed to `/Applications`;
  `codesign --verify --deep --strict` and `spctl --assess --type execute` both pass;
  `open -a Astroshots` launches the menu-bar app
- `brew uninstall --cask astroshots` → removed, and `uninstall quit:` stopped the running app
- Round-trip: `update-astroshots-cask.sh <version> <sha>` reproduces the checked-in
  cask byte-for-byte
- **A wrong sha256 passes plain `brew audit --cask` (exit 0)** but fails
  `brew audit --cask --online` (exit 1, `SHA-256 mismatch`). That is why the tap's
  new audit job uses `--online`: it is the only gate that stops a broken checksum
  reaching `brew install`.

## Apply

Create a branch in a clone of the tap, add the two new files plus the two diffs
below, make the script executable, then open a PR:

```bash
git clone https://github.com/ArchAstro/homebrew-tools.git
cd homebrew-tools
git switch -c feat/astroshots-cask
mkdir -p Casks
# write Casks/astroshots.rb and scripts/update-astroshots-cask.sh from below,
# then apply the validate.yml and README.md diffs
chmod +x scripts/update-astroshots-cask.sh
```

Stage `Casks/astroshots.rb`, `scripts/update-astroshots-cask.sh`,
`.github/workflows/validate.yml`, and `README.md`; commit as
`Add Astroshots cask`; push the branch and open the PR.

Then verify:

```bash
brew audit --cask --online ArchAstro/tools/astroshots
brew install --cask ArchAstro/tools/astroshots && open -a Astroshots
brew uninstall --cask astroshots
bash scripts/update-astroshots-cask.sh --help
```

## `Casks/astroshots.rb`

`version` and `sha256` match the current v0.2.8 release; later releases overwrite
them automatically via the updater script.

```ruby
cask "astroshots" do
  version "0.2.8"
  sha256 "1fd1996fdb066b6ed9160cf331d231ca367c2e7f2a0e94ab57982f03e9ba10f6"

  url "https://github.com/ArchAstro/astroshots/releases/download/v#{version}/Astroshots-#{version}.dmg",
      verified: "github.com/ArchAstro/astroshots/"
  name "Astroshots"
  desc "Menu-bar tray for screenshots, journey movies, and UX friction logs"
  homepage "https://github.com/ArchAstro/astroshots"

  livecheck do
    url :url
    regex(/^v?(\d+(?:\.\d+)+)$/i)
    strategy :github_releases
  end

  # Astroshots updates itself through Sparkle, so Homebrew must not treat an
  # in-place self-update as a conflicting install.
  auto_updates true
  depends_on macos: :sonoma

  app "Astroshots.app"

  uninstall quit: "ai.archastro.Astroshots"

  zap trash: [
    "~/Library/Application Support/ai.archastro.Astroshots",
    "~/Library/Application Support/Astroshots",
    "~/Library/Caches/ai.archastro.Astroshots",
    "~/Library/HTTPStorages/ai.archastro.Astroshots",
    "~/Library/Preferences/ai.archastro.Astroshots.plist",
    "~/Library/Saved Application State/ai.archastro.Astroshots.savedState",
  ]
end
```

Notes on specific stanzas:

- `auto_updates true` — required by the task: Sparkle self-updates must not put
  Homebrew's records into conflict. It also makes `brew upgrade` skip the cask.
- `depends_on macos: :sonoma` — macOS 14 minimum, matching
  `MACOSX_DEPLOYMENT_TARGET: "14.0"` in `macos/project.yml`. Written in the
  symbol form `brew style` autocorrects to; `brew info` still reports
  `macOS >= 14`.
- `uninstall quit:` — the app is a menu-bar agent with no Dock icon, so
  uninstall must stop the running process before removing the bundle. Verified.
- `zap trash:` — paths confirmed on a real install: Application Support uses
  both `Astroshots/` (narration) and `ai.archastro.Astroshots/` (shot index).

## `scripts/update-astroshots-cask.sh`

Mirrors the existing `update-*-formula.sh` helpers (validate inputs, then
rewrite the file). Output must stay byte-identical to the checked-in cask for the
same version + sha — the validate workflow asserts exactly that.

```bash
#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: update-astroshots-cask.sh <version> <dmg-sha256>

Rewrites Casks/astroshots.rb for a published Astroshots macOS release.

Arguments:
  version      Release version, with or without the leading "v" (e.g. 0.2.9).
  dmg-sha256   SHA-256 of Astroshots-<version>.dmg from that GitHub Release.

Options:
  -h, --help   Show this help and exit.

Example:
  scripts/update-astroshots-cask.sh 0.2.9 \
    "$(shasum -a 256 Astroshots-0.2.9.dmg | cut -d ' ' -f 1)"
  brew audit --cask ArchAstro/tools/astroshots
EOF
}

if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -ne 2 ]]; then
  usage >&2
  exit 1
fi

VERSION="${1#v}"
DMG_SHA="$2"
CASK_PATH="$(cd "$(dirname "$0")/.." && pwd)/Casks/astroshots.rb"

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "invalid version: $1" >&2
  exit 1
fi

if [[ ! "$DMG_SHA" =~ ^[0-9a-f]{64}$ ]]; then
  echo "invalid SHA-256 checksum: $DMG_SHA" >&2
  exit 1
fi

mkdir -p "$(dirname "$CASK_PATH")"

cat >"$CASK_PATH" <<EOF
cask "astroshots" do
  version "$VERSION"
  sha256 "$DMG_SHA"

  url "https://github.com/ArchAstro/astroshots/releases/download/v#{version}/Astroshots-#{version}.dmg",
      verified: "github.com/ArchAstro/astroshots/"
  name "Astroshots"
  desc "Menu-bar tray for screenshots, journey movies, and UX friction logs"
  homepage "https://github.com/ArchAstro/astroshots"

  livecheck do
    url :url
    regex(/^v?(\\d+(?:\\.\\d+)+)\$/i)
    strategy :github_releases
  end

  # Astroshots updates itself through Sparkle, so Homebrew must not treat an
  # in-place self-update as a conflicting install.
  auto_updates true
  depends_on macos: :sonoma

  app "Astroshots.app"

  uninstall quit: "ai.archastro.Astroshots"

  zap trash: [
    "~/Library/Application Support/ai.archastro.Astroshots",
    "~/Library/Application Support/Astroshots",
    "~/Library/Caches/ai.archastro.Astroshots",
    "~/Library/HTTPStorages/ai.archastro.Astroshots",
    "~/Library/Preferences/ai.archastro.Astroshots.plist",
    "~/Library/Saved Application State/ai.archastro.Astroshots.savedState",
  ]
end
EOF

echo "updated $CASK_PATH to $VERSION"
```

Behaviour checked: `--help` exits 0; no args, one arg, a bad version, and a
short/non-hex checksum each exit 1 with a specific message; `shellcheck` clean.

## `.github/workflows/validate.yml` diff

The tap had **no** cask coverage at all. The existing `validate` job gains cask
syntax and updater round-trip checks; a new `audit-casks` job runs on macOS,
because casks cannot be audited on ubuntu, and uses `--online` so a wrong sha256
fails CI. `actionlint` + `shellcheck` clean.

```diff
@@ jobs.validate.steps @@
       - name: Validate formula syntax
         run: |
           for formula in Formula/*.rb; do
             ruby -c "$formula"
           done
 
+      - name: Validate cask syntax
+        run: |
+          for cask in Casks/*.rb; do
+            ruby -c "$cask"
+          done
+
       - name: Validate update scripts
         run: |
           for script in scripts/*.sh; do
             bash -n "$script"
           done
+
+      - name: Validate cask updater is byte-identical to the checked-in cask
+        run: |
+          set -euo pipefail
+          version="$(sed -n 's/^  version "\(.*\)"$/\1/p' Casks/astroshots.rb)"
+          sha256="$(sed -n 's/^  sha256 "\(.*\)"$/\1/p' Casks/astroshots.rb)"
+          cp Casks/astroshots.rb /tmp/astroshots-cask-before.rb
+          ./scripts/update-astroshots-cask.sh "$version" "$sha256"
+          diff -u /tmp/astroshots-cask-before.rb Casks/astroshots.rb
+
+  # Casks need a macOS runner and a real tap checkout. `ruby -c` above only
+  # proves the file parses; `brew audit --online` re-downloads the DMG and
+  # verifies the declared sha256, which is what stops a broken checksum from
+  # shipping to `brew install --cask`.
+  audit-casks:
+    name: Audit casks
+    runs-on: macos-latest
+
+    steps:
+      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
+        with:
+          path: tap
+
+      - name: Link the checkout as a Homebrew tap
+        run: |
+          set -euo pipefail
+          tap_dir="$(brew --repository)/Library/Taps/archastro/homebrew-tools"
+          rm -rf "$tap_dir"
+          mkdir -p "$(dirname "$tap_dir")"
+          ln -s "$GITHUB_WORKSPACE/tap" "$tap_dir"
+          brew tap
+
+      - name: brew style --cask
+        run: |
+          set -euo pipefail
+          for cask in tap/Casks/*.rb; do
+            name="$(basename "$cask" .rb)"
+            brew style --cask "archastro/tools/$name"
+          done
+
+      - name: brew audit --cask --online
+        run: |
+          set -euo pipefail
+          for cask in tap/Casks/*.rb; do
+            name="$(basename "$cask" .rb)"
+            echo "::group::brew audit --cask --online $name"
+            brew audit --cask --online "archastro/tools/$name"
+            echo "::endgroup::"
+          done
```

## `README.md` (tap) diff

```diff
-Public Homebrew tap for ArchAstro command-line tools.
+Public Homebrew tap for ArchAstro command-line tools and macOS apps.
@@
 brew install ArchAstro/tools/scopey
 ```
 
+Astroshots is a macOS menu-bar app, so it ships as a cask:
+
+```bash
+brew install --cask ArchAstro/tools/astroshots
+open -a Astroshots
+```
+
 ## Release Update Flow
@@ end of file @@
+
+### Astroshots cask
+
+The Astroshots **Release DMG** workflow runs `scripts/update-astroshots-cask.sh`
+and pushes the version commit here, so a normal release needs no manual edit.
+The cask sets `auto_updates true` because installed copies update themselves
+through Sparkle; Homebrew therefore only handles first install.
+
+For a manual update or recovery, pass the version and the SHA-256 of that
+release's `Astroshots-<version>.dmg`:
+
+```bash
+scripts/update-astroshots-cask.sh 0.2.9 <dmg-sha256>
+brew audit --cask ArchAstro/tools/astroshots
+brew install --cask ArchAstro/tools/astroshots
+open -a Astroshots
+```
```

## After the tap merges

1. Confirm the cask resolves: `brew info --cask ArchAstro/tools/astroshots`.
2. Drop the "Requires the cask to be merged in the tap" caveat from this repo's
   README Homebrew section and the `No available cask` aside in the Quickstart.
3. Add `ARCHASTRO_RELEASE_GITHUB_TOKEN` to this repo's secrets
   (`docs/SIGNING.md` step 10c) so the next release bumps the cask automatically
   instead of failing the `Bump Homebrew cask` step.
