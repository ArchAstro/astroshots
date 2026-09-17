#!/usr/bin/env bash
# Credential-free unsigned assembly. Build-time network access only.
# Packaging must consume the result; it must never call this script.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/macos/build/Tools"
CACHE="$ROOT/macos/build/tools-downloads"
NODE_VERSION=22.16.0
# npm bundled inside the SHA-pinned Node tarball below. This is deliberately
# not the repo's packageManager version: the build prefix comes from the
# tarball, so it must assert what the tarball actually ships.
NPM_VERSION=10.9.2
case "$(uname -m)" in
  arm64) ARCH=arm64; SHA=1d7f34ec4c03e12d8b33481e5c4560432d7dc31a0ef3ff5a4d9a8ada7cf6ecc9 ;;
  x86_64) ARCH=x64; SHA=838d400f7e66c804e5d11e2ecb61d6e9e878611146baff69d6a2def3cc23f4ac ;;
  *) echo 'Unsupported macOS architecture' >&2; exit 1 ;;
esac
mkdir -p "$CACHE"
ARCHIVE="node-v${NODE_VERSION}-darwin-$ARCH.tar.gz"
if [[ ! -f "$CACHE/$ARCHIVE" ]]; then
  curl --fail --location --retry 3 "https://nodejs.org/dist/v${NODE_VERSION}/$ARCHIVE" -o "$CACHE/$ARCHIVE"
fi
echo "$SHA  $CACHE/$ARCHIVE" | shasum -a 256 -c -

# Full Node+npm prefix is build-time only. The payload ships node, not npm.
# Always extract from the hashed tarball so this prefix cannot drift.
BUILD_NODE="$CACHE/node-build"
rm -rf "$BUILD_NODE"
mkdir -p "$BUILD_NODE"
tar -xzf "$CACHE/$ARCHIVE" -C "$BUILD_NODE" --strip-components=1
export PATH="$BUILD_NODE/bin:$PATH"
BUILD_NODE_VERSION="$(node -p process.versions.node)"
if [[ "$BUILD_NODE_VERSION" != "$NODE_VERSION" ]]; then
  echo "Expected bundled node $NODE_VERSION, got $BUILD_NODE_VERSION" >&2
  exit 1
fi
BUILD_NPM_VERSION="$(npm -v)"
if [[ "$BUILD_NPM_VERSION" != "$NPM_VERSION" ]]; then
  echo "Expected bundled npm $NPM_VERSION, got $BUILD_NPM_VERSION" >&2
  exit 1
fi

cd "$ROOT"
# Always lockfile-install. prepack/pack need the reviewed workspace tree.
npm ci --ignore-scripts
# npm pack is the same artifact boundary exercised by pack:check.
# prepack builds engines from the lockfile-resolved workspace.
npm pack --workspace astroshot --pack-destination "$CACHE" >/dev/null
rm -rf "$OUT" "$CACHE/consumer"
mkdir -p "$OUT/node/bin" "$OUT/bin" "$CACHE/consumer"
tar -xzf "$CACHE/$ARCHIVE" -C "$OUT/node" --strip-components=1 \
  "node-v${NODE_VERSION}-darwin-$ARCH/bin/node" \
  "node-v${NODE_VERSION}-darwin-$ARCH/LICENSE"
VERSION="$(node -p "require('./packages/astroshot-unscoped/package.json').version")"
printf '{"private":true}\n' > "$CACHE/consumer/package.json"
# Scripts stay off. Optional native packages come from the lockfile-backed cache.
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --prefix "$CACHE/consumer" \
  --omit=dev --ignore-scripts --offline --no-audit --no-fund \
  "$CACHE/astroshot-$VERSION.tgz"
cp -RL "$CACHE/consumer" "$OUT/cli"
cp -RL "$ROOT/skills" "$OUT/skills"
python3 "$ROOT/macos/scripts/tools-payload.py" launcher "$OUT/bin/astroshot"
chmod +x "$OUT/bin/astroshot"
python3 "$ROOT/macos/scripts/tools-payload.py" prune "$OUT"
python3 "$ROOT/macos/scripts/tools-payload.py" seal "$OUT"
"$ROOT/macos/scripts/verify-tools-payload.sh" "$OUT"
