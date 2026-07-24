#!/usr/bin/env bash
# Import a Developer ID Application .p12 into a temporary keychain for CI.
#
# Required env:
#   MACOS_CERTIFICATE_P12_BASE64  base64 of the .p12 file
#   MACOS_CERTIFICATE_PASSWORD    password for the .p12
# Optional:
#   KEYCHAIN_PASSWORD             password for the ephemeral keychain (default: random)
#
# After import, codesign / xcodebuild can see the identity. Cleanup is automatic
# when the runner tears down; for local use, call with --cleanup later if needed.
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

: "${MACOS_CERTIFICATE_P12_BASE64:?set MACOS_CERTIFICATE_P12_BASE64}"
: "${MACOS_CERTIFICATE_PASSWORD:?set MACOS_CERTIFICATE_PASSWORD}"

KEYCHAIN_PASSWORD="${KEYCHAIN_PASSWORD:-$(openssl rand -base64 24)}"
KEYCHAIN_PATH="${RUNNER_TEMP:-/tmp}/astroshots-signing.keychain-db"
CERT_PATH="${RUNNER_TEMP:-/tmp}/astroshots-signing.p12"

echo "==> Decoding certificate"
echo "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode >"$CERT_PATH"

echo "==> Creating temporary keychain"
security delete-keychain "$KEYCHAIN_PATH" 2>/dev/null || true
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"

echo "==> Importing .p12"
security import "$CERT_PATH" \
  -P "$MACOS_CERTIFICATE_PASSWORD" \
  -A \
  -t cert \
  -f pkcs12 \
  -k "$KEYCHAIN_PATH"

# Allow codesign to use the key without UI prompts.
security set-key-partition-list \
  -S apple-tool:,apple:,codesign: \
  -s \
  -k "$KEYCHAIN_PASSWORD" \
  "$KEYCHAIN_PATH"

# Prepend our keychain so the imported identity is preferred.
EXISTING="$(security list-keychains -d user | sed -E 's/^[[:space:]]*"//g; s/"$//g' | tr '\n' ' ')"
# shellcheck disable=SC2086
security list-keychains -d user -s "$KEYCHAIN_PATH" $EXISTING

rm -f "$CERT_PATH"

echo "==> Available signing identities"
security find-identity -v -p codesigning "$KEYCHAIN_PATH" || security find-identity -v -p codesigning

# Export for later steps / package script.
if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    echo "KEYCHAIN_PATH=$KEYCHAIN_PATH"
    echo "KEYCHAIN_PASSWORD=$KEYCHAIN_PASSWORD"
  } >>"$GITHUB_ENV"
fi

echo "==> Certificate import complete"
