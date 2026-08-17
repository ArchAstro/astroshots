#!/usr/bin/env bash
# Prove the two first-win verbs against the documented .astroshot contract.
#
#   1. `astroshot demo` writes a still + movie poster/video + valid manifest.json
#      with zero prerequisites (empty Playwright browser store, no ffmpeg, no
#      user assets).
#   2. `astroshot doctor` reports every required check with a remediation line
#      and never mutates the project it inspects.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
CLI="$REPO_ROOT/packages/astroshot/bin/astroshot.mjs"

fail() {
  echo "verify-demo-doctor: $*" >&2
  exit 1
}

WORK="$(mktemp -d)"
BROWSERS="$(mktemp -d)"
cleanup() {
  rm -rf "$WORK" "$BROWSERS"
}
trap cleanup EXIT

DEMO_PROJECT="$WORK/demo-project"
mkdir -p "$DEMO_PROJECT"
(
  cd "$DEMO_PROJECT"
  git init -q .
  # Empty browser store proves no managed Chromium is required.
  PLAYWRIGHT_BROWSERS_PATH="$BROWSERS" node "$CLI" demo >/dev/null
)

FEATURE_DIR="$DEMO_PROJECT/.astroshot/astroshot-demo"
[[ -s "$FEATURE_DIR/manifest.json" ]] ||
  fail "astroshot demo did not write manifest.json"

node "$REPO_ROOT/scripts/assert-demo-manifest.mjs" "$FEATURE_DIR" ||
  fail "astroshot demo output does not match the manifest contract"

DOCTOR_PROJECT="$WORK/doctor-project"
mkdir -p "$DOCTOR_PROJECT"
(
  cd "$DOCTOR_PROJECT"
  node "$CLI" doctor --json --skip-screen >"$WORK/doctor.json" || true
)
node "$REPO_ROOT/scripts/assert-doctor-report.mjs" "$WORK/doctor.json" ||
  fail "astroshot doctor report is missing checks or remediation lines"
[[ ! -e "$DOCTOR_PROJECT/.astroshot" ]] ||
  fail "astroshot doctor must not write .astroshot"

echo "verify-demo-doctor: demo writes a valid zero-prerequisite set and doctor reports read-only"
