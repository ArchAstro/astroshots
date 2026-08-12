#!/usr/bin/env bash
# End-to-end proof for the public Sparkle update contract: fetch a fresh feed,
# resolve its newest item, verify every release-notes URL, download the actual
# update archive, and validate its size, ZIP structure, and EdDSA signature.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FEED_URL="${1:-https://github.com/ArchAstro/astroshots/releases/download/appcast/appcast.xml}"
EXPECTED_VERSION="${2:-}"
PROOF_DIR="$(mktemp -d "${TMPDIR:-/tmp}/astroshots-update-proof.XXXXXX")"

cleanup() {
  rm -rf "$PROOF_DIR"
}
trap cleanup EXIT

CURL=(
  curl --fail --silent --show-error --location
  --retry 12 --retry-all-errors --retry-delay 5 --retry-max-time 120
)
refresh_separator="?"
if [[ "$FEED_URL" == *\?* ]]; then
  refresh_separator="&"
fi
REFRESHED_FEED_URL="${FEED_URL}${refresh_separator}refresh=$(date +%s)-$$"
FEED="$PROOF_DIR/appcast.xml"
"${CURL[@]}" --connect-timeout 15 --max-time 120 --output "$FEED" "$REFRESHED_FEED_URL"
xmllint --noout "$FEED"

read -r VERSION BUILD ARCHIVE_URL DECLARED_LENGTH SIGNATURE _NOTES_URL < <(
  python3 - "$FEED" <<'PY'
import sys
import xml.etree.ElementTree as ET

sparkle = "http://www.andymatuschak.org/xml-namespaces/sparkle"
root = ET.parse(sys.argv[1]).getroot()
item = root.find("./channel/item")
if item is None:
    raise SystemExit("appcast has no update items")
enclosure = item.find("enclosure")
if enclosure is None:
    raise SystemExit("latest appcast item has no enclosure")

def text(name):
    node = item.find(f"{{{sparkle}}}{name}")
    return "" if node is None or node.text is None else node.text.strip()

notes = item.find(f"{{{sparkle}}}releaseNotesLink")
values = (
    text("shortVersionString"),
    text("version"),
    enclosure.attrib.get("url", ""),
    enclosure.attrib.get("length", ""),
    enclosure.attrib.get(f"{{{sparkle}}}edSignature", ""),
    "" if notes is None or notes.text is None else notes.text.strip(),
)
if not all(values):
    raise SystemExit(f"latest appcast item is incomplete: {values!r}")
print(*values)
PY
)

if [[ -n "$EXPECTED_VERSION" && "$VERSION" != "$EXPECTED_VERSION" ]]; then
  echo "error: latest feed version is $VERSION, expected $EXPECTED_VERSION" >&2
  exit 1
fi

# Historical items remain visible in the cumulative feed, so all of their
# release-note links must remain live as well.
python3 - "$FEED" <<'PY' | while IFS= read -r notes_url; do
import sys
import xml.etree.ElementTree as ET

sparkle = "http://www.andymatuschak.org/xml-namespaces/sparkle"
for item in ET.parse(sys.argv[1]).getroot().findall("./channel/item"):
    notes = item.find(f"{{{sparkle}}}releaseNotesLink")
    if notes is not None and notes.text:
        print(notes.text.strip())
PY
  "${CURL[@]}" --connect-timeout 15 --max-time 120 --output /dev/null "$notes_url"
done

ARCHIVE="$PROOF_DIR/Astroshots-${VERSION}.zip"
"${CURL[@]}" --connect-timeout 15 --max-time 600 --output "$ARCHIVE" "$ARCHIVE_URL"
if stat -f '%z' "$ARCHIVE" >/dev/null 2>&1; then
  ACTUAL_LENGTH="$(stat -f '%z' "$ARCHIVE")"
else
  ACTUAL_LENGTH="$(stat -c '%s' "$ARCHIVE")"
fi
if [[ "$ACTUAL_LENGTH" != "$DECLARED_LENGTH" ]]; then
  echo "error: archive bytes $ACTUAL_LENGTH do not match appcast $DECLARED_LENGTH" >&2
  exit 1
fi
unzip -tq "$ARCHIVE" >/dev/null

SIGN_UPDATE="${SPARKLE_BIN_DIR:+${SPARKLE_BIN_DIR}/sign_update}"
if [[ -z "$SIGN_UPDATE" || ! -x "$SIGN_UPDATE" ]]; then
  SIGN_UPDATE="$(
    find "$ROOT/build" "$HOME/Library/Developer/Xcode/DerivedData" \
      -path '*/artifacts/sparkle/Sparkle/bin/sign_update' \
      -type f -perm +111 2>/dev/null | head -1
  )"
fi
if [[ -z "$SIGN_UPDATE" || ! -x "$SIGN_UPDATE" ]]; then
  echo "error: Sparkle sign_update tool not found" >&2
  exit 1
fi

if [[ -n "${SPARKLE_ED_KEY_FILE:-}" ]]; then
  "$SIGN_UPDATE" --verify --ed-key-file "$SPARKLE_ED_KEY_FILE" "$ARCHIVE" "$SIGNATURE"
elif [[ -n "${SPARKLE_ED_PRIVATE_KEY:-}" ]]; then
  printf '%s' "$SPARKLE_ED_PRIVATE_KEY" \
    | "$SIGN_UPDATE" --verify --ed-key-file - "$ARCHIVE" "$SIGNATURE"
else
  "$SIGN_UPDATE" --verify \
    --account "${SPARKLE_KEYCHAIN_ACCOUNT:-ai.archastro.Astroshots}" \
    "$ARCHIVE" "$SIGNATURE"
fi

echo "Sparkle update proof passed: version=$VERSION build=$BUILD bytes=$ACTUAL_LENGTH"
