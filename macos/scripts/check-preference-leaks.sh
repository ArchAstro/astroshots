#!/usr/bin/env bash
# Authoritative, end-of-run regression guard for the preference-plist leak.
#
# Background. The unit tests must never create a real UserDefaults suite. A
# registered suite is owned by cfprefsd, which flushes it to disk on its own
# schedule *including after the writing process exits*, so no in-process cleanup
# removes it reliably. 631 such plists had accumulated in ~/Library/Preferences
# before this guard existed. Tests take throwaway defaults from
# AstroshotsTests/TestDefaults.swift, which is memory-only and never registers a
# suite.
#
# Why this script exists in addition to PreferenceLeakGuardTests. Swift Testing
# runs tests in parallel, so no in-process test can be scheduled last; an
# in-suite check cannot observe a leak created by a test that runs after it.
# This script runs after `xcodebuild … test`, when every test has finished.
#
# Why it waits. The trap that hid this bug is that a broken cleanup looks clean
# for a few seconds: measured, 50 leaked suites showed up as 0 immediately and 50
# three seconds later, once cfprefsd flushed. Sampling immediately would go green
# over a live leak, so the check settles first — it must not race cfprefsd.
#
# Usage:
#   scripts/check-preference-leaks.sh              # settle, then assert none
#   ASTROSHOTS_LEAK_SETTLE_SECONDS=0 …             # skip the wait
set -euo pipefail

PREFS_DIR="$HOME/Library/Preferences"
PREFIX="astroshots-"
SETTLE_SECONDS="${ASTROSHOTS_LEAK_SETTLE_SECONDS:-5}"

list_leaked() {
  # `ls | grep` would trip `set -e` when there is no match, so enumerate with
  # find and tolerate an empty result.
  find "$PREFS_DIR" \
    -maxdepth 1 \
    -name "${PREFIX}*.plist" \
    -type f \
    2>/dev/null |
    sort
}

# Nudge cfprefsd to flush any suite it is still holding, so a pending write
# surfaces now rather than after CI has already gone green.
if [[ "$SETTLE_SECONDS" != "0" ]]; then
  echo "Waiting ${SETTLE_SECONDS}s for cfprefsd to flush pending suites…"
  sleep "$SETTLE_SECONDS"
fi

leaked="$(list_leaked)"

if [[ -z "$leaked" ]]; then
  echo "No leaked preference plists in $PREFS_DIR"
  exit 0
fi

count="$(printf '%s\n' "$leaked" | wc -l | tr -d ' ')"

cat >&2 <<EOF
error: $count leaked preference plist(s) in $PREFS_DIR

A test created a real UserDefaults suite instead of using TestDefaults().
Registered suites are written to disk by cfprefsd, after the test process
exits, and cannot be reliably removed afterwards — see the measurements in
macos/AstroshotsTests/TestDefaults.swift.

Fix the offending test to use \`TestDefaults()\`, then clear the residue:

  rm -f ~/Library/Preferences/${PREFIX}*.plist

Leaked files:
EOF
printf '%s\n' "$leaked" | sed 's/^/  /' >&2

exit 1
