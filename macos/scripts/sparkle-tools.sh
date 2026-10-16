#!/usr/bin/env bash
# Shared tool resolution. Explicit SPARKLE_BIN_DIR is authoritative; otherwise
# use the fallback cache, then SPM artifacts. Only --download permits a download.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

validate_bin() {
  [[ -x "$1/generate_appcast" && -x "$1/sign_update" ]]
}

if [[ -n "${SPARKLE_BIN_DIR:-}" ]]; then
  if ! validate_bin "$SPARKLE_BIN_DIR"; then
    echo "error: SPARKLE_BIN_DIR=$SPARKLE_BIN_DIR must contain executable generate_appcast and sign_update" >&2
    exit 1
  fi
  (cd "$SPARKLE_BIN_DIR" && pwd)
  exit 0
fi

TOOLS_DIR="$ROOT/build/sparkle-tools"
if validate_bin "$TOOLS_DIR/bin"; then
  echo "$TOOLS_DIR/bin"
  exit 0
fi

# Missing optional directories are normal on a fresh runner. Keep real find
# failures visible, and avoid head/SIGPIPE under pipefail.
for search_root in "$ROOT/build" "$HOME/Library/Developer/Xcode/DerivedData"; do
  [[ -d "$search_root" ]] || continue
  candidates="$(find "$search_root" -type d -path '*/artifacts/sparkle/Sparkle/bin')"
  while IFS= read -r candidate; do
    if validate_bin "$candidate"; then
      echo "$candidate"
      exit 0
    fi
  done <<< "$candidates"
done

if [[ "${1:-}" != --download ]]; then
  echo "error: Sparkle tools not found; set SPARKLE_BIN_DIR to a bin directory containing generate_appcast and sign_update, or run macos/scripts/sparkle-tools.sh --download" >&2
  exit 1
fi
mkdir -p "$TOOLS_DIR"
archive="$TOOLS_DIR/Sparkle-for-Swift-Package-Manager.zip"
echo '==> Downloading Sparkle tools (2.9.5)' >&2
curl -fsSL -o "$archive" https://github.com/sparkle-project/Sparkle/releases/download/2.9.5/Sparkle-for-Swift-Package-Manager.zip
mkdir -p "$TOOLS_DIR/extract"
unzip -qo "$archive" -d "$TOOLS_DIR/extract"
if ! validate_bin "$TOOLS_DIR/extract/bin"; then
  echo "error: Sparkle 2.9.5 archive lacks executable bin/generate_appcast and bin/sign_update" >&2
  exit 1
fi
ln -sfn "$TOOLS_DIR/extract/bin" "$TOOLS_DIR/bin"
echo "$TOOLS_DIR/bin"
