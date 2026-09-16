#!/usr/bin/env bash
# Canonical structural/integrity and network-denied execution proof.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
PAYLOAD="${1:-$HERE/../build/Tools}"
python3 "$HERE/tools-payload.py" verify "$PAYLOAD"
echo 'PASS: valid payload'
HOME_DIR="$(mktemp -d)"
trap 'rm -rf "$HOME_DIR"' EXIT
# Empty environment, no external Node/npm, and kernel-enforced network denial.
# sandbox-exec remains available on supported macOS builders.
/usr/bin/env -i HOME="$HOME_DIR" PATH=/usr/bin:/bin TMPDIR="$HOME_DIR" \
  /usr/bin/sandbox-exec -p '(version 1)(allow default)(deny network*)' \
  "$PAYLOAD/bin/astroshot" --help > "$HOME_DIR/help"
grep -qi astroshot "$HOME_DIR/help"
echo 'PASS: offline CLI execution'
if [[ "${2:-}" == '--reject-cases' ]]; then
  python3 "$HERE/test-tools-payload.py" "$PAYLOAD"
fi
