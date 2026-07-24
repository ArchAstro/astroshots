#!/usr/bin/env bash
# Generate the Xcode project. Requires XcodeGen (brew install xcodegen).
set -euo pipefail
cd "$(dirname "$0")/.."
xcodegen generate
echo "Open Astroshots.xcodeproj"
