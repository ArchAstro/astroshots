#!/usr/bin/env bash
# Outline only — copy into your repo and fill in project-specific URL discovery.
# Generic browser UI smoke runner (agent-browser + optional Astroshots dual-write).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CASES_DIR="${CASES_DIR:-$SCRIPT_DIR/cases}"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null || pwd)"

case_name="${1:-example}"
case_file="$CASES_DIR/$case_name.sh"
[[ -f "$case_file" ]] || { echo "unknown case: $case_name" >&2; exit 1; }

# --- configure for your app ---
APP_BASE_URL="${APP_BASE_URL:-http://127.0.0.1:3000}"
ARTIFACTS="${SMOKE_ARTIFACTS_DIR:-/tmp/ui-smoke/$(basename "$REPO_ROOT")/${case_name}-$(date -u +%Y%m%dT%H%M%SZ)-$$}"
SESSION="${SMOKE_SESSION:-ui-$(basename "$REPO_ROOT")-${case_name}-$$}"
HOLD="${SMOKE_HOLD:-0}"

mkdir -p "$ARTIFACTS/screenshots" "$ARTIFACTS/snapshots"
REPORT="$ARTIFACTS/report.md"
step_number=0
browser_started=0

log() { printf '[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" | tee -a "$ARTIFACTS/run.log" >&2; }

smoke_browser() {
  log "agent-browser $*"
  agent-browser --session "$SESSION" "$@" 2>&1 | tee -a "$ARTIFACTS/run.log"
}

smoke_step() {
  step_number=$((step_number + 1))
  log "STEP $step_number: $*"
  printf '\n## %02d. %s\n\n' "$step_number" "$*" >>"$REPORT"
}

smoke_wait_text() {
  local text="$1" timeout="${2:-20}" deadline=$((SECONDS + timeout))
  local enc
  enc="$(jq -Rn --arg v "$text" '$v')"
  while (( SECONDS < deadline )); do
    if smoke_browser eval "Boolean(document.body && document.body.innerText.includes($enc))" 2>/dev/null \
      | tail -1 | grep -q true; then
      return 0
    fi
    sleep 0.4
  done
  smoke_fail "timeout waiting for text: $text"
}

smoke_capture() {
  local slug="$1" description="$2"
  local shot="$ARTIFACTS/screenshots/$(printf '%02d' "$step_number")-${slug}.png"
  smoke_browser wait 400 >/dev/null || true
  smoke_browser screenshot --full "$shot" >/dev/null
  printf -- '- %s\n- Screenshot: `%s`\n' "$description" "$shot" >>"$REPORT"
  if command -v astroshot-capture >/dev/null 2>&1; then
    astroshot-capture --root "$REPO_ROOT" --feature "$case_name" --slug "$slug" \
      --description "$description" --status running --source "$shot" >/dev/null || true
  fi
  log "captured $shot"
}

smoke_fail() {
  log "FAIL: $*"
  printf '\n## Result\n\nFAIL: %s\n' "$*" >>"$REPORT"
  agent-browser --session "$SESSION" screenshot --full "$ARTIFACTS/failure.png" >/dev/null 2>&1 || true
  agent-browser --session "$SESSION" console >"$ARTIFACTS/console.txt" 2>/dev/null || true
  exit 1
}

cleanup() {
  if [[ "$browser_started" == "1" && "$HOLD" != "1" ]]; then
    agent-browser --session "$SESSION" close >/dev/null 2>&1 || true
  fi
  if command -v astroshot-capture >/dev/null 2>&1; then
    astroshot-capture --root "$REPO_ROOT" --feature "$case_name" --status pass --finalize >/dev/null 2>&1 || true
  fi
  log "artifacts: $ARTIFACTS"
}
trap cleanup EXIT

# health check — customize path
curl -sf "$APP_BASE_URL" >/dev/null || {
  echo "error: app not reachable at $APP_BASE_URL" >&2
  exit 1
}

cat >"$REPORT" <<EOF
# UI smoke report

- Case: \`$case_name\`
- App: \`$APP_BASE_URL\`
- Session: \`$SESSION\`
EOF

# shellcheck source=/dev/null
source "$case_file"
declare -F run_case >/dev/null || { echo "case must define run_case" >&2; exit 1; }

log "open session $SESSION"
agent-browser --session "$SESSION" open "$APP_BASE_URL" >/dev/null
browser_started=1
agent-browser --session "$SESSION" set viewport 1280 900 >/dev/null || true

run_case
printf '\n## Result\n\nPASS\n' >>"$REPORT"
log "PASS: $case_name"
