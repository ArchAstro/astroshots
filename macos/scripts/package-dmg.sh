#!/usr/bin/env bash
# Build a Release Astroshots.app and wrap it in a drag-to-Applications DMG.
#
# Usage (from macos/):
#   ./scripts/package-dmg.sh
#   ./scripts/package-dmg.sh --out /tmp/Astroshots.dmg
#
# Signing:
#   Default is ad-hoc (CODE_SIGN_IDENTITY=-) so CI and local builds work without
#   a Developer ID. For distribution builds, export:
#     CODE_SIGN_IDENTITY="Developer ID Application: …"
#     DEVELOPMENT_TEAM=XXXXXXXXXX
#   Notarization is intentionally out of scope here (needs Apple credentials).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT_DMG=""
CONFIGURATION="${CONFIGURATION:-Release}"
IDENTITY="${CODE_SIGN_IDENTITY:--}"
TEAM="${DEVELOPMENT_TEAM:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)
      OUT_DMG="${2:-}"
      shift 2
      ;;
    --configuration)
      CONFIGURATION="${2:-}"
      shift 2
      ;;
    -h|--help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: required command not found: $1" >&2
    exit 1
  }
}

require_cmd xcodegen
require_cmd xcodebuild
require_cmd hdiutil

echo "==> Generating Xcode project"
xcodegen generate

DERIVED="$ROOT/build/DerivedData"
STAGE="$ROOT/build/dmg-root"
mkdir -p "$DERIVED"
rm -rf "$STAGE"
mkdir -p "$STAGE"

SIGN_ARGS=(
  CODE_SIGN_IDENTITY="$IDENTITY"
  CODE_SIGNING_ALLOWED=YES
)
if [[ -n "$TEAM" ]]; then
  SIGN_ARGS+=(DEVELOPMENT_TEAM="$TEAM")
else
  # Ad-hoc / local: don't require a team or provisioning profile.
  SIGN_ARGS+=(
    CODE_SIGNING_REQUIRED=NO
    DEVELOPMENT_TEAM=
  )
fi

echo "==> Building $CONFIGURATION (identity=${IDENTITY:-none})"
xcodebuild \
  -project Astroshots.xcodeproj \
  -scheme Astroshots \
  -configuration "$CONFIGURATION" \
  -destination 'generic/platform=macOS' \
  -derivedDataPath "$DERIVED" \
  "${SIGN_ARGS[@]}" \
  ONLY_ACTIVE_ARCH=NO \
  build

APP="$DERIVED/Build/Products/$CONFIGURATION/Astroshots.app"
if [[ ! -d "$APP" ]]; then
  echo "error: expected app missing: $APP" >&2
  exit 1
fi

echo "==> Staging DMG contents"
cp -R "$APP" "$STAGE/Astroshots.app"
ln -s /Applications "$STAGE/Applications"

# Optional: drop a short readme on the disk image
cat >"$STAGE/README.txt" <<'EOF'
Astroshots
==========

Drag Astroshots.app into Applications, then launch from the menu bar
(camera icon). There is no Dock icon (LSUIElement).

Default watch root: ~/archastro
Writes go under: <worktree>/.astroshot/<feature>/

Ad-hoc signed CI builds may need right-click → Open the first time
(Gatekeeper). Developer ID + notarized builds will not.
EOF

VERSION="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
    "$APP/Contents/Info.plist" 2>/dev/null || echo "0.1.0"
)"
BUILD="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' \
    "$APP/Contents/Info.plist" 2>/dev/null || echo "1"
)"

if [[ -z "$OUT_DMG" ]]; then
  OUT_DMG="$ROOT/build/Astroshots-${VERSION}-${BUILD}.dmg"
fi
mkdir -p "$(dirname "$OUT_DMG")"
rm -f "$OUT_DMG"

VOLNAME="Astroshots ${VERSION}"
echo "==> Creating DMG: $OUT_DMG"
# UDZO = compressed read-only image; fine for distribution artifacts.
hdiutil create \
  -volname "$VOLNAME" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  -imagekey zlib-level=9 \
  "$OUT_DMG"

# Convenience copy without version for scripts that want a stable name.
STABLE="$ROOT/build/Astroshots.dmg"
cp "$OUT_DMG" "$STABLE"

echo "==> Done"
echo "    $OUT_DMG"
echo "    $STABLE"
ls -lh "$OUT_DMG"
