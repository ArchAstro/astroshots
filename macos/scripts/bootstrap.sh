#!/usr/bin/env bash
# Generate the Xcode project and ensure MLX build prereqs.
# Requires XcodeGen (brew install xcodegen) and Xcode with Metal support.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# shellcheck source=xcode-env.sh
source "$ROOT/scripts/xcode-env.sh"
ensure_mlx_build_prereqs

xcodegen generate
echo "Open Astroshots.xcodeproj"
echo "CLI builds need: xcodebuild ... ${ASTROSHOTS_XCODEBUILD_FLAGS[*]}"
