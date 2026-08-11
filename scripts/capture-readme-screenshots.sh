#!/usr/bin/env bash
# Regenerate every product image referenced by the root README from the native app.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ASSET_DIR="$REPO_ROOT/docs/images"
DERIVED_DATA="$REPO_ROOT/macos/build/DerivedData-readme"
FIXTURE_ROOT="$(mktemp -d /tmp/astroshots-readme.XXXXXX)"
PRODUCT_ROOT="$FIXTURE_ROOT/demo-app"
CAPTURE_DIR="$FIXTURE_ROOT/captures"
MOVIE_DIR="$PRODUCT_ROOT/.astroshot/product-tour"
FRICTION_ROOT="$PRODUCT_ROOT/.astroshot/friction-logs/checkout-as-new-user"

cleanup() {
  case "$FIXTURE_ROOT" in
    /tmp/astroshots-readme.*) rm -rf "$FIXTURE_ROOT" ;;
  esac
}
trap cleanup EXIT

wait_for_file() {
  local file="$1"
  local attempts=0
  while [[ ! -s "$file" && "$attempts" -lt 120 ]]; do
    sleep 0.25
    attempts=$((attempts + 1))
  done
  [[ -s "$file" ]] || {
    echo "capture-readme-screenshots: timed out waiting for $file" >&2
    return 1
  }
}

mkdir -p "$ASSET_DIR" "$CAPTURE_DIR" "$FRICTION_ROOT/runs/20260811T153000Z" \
  "$FRICTION_ROOT/runs/20260810T180000Z"

cd "$REPO_ROOT"
npm run build --workspace @archastro/movie-harness >/dev/null
node packages/astroshot/bin/astroshot.mjs movie run \
  --source frames \
  --feature product-tour \
  --slug onboarding-movie \
  --root "$PRODUCT_ROOT" \
  --run-id readme-20260811 \
  --demo-frames 8 \
  --fps 8 \
  --title "Onboarding movie" \
  --description "A complete product journey ready for review." \
  --status pass >/dev/null

cp "$MOVIE_DIR/0001-onboarding-movie.png" \
  "$FRICTION_ROOT/runs/20260811T153000Z/0001-choose-plan.png"
cp "$MOVIE_DIR/0001-onboarding-movie.png" \
  "$FRICTION_ROOT/runs/20260811T153000Z/0002-confirm-checkout.png"
cp "$MOVIE_DIR/0001-onboarding-movie.png" \
  "$FRICTION_ROOT/runs/20260810T180000Z/0001-choose-plan.png"

cat >"$FRICTION_ROOT/prompt.md" <<'EOF'
# Checkout as a new user

Complete checkout with a clean session, compare plans, and confirm the receipt.
EOF

cat >"$FRICTION_ROOT/meta.json" <<'EOF'
{
  "version": 1,
  "slug": "checkout-as-new-user",
  "title": "Checkout as a new user",
  "description": "Fresh session from pricing to receipt",
  "status": "complete",
  "updated_at": "2026-08-11T15:32:00Z"
}
EOF

cat >"$FRICTION_ROOT/runs/20260811T153000Z/log.jsonl" <<'EOF'
{"step":1,"id":"choose-plan","title":"Choose a plan","description":"Compared the available plans from a clean session.","transcript":"I arrive at pricing with a clean session and compare the plans. The differences are easy to scan, but annual savings need a clearer explanation. I choose the team plan and continue to checkout.","screenshots":["0001-choose-plan.png"],"good":["Plan differences are easy to scan"],"improve":["Annual savings need a clearer explanation"],"url":"/pricing","captured_at":"2026-08-11T15:30:00Z"}
{"step":2,"id":"confirm-checkout","title":"Confirm checkout","description":"Submitted payment and reached the receipt.","transcript":"From the team plan I submit the test payment and reach a clear receipt. The order summary builds confidence, though the next action is easy to miss. The checkout goal is complete.","screenshots":["0002-confirm-checkout.png"],"good":["Receipt and order summary are clear"],"improve":["The next action is visually quiet"],"url":"/receipt","captured_at":"2026-08-11T15:32:00Z"}
EOF

cat >"$FRICTION_ROOT/runs/20260810T180000Z/log.jsonl" <<'EOF'
{"step":1,"id":"choose-plan","title":"Choose a plan","description":"Earlier pricing pass from a clean session.","transcript":"I compare the plans in an earlier pass. The structure is readable, but the billing toggle is ambiguous, so I stop before checkout and return with a clearer test plan.","screenshots":["0001-choose-plan.png"],"good":["Plan structure is readable"],"improve":["Billing toggle is ambiguous"],"url":"/pricing","captured_at":"2026-08-10T18:00:00Z"}
EOF

# Friction-log history is newest-first by run-directory mtime.
touch -t "$(date -v-1d '+%Y%m%d%H%M')" "$FRICTION_ROOT/runs/20260810T180000Z"
touch "$FRICTION_ROOT/runs/20260811T153000Z"

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
    build >/dev/null
)

APP_BINARY="$DERIVED_DATA/Build/Products/Debug/Astroshots.app/Contents/MacOS/Astroshots"
[[ -x "$APP_BINARY" ]] || {
  echo "capture-readme-screenshots: missing app binary at $APP_BINARY" >&2
  exit 1
}

ASTROSHOTS_UI_TEST_TRAY_ROOT="$FIXTURE_ROOT" \
ASTROSHOTS_FRICTION_CAPTURE_DIR="$CAPTURE_DIR" \
ASTROSHOTS_UI_TEST_NARRATION_READY=1 \
  "$APP_BINARY" &
tray_pid=$!
wait_for_file "$CAPTURE_DIR/CAPTURE_OK"
wait "$tray_pid"

OVERLAY_PATH="$CAPTURE_DIR/overlay-card.png"
ASTROSHOTS_UI_TEST_OVERLAY_PATH="$MOVIE_DIR/0001-onboarding-movie.png" \
ASTROSHOTS_UI_TEST_OVERLAY_CAPTURE_PATH="$OVERLAY_PATH" \
  "$APP_BINARY" &
overlay_pid=$!
wait_for_file "$OVERLAY_PATH"
kill "$overlay_pid" 2>/dev/null || true
wait "$overlay_pid" 2>/dev/null || true

cp "$OVERLAY_PATH" "$ASSET_DIR/overlay-card.png"
cp "$CAPTURE_DIR/0001-open-tray-stream.png" "$ASSET_DIR/shots-movie-stream.png"
cp "$CAPTURE_DIR/0003-shot-detail.png" "$ASSET_DIR/movie-detail.png"
cp "$CAPTURE_DIR/0004-friction-logs-list.png" "$ASSET_DIR/friction-logs.png"
cp "$CAPTURE_DIR/0005-friction-log-detail.png" "$ASSET_DIR/friction-run.png"
cp "$CAPTURE_DIR/0006-friction-step-detail.png" "$ASSET_DIR/friction-step.png"

for asset in \
  overlay-card.png \
  shots-movie-stream.png \
  movie-detail.png \
  friction-logs.png \
  friction-run.png \
  friction-step.png; do
  [[ -s "$ASSET_DIR/$asset" ]] || {
    echo "capture-readme-screenshots: empty output $asset" >&2
    exit 1
  }
done

echo "Refreshed README screenshots in $ASSET_DIR"
