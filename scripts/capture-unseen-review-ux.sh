#!/usr/bin/env bash
# Capture Shots/Friction Logs unseen badges and Seen/History into a temp dir,
# then stream them into .astroshot/unseen-review-ux for tray review.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DERIVED_DATA="${DERIVED_DATA:-$REPO_ROOT/macos/build/DerivedData-hide-friction}"
FIXTURE_ROOT="$(mktemp -d /tmp/astroshots-unseen-ux.XXXXXX)"
CAPTURE_DIR="$FIXTURE_ROOT/captures"
PRODUCT="$FIXTURE_ROOT/demo-app"
FEATURE="$PRODUCT/.astroshot/install-wizard"
LOG_A="$PRODUCT/.astroshot/friction-logs/checkout-as-new-user"
LOG_B="$PRODUCT/.astroshot/friction-logs/invite-teammate"
RUN_ID="ux-$(date -u +%Y%m%dT%H%M%SZ)"

cleanup() {
  case "$FIXTURE_ROOT" in
    /tmp/astroshots-unseen-ux.*) rm -rf "$FIXTURE_ROOT" ;;
  esac
}
trap cleanup EXIT

wait_for_file() {
  local file="$1"
  local attempts=0
  while [[ ! -s "$file" && "$attempts" -lt 160 ]]; do
    sleep 0.25
    attempts=$((attempts + 1))
  done
  [[ -s "$file" ]] || {
    echo "timed out waiting for $file" >&2
    return 1
  }
}

write_png() {
  python3 - "$1" "$2" <<'PY'
import struct, sys, zlib
path, label = sys.argv[1], sys.argv[2]
w, h = 960, 540
row = b"\x00" + bytes([40, 44, 72]) * w
raw = row * h
def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(raw, 9))
png += chunk(b"IEND", b"")
open(path, "wb").write(png)
PY
}

mkdir -p "$CAPTURE_DIR" "$FEATURE" \
  "$LOG_A/runs/20260813T150000Z" \
  "$LOG_B/runs/20260813T151000Z"

write_png "$FEATURE/0001-welcome.png" welcome
write_png "$FEATURE/0002-configure.png" configure
cat >"$FEATURE/manifest.json" <<'EOF'
{
  "version": 1,
  "feature": "install-wizard",
  "run_id": "fixture-run-1",
  "status": "pass",
  "shots": [
    {"id":"0001","file":"0001-welcome.png","slug":"welcome","title":"Welcome","description":"Marketing welcome step.","captured_at":"2026-08-13T15:00:00Z"},
    {"id":"0002","file":"0002-configure.png","slug":"configure","title":"Configure project","description":"Org and project fields.","captured_at":"2026-08-13T15:01:00Z"}
  ]
}
EOF

cat >"$LOG_A/prompt.md" <<'EOF'
# Checkout as a new user

## Goal
Complete checkout with a clean session.

## Persona
First-time buyer on the marketing site.
EOF
cat >"$LOG_A/meta.json" <<'EOF'
{"version":1,"slug":"checkout-as-new-user","title":"Checkout as a new user","description":"Fresh session from pricing to receipt","status":"complete"}
EOF
write_png "$LOG_A/runs/20260813T150000Z/0001-plan.png" plan
cat >"$LOG_A/runs/20260813T150000Z/log.jsonl" <<'EOF'
{"step":1,"id":"plan","title":"Choose a plan","description":"Compared plans.","transcript":"I compare the plans.","screenshots":["0001-plan.png"],"good":["Easy to scan"],"improve":["Annual savings unclear"],"url":"/pricing","captured_at":"2026-08-13T15:00:00Z"}
EOF

cat >"$LOG_B/prompt.md" <<'EOF'
# Invite a teammate

## Goal
Send the first invite from an empty workspace.

## Persona
New admin on day one.
EOF
cat >"$LOG_B/meta.json" <<'EOF'
{"version":1,"slug":"invite-teammate","title":"Invite a teammate","description":"Empty workspace to first invite","status":"complete"}
EOF
write_png "$LOG_B/runs/20260813T151000Z/0001-invite.png" invite
cat >"$LOG_B/runs/20260813T151000Z/log.jsonl" <<'EOF'
{"step":1,"id":"invite","title":"Open invite","description":"Opened the invite dialog.","transcript":"I open invite.","screenshots":["0001-invite.png"],"good":["Dialog is clear"],"improve":["Role picker is hidden"],"url":"/team","captured_at":"2026-08-13T15:10:00Z"}
EOF

source "$REPO_ROOT/macos/scripts/xcode-env.sh"
ensure_mlx_build_prereqs
(
  cd "$REPO_ROOT/macos"
  xcodebuild \
    -project Astroshots.xcodeproj \
    -scheme Astroshots \
    -configuration Debug \
    -destination 'platform=macOS' \
    -derivedDataPath "$DERIVED_DATA" \
    "${ASTROSHOTS_XCODEBUILD_FLAGS[@]}" \
    build
)

APP_BINARY="$DERIVED_DATA/Build/Products/Debug/Astroshots.app/Contents/MacOS/Astroshots"
[[ -x "$APP_BINARY" ]] || {
  echo "missing $APP_BINARY" >&2
  exit 1
}

# Don't fight a running menu-bar instance if we can avoid it; this launch is
# a fixture-only Debug process that terminates itself.
ASTROSHOTS_UI_TEST_TRAY_ROOT="$FIXTURE_ROOT" \
ASTROSHOTS_FRICTION_CAPTURE_DIR="$CAPTURE_DIR" \
ASTROSHOTS_UI_TEST_NARRATION_READY=1 \
  "$APP_BINARY"
wait_for_file "$CAPTURE_DIR/CAPTURE_OK"

CAPTURE=""
for candidate in \
  "$REPO_ROOT/bin/astroshot-capture" \
  "$HOME/.grok/skills/astroshots-review/scripts/astroshot-capture" \
  "$HOME/.agents/skills/astroshots-review/scripts/astroshot-capture"; do
  if [[ -x "$candidate" ]]; then
    CAPTURE="$candidate"
    break
  fi
done
if [[ ! -x "$CAPTURE" ]]; then
  echo "astroshot-capture not found; leaving raw captures in $CAPTURE_DIR" >&2
  ls -la "$CAPTURE_DIR"
  trap - EXIT
  echo "FIXTURE_ROOT=$FIXTURE_ROOT"
  exit 0
fi

cd "$REPO_ROOT"
for spec in \
  "0001-open-tray-stream.png|shots-tab-badge|Shots tab with unseen badge|The Shots tab shows an unseen count and the Unseen stream header." \
  "0004-friction-logs-list.png|friction-unseen|Friction Logs unseen|Unseen friction logs with mark-seen controls and a History chip." \
  "0004b-friction-logs-unseen.png|friction-unseen-remaining|Friction Logs after one Seen|The unseen list after marking one log seen." \
  "0004c-friction-logs-history.png|friction-history|Friction Logs history|History shows the log that was marked Seen."; do
  IFS='|' read -r file slug title desc <<<"$spec"
  src="$CAPTURE_DIR/$file"
  [[ -s "$src" ]] || {
    echo "missing capture $file" >&2
    continue
  }
  "$CAPTURE" --feature unseen-review-ux \
    --slug "$slug" \
    --title "$title" \
    --description "$desc" \
    --run-id "$RUN_ID" \
    --source "$src"
done

"$CAPTURE" --feature unseen-review-ux --status pass --finalize --run-id "$RUN_ID"
echo "Streamed captures under .astroshot/unseen-review-ux (run $RUN_ID)"
ls -la "$REPO_ROOT/.astroshot/unseen-review-ux"
