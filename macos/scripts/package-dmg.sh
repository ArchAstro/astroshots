#!/usr/bin/env bash
# Build a Release Astroshots.app, wrap it in a DMG, and (by default) Developer-ID
# sign + notarize so Gatekeeper accepts a normal double-click install.
#
# Usage (from macos/):
#   ./scripts/package-dmg.sh
#   ./scripts/package-dmg.sh --out /tmp/Astroshots.dmg
#   ./scripts/package-dmg.sh --adhoc          # local-only, Gatekeeper will warn
#   ./scripts/package-dmg.sh --skip-notarize  # sign only
#
# Developer ID + notarize (CI or local) — set:
#   CODE_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
#   DEVELOPMENT_TEAM=TEAMID
# And one of:
#   # App Store Connect API key (preferred for CI)
#   APPLE_API_KEY_PATH=/path/to/AuthKey_XXX.p8   # or APPLE_API_KEY_BASE64
#   APPLE_API_KEY_ID=XXXXXXXXXX
#   APPLE_API_ISSUER_ID=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
#   # or Apple ID + app-specific password
#   APPLE_ID=you@example.com
#   APPLE_APP_SPECIFIC_PASSWORD=xxxx-xxxx-xxxx-xxxx
#   APPLE_TEAM_ID=TEAMID   # defaults to DEVELOPMENT_TEAM
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT_DMG=""
CONFIGURATION="${CONFIGURATION:-Release}"
IDENTITY="${CODE_SIGN_IDENTITY:-}"
TEAM="${DEVELOPMENT_TEAM:-}"
ADHOC=0
SKIP_NOTARIZE=0
SKIP_SIGN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_DMG="${2:-}"; shift 2 ;;
    --configuration) CONFIGURATION="${2:-}"; shift 2 ;;
    --adhoc) ADHOC=1; shift ;;
    --skip-notarize) SKIP_NOTARIZE=1; shift ;;
    --skip-sign) SKIP_SIGN=1; ADHOC=1; shift ;;
    -h|--help)
      sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
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
require_cmd codesign
require_cmd ditto

if [[ "$ADHOC" == "1" || "$SKIP_SIGN" == "1" ]]; then
  IDENTITY="-"
  TEAM=""
  SKIP_NOTARIZE=1
elif [[ -z "$IDENTITY" || -z "$TEAM" ]]; then
  cat >&2 <<'EOF'
error: Developer ID signing requires:

  export CODE_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
  export DEVELOPMENT_TEAM=TEAMID

Or pass --adhoc for a local-only build (Gatekeeper will warn).

See docs/SIGNING.md for certificate + notarization setup.
EOF
  exit 1
fi

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
  ENABLE_HARDENED_RUNTIME=YES
  OTHER_CODE_SIGN_FLAGS=--timestamp
)
if [[ "$ADHOC" == "1" ]]; then
  SIGN_ARGS+=(
    CODE_SIGNING_REQUIRED=NO
    CODE_SIGN_STYLE=Automatic
    DEVELOPMENT_TEAM=
  )
else
  SIGN_ARGS+=(
    CODE_SIGNING_REQUIRED=YES
    CODE_SIGN_STYLE=Manual
    DEVELOPMENT_TEAM="$TEAM"
  )
fi

echo "==> Building $CONFIGURATION (identity=$IDENTITY team=${TEAM:-none})"
xcodebuild \
  -project Astroshots.xcodeproj \
  -scheme Astroshots \
  -configuration "$CONFIGURATION" \
  -destination 'generic/platform=macOS' \
  -derivedDataPath "$DERIVED" \
  "${SIGN_ARGS[@]}" \
  ONLY_ACTIVE_ARCH=NO \
  build

APP_SRC="$DERIVED/Build/Products/$CONFIGURATION/Astroshots.app"
if [[ ! -d "$APP_SRC" ]]; then
  echo "error: expected app missing: $APP_SRC" >&2
  exit 1
fi

# Copy with resource forks / xattrs preserved for codesign.
APP="$STAGE/Astroshots.app"
echo "==> Staging app"
ditto "$APP_SRC" "$APP"
ln -s /Applications "$STAGE/Applications"

sign_app() {
  local app="$1"
  echo "==> Codesigning app ($IDENTITY)"
  # Entitlements from the built app bundle when present.
  local entitlements="$ROOT/Astroshots/Astroshots.entitlements"
  local ent_args=()
  if [[ -f "$entitlements" ]]; then
    ent_args=(--entitlements "$entitlements")
  fi

  # Sign nested code first if any, then the bundle. --deep is discouraged for
  # modern notarization; ditto'd Release apps are usually flat enough.
  codesign \
    --force \
    --options runtime \
    --timestamp \
    --sign "$IDENTITY" \
    "${ent_args[@]}" \
    "$app"

  codesign --verify --verbose=2 "$app"
  spctl --assess --type execute --verbose=4 "$app" 2>&1 || {
    # Pre-notarization assess often fails; verify signature only.
    echo "note: spctl assess before notarization may fail; signature verify passed"
  }
}

if [[ "$ADHOC" == "1" ]]; then
  echo "==> Ad-hoc signing (not Gatekeeper-clean)"
  codesign --force --sign - --options runtime "$APP" || true
else
  sign_app "$APP"
fi

cat >"$STAGE/README.txt" <<EOF
Astroshots
==========

Drag Astroshots.app into Applications, then launch from the menu bar
(camera icon). There is no Dock icon (LSUIElement).

Default watch root: ~/archastro
Writes go under: <worktree>/.astroshot/<feature>/

Signed with: $IDENTITY
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
hdiutil create \
  -volname "$VOLNAME" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  -imagekey zlib-level=9 \
  "$OUT_DMG"

if [[ "$ADHOC" != "1" ]]; then
  echo "==> Codesigning DMG"
  codesign --force --sign "$IDENTITY" --timestamp "$OUT_DMG"
  codesign --verify --verbose=2 "$OUT_DMG"
fi

notarize_dmg() {
  local dmg="$1"
  local team_id="${APPLE_TEAM_ID:-$TEAM}"
  local key_path="${APPLE_API_KEY_PATH:-}"

  if [[ -z "$key_path" && -n "${APPLE_API_KEY_BASE64:-}" ]]; then
    key_path="${RUNNER_TEMP:-/tmp}/AuthKey_${APPLE_API_KEY_ID:-ci}.p8"
    echo "$APPLE_API_KEY_BASE64" | base64 --decode >"$key_path"
    chmod 600 "$key_path"
  fi

  if [[ -n "$key_path" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]; then
    echo "==> Notarizing with App Store Connect API key"
    xcrun notarytool submit "$dmg" \
      --key "$key_path" \
      --key-id "$APPLE_API_KEY_ID" \
      --issuer "$APPLE_API_ISSUER_ID" \
      --wait
  elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" && -n "$team_id" ]]; then
    echo "==> Notarizing with Apple ID"
    xcrun notarytool submit "$dmg" \
      --apple-id "$APPLE_ID" \
      --password "$APPLE_APP_SPECIFIC_PASSWORD" \
      --team-id "$team_id" \
      --wait
  else
    cat >&2 <<'EOF'
error: notarization credentials missing.

Set either:
  APPLE_API_KEY_ID + APPLE_API_ISSUER_ID + APPLE_API_KEY_PATH (or APPLE_API_KEY_BASE64)
or:
  APPLE_ID + APPLE_APP_SPECIFIC_PASSWORD + APPLE_TEAM_ID

Or pass --skip-notarize (signed but not Gatekeeper-clean until you notarize).
EOF
    return 1
  fi

  echo "==> Stapling notarization ticket"
  xcrun stapler staple "$dmg"
  xcrun stapler validate "$dmg"

  echo "==> Gatekeeper assessment"
  spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg"
}

if [[ "$SKIP_NOTARIZE" == "1" ]]; then
  if [[ "$ADHOC" == "1" ]]; then
    echo "==> Skipping notarization (ad-hoc build)"
  else
    echo "==> Skipping notarization (--skip-notarize); DMG is signed only"
  fi
else
  notarize_dmg "$OUT_DMG"
fi

STABLE="$ROOT/build/Astroshots.dmg"
cp "$OUT_DMG" "$STABLE"

echo "==> Done"
echo "    $OUT_DMG"
echo "    $STABLE"
ls -lh "$OUT_DMG"
