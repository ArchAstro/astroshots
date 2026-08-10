#!/usr/bin/env bash
# Shared environment for building Astroshots with mlx-audio-swift / mlx-swift.
#
# Requirements:
#   - Swift tools ≥ 6.2 (Xcode 26+) — mlx-audio-swift Package.swift is 6.2
#   - Metal Toolchain (metal compiler) for mlx-swift shaders
#   - -skipPackagePluginValidation — mlx-swift CUDA plugin fingerprint fails on
#     macOS CLI builds
#
# CI: run the macOS app job on macos-26 with Xcode 26 selected.
#
# Source this file, then invoke xcodebuild with "${ASTROSHOTS_XCODEBUILD_FLAGS[@]}".
#
# Usage:
#   source "$(dirname "$0")/xcode-env.sh"
#   ensure_mlx_build_prereqs
#   xcodebuild ... "${ASTROSHOTS_XCODEBUILD_FLAGS[@]}" build

set -euo pipefail

ASTROSHOTS_XCODEBUILD_FLAGS=(
  -skipPackagePluginValidation
)

ensure_mlx_build_prereqs() {
  # Prefer an already-available metal tool.
  if xcrun -f metal >/dev/null 2>&1; then
    echo "Metal toolchain: $(xcrun -f metal)"
    return 0
  fi

  echo "Metal toolchain missing — downloading via xcodebuild -downloadComponent MetalToolchain"
  # Non-interactive download (Xcode 16+). Safe to re-run if already present.
  if ! xcodebuild -downloadComponent MetalToolchain; then
    echo "warning: MetalToolchain download failed; mlx-swift Metal compile may fail" >&2
    return 0
  fi

  if xcrun -f metal >/dev/null 2>&1; then
    echo "Metal toolchain ready: $(xcrun -f metal)"
  else
    echo "warning: metal still not on PATH after download; try restarting the shell or Xcode" >&2
  fi
}
