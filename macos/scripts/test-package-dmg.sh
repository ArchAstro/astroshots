#!/usr/bin/env bash
# Canonical end-to-end proof for the distributable macOS experience:
# build the real app, create the branded DMG through Finder, mount the resulting
# image, and assert the files a user sees at that process boundary.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROOF_DIR="$(mktemp -d "${TMPDIR:-/tmp}/astroshots-dmg-proof.XXXXXX")"
DMG="$PROOF_DIR/Astroshots.dmg"
MOUNT="$PROOF_DIR/mount"
MOUNTED=0

cleanup() {
  if [[ "$MOUNTED" == "1" ]]; then
    hdiutil detach "$MOUNT" -force >/dev/null 2>&1 || true
  fi
  rm -rf "$PROOF_DIR"
}
trap cleanup EXIT

# Build and package across the same Xcode, codesign, hdiutil, and Finder
# boundaries used by a release, while avoiding release-only credentials.
"$ROOT/scripts/package-dmg.sh" --adhoc --out "$DMG"
hdiutil verify "$DMG"

# Cross the installation boundary by mounting the final compressed image.
mkdir -p "$MOUNT"
hdiutil attach \
  "$DMG" \
  -readonly \
  -nobrowse \
  -mountpoint "$MOUNT" >/dev/null
MOUNTED=1

# Assert the externally observable installer contract, including the saved
# Finder layout that positions the branded artwork and drag targets.
test -d "$MOUNT/Astroshots.app"
test -L "$MOUNT/Applications"
test "$(readlink "$MOUNT/Applications")" = "/Applications"
test -f "$MOUNT/.background/Astroshots.png"
test -f "$MOUNT/.DS_Store"
cmp \
  "$ROOT/Design/Generated/astroshots-dmg-background.png" \
  "$MOUNT/.background/Astroshots.png"
codesign --verify --deep --strict "$MOUNT/Astroshots.app"
"$ROOT/scripts/verify-tools-payload.sh" \
  "$ROOT/Astroshots/Resources/ToolsPayload" \
  "$MOUNT/Astroshots.app"

echo "Branded DMG end-to-end proof passed"
