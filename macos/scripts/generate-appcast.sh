#!/usr/bin/env bash
# Build / refresh the Sparkle appcast for a versioned Astroshots update zip.
#
# Usage (from macos/):
#   ./scripts/generate-appcast.sh \
#     --zip /path/to/Astroshots-0.2.1.zip \
#     --version 0.2.1 \
#     --out /path/to/appcast.xml
#
# Requires either:
#   SPARKLE_ED_PRIVATE_KEY   — raw base64 EdDSA private key (CI secret), or
#   SPARKLE_ED_KEY_FILE      — path to the private key file, or
#   macOS Keychain entry for account "ai.archastro.Astroshots" (local dev)
#
# Optional:
#   SPARKLE_BIN_DIR          — path to Sparkle bin/ (generate_appcast, sign_update)
#   DOWNLOAD_URL_PREFIX      — override enclosure base URL
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "$ROOT/.." && pwd)"

ZIP=""
VERSION=""
OUT=""
ACCOUNT="${SPARKLE_KEYCHAIN_ACCOUNT:-ai.archastro.Astroshots}"
MAXIMUM_VERSIONS="${MAXIMUM_VERSIONS:-10}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --zip) ZIP="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --account) ACCOUNT="${2:-}"; shift 2 ;;
    -h|--help)
      sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -z "$ZIP" || ! -f "$ZIP" ]]; then
  echo "error: --zip path required and must exist" >&2
  exit 1
fi
if [[ -z "$VERSION" ]]; then
  echo "error: --version required (marketing version, e.g. 0.2.1)" >&2
  exit 1
fi
if [[ -z "$OUT" ]]; then
  OUT="$REPO_ROOT/appcast.xml"
fi

DOWNLOAD_URL_PREFIX="${DOWNLOAD_URL_PREFIX:-https://github.com/ArchAstro/astroshots/releases/download/v${VERSION}/}"
LINK="${SPARKLE_PRODUCT_LINK:-https://github.com/ArchAstro/astroshots/releases}"

find_sparkle_bin() {
  if [[ -n "${SPARKLE_BIN_DIR:-}" && -x "${SPARKLE_BIN_DIR}/generate_appcast" ]]; then
    echo "$SPARKLE_BIN_DIR"
    return 0
  fi

  # SPM artifact layout after resolving Sparkle in DerivedData.
  local candidate
  while IFS= read -r candidate; do
    if [[ -x "$candidate/generate_appcast" ]]; then
      echo "$candidate"
      return 0
    fi
  done < <(
    find "${HOME}/Library/Developer/Xcode/DerivedData" \
      -path '*/artifacts/sparkle/Sparkle/bin' \
      -type d 2>/dev/null | head -20
  )

  # Download a release tool bundle if nothing local is cached.
  local tools_dir="$ROOT/build/sparkle-tools"
  local archive="$tools_dir/Sparkle-for-Swift-Package-Manager.zip"
  mkdir -p "$tools_dir"
  if [[ ! -x "$tools_dir/bin/generate_appcast" ]]; then
    # stdout is reserved for find_sparkle_bin's returned path.
    echo "==> Downloading Sparkle tools (2.9.5)" >&2
    curl -fsSL \
      -o "$archive" \
      "https://github.com/sparkle-project/Sparkle/releases/download/2.9.5/Sparkle-for-Swift-Package-Manager.zip"
    rm -rf "$tools_dir/extract"
    mkdir -p "$tools_dir/extract"
    unzip -q "$archive" -d "$tools_dir/extract"
    # Layout: artifacts/sparkle/Sparkle/bin or top-level bin
    if [[ -d "$tools_dir/extract/bin" ]]; then
      ln -sfn "$tools_dir/extract/bin" "$tools_dir/bin"
    else
      local found
      found="$(find "$tools_dir/extract" -type d -name bin | head -1)"
      if [[ -z "$found" ]]; then
        echo "error: could not locate Sparkle bin/ in release archive" >&2
        exit 1
      fi
      ln -sfn "$found" "$tools_dir/bin"
    fi
  fi
  echo "$tools_dir/bin"
}

SPARKLE_BIN="$(find_sparkle_bin)"
GENERATE_APPCAST="$SPARKLE_BIN/generate_appcast"
if [[ ! -x "$GENERATE_APPCAST" ]]; then
  echo "error: generate_appcast not executable at $GENERATE_APPCAST" >&2
  exit 1
fi

KEY_ARGS=()
KEY_FILE=""
cleanup_key() {
  if [[ -n "${KEY_FILE:-}" && -f "${KEY_FILE:-}" && "${KEY_FILE_TEMP:-}" == "1" ]]; then
    rm -f "$KEY_FILE"
  fi
}
trap cleanup_key EXIT

if [[ -n "${SPARKLE_ED_KEY_FILE:-}" ]]; then
  KEY_FILE="$SPARKLE_ED_KEY_FILE"
elif [[ -n "${SPARKLE_ED_PRIVATE_KEY:-}" ]]; then
  KEY_FILE="$(mktemp "${TMPDIR:-/tmp}/astroshots-sparkle-key.XXXXXX")"
  KEY_FILE_TEMP=1
  # Accept raw key bytes or a base64-wrapped secret.
  if printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | grep -q '[^A-Za-z0-9+/=]'; then
    printf '%s' "$SPARKLE_ED_PRIVATE_KEY" >"$KEY_FILE"
  else
    # 44-char base64 ed25519 secret seed is the usual generate_keys export.
    printf '%s' "$SPARKLE_ED_PRIVATE_KEY" >"$KEY_FILE"
  fi
  chmod 600 "$KEY_FILE"
fi

if [[ -n "$KEY_FILE" ]]; then
  KEY_ARGS+=(--ed-key-file "$KEY_FILE")
else
  KEY_ARGS+=(--account "$ACCOUNT")
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/astroshots-appcast.XXXXXX")"
cleanup_all() {
  cleanup_key
  rm -rf "$WORK"
}
trap cleanup_all EXIT

# Seed workdir with prior appcast (if any) so history is preserved.
if [[ -f "$OUT" ]]; then
  cp "$OUT" "$WORK/appcast.xml"
elif [[ -f "$REPO_ROOT/appcast.xml" ]]; then
  cp "$REPO_ROOT/appcast.xml" "$WORK/appcast.xml"
fi

# generate_appcast names enclosures from the archive filename.
ZIP_NAME="Astroshots-${VERSION}.zip"
cp "$ZIP" "$WORK/$ZIP_NAME"

# Optional markdown release notes beside the zip.
NOTES_CANDIDATES=(
  "$REPO_ROOT/CHANGELOG.md"
)
for notes in "${NOTES_CANDIDATES[@]}"; do
  if [[ -f "$notes" ]]; then
    # Short embedded notes: title only; full changelog is on GitHub Releases.
    cat >"$WORK/Astroshots-${VERSION}.md" <<EOF
## Astroshots ${VERSION}

See the [GitHub release notes](https://github.com/ArchAstro/astroshots/releases/tag/v${VERSION}) for the full changelog.
EOF
    break
  fi
done

echo "==> Generating appcast (version=$VERSION prefix=$DOWNLOAD_URL_PREFIX)"
"$GENERATE_APPCAST" \
  "${KEY_ARGS[@]}" \
  --download-url-prefix "$DOWNLOAD_URL_PREFIX" \
  --link "$LINK" \
  --maximum-versions "$MAXIMUM_VERSIONS" \
  --maximum-deltas 0 \
  -o "$WORK/appcast.xml" \
  "$WORK"

if [[ ! -f "$WORK/appcast.xml" ]]; then
  echo "error: generate_appcast did not write appcast.xml" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT")"
cp "$WORK/appcast.xml" "$OUT"
echo "==> Wrote $OUT"
# Show the new item enclosure line for CI logs (no private key material).
grep -E 'sparkle:(version|shortVersionString)|enclosure |title>' "$OUT" | head -40 || true
