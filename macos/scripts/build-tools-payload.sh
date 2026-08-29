#!/usr/bin/env bash
# Stage the offline tools payload embedded in Astroshots.app. This deliberately
# copies an already-installed Node runtime and dependency closure: installing
# the app must never call npm or download executable content.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUTPUT="$ROOT/macos/Astroshots/Resources/ToolsPayload"
NODE_BINARY="${ASTROSHOTS_NODE_BINARY:-$(command -v node || true)}"
# Release CI supplies an x86_64 Node alongside its native arm64 Node so the
# embedded runtime matches Astroshots.app's universal binary.
NODE_X64_BINARY="${ASTROSHOTS_NODE_X64_BINARY:-}"
SKILLS=(agent-browser astroshot astroshots-review browser-ui-harness friction-log screenshot)

usage() {
  cat <<'EOF'
Usage: build-tools-payload.sh [--output PATH] [--node PATH]

Stages the versioned, offline Astroshots tools payload. Run `npm ci` and build
workspace CLIs first; this script does not install packages or contact a network.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) OUTPUT="${2:?missing output path}"; shift 2 ;;
    --node) NODE_BINARY="${2:?missing node path}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
done

fail() { echo "build-tools-payload: $*" >&2; exit 1; }
[[ "$(uname -s)" == "Darwin" ]] || fail "the embedded payload is macOS-only"
[[ -n "$NODE_BINARY" && -x "$NODE_BINARY" ]] || fail "a runnable Node binary is required (pass --node)"
NODE_BINARY="$(cd "$(dirname "$NODE_BINARY")" && pwd -P)/$(basename "$NODE_BINARY")"
NODE_ROOT="$(cd "$(dirname "$NODE_BINARY")/.." && pwd -P)"
[[ -d "$NODE_ROOT/bin" ]] || fail "Node runtime root is missing bin/"
if [[ -n "$NODE_X64_BINARY" ]]; then
  [[ -x "$NODE_X64_BINARY" ]] || fail "x86_64 Node runtime is not executable"
  NODE_X64_BINARY="$(cd "$(dirname "$NODE_X64_BINARY")" && pwd -P)/$(basename "$NODE_X64_BINARY")"
  lipo -archs "$NODE_X64_BINARY" | grep -qw x86_64 || fail "x86_64 Node runtime lacks an x86_64 slice"
  lipo -archs "$NODE_BINARY" | grep -qw arm64 || fail "primary Node runtime lacks an arm64 slice"
fi
[[ -d "$ROOT/node_modules" ]] || fail "missing node_modules; run npm ci before staging"
[[ -f "$ROOT/packages/astroshot-unscoped/bin/astroshot.mjs" ]] || fail "missing astroshot wrapper"

# Copy Node as a runtime home, not as an orphan executable: supported Node
# distributions may put libnode.dylib at ../lib relative to bin/node.
rm -rf "$OUTPUT"
mkdir -p "$OUTPUT/runtime/bin" "$OUTPUT/skills" "$OUTPUT/astroshot"
cp -p "$NODE_BINARY" "$OUTPUT/runtime/bin/node"
if [[ -n "$NODE_X64_BINARY" ]]; then
  lipo -create "$NODE_BINARY" "$NODE_X64_BINARY" -output "$OUTPUT/runtime/bin/node"
fi
if [[ -d "$NODE_ROOT/lib" ]]; then
  cp -RLp "$NODE_ROOT/lib" "$OUTPUT/runtime/lib"
fi

for skill in "${SKILLS[@]}"; do
  [[ -f "$ROOT/skills/$skill/SKILL.md" ]] || fail "missing skill: $skill"
  cp -RLp "$ROOT/skills/$skill" "$OUTPUT/skills/$skill"
done

# Copy the complete dependency closure, dereferencing workspace links. This is
# intentionally larger than the publish tarball: it includes every package the
# bundled CLI can resolve, including native macOS artifacts, without an install.
cp -RLp "$ROOT/packages/astroshot-unscoped/." "$OUTPUT/astroshot/"
# The wrapper's local node_modules can contain workspace links. Replace it with
# the dereferenced root closure so module resolution remains within the payload.
rm -rf "$OUTPUT/astroshot/node_modules"
cp -RLp "$ROOT/node_modules" "$OUTPUT/astroshot/node_modules"
# npm's cache is a build-time optimization, not part of the runtime closure.
rm -rf "$OUTPUT/astroshot/node_modules/.cache"

cat >"$OUTPUT/astroshot/astroshot" <<'EOF'
#!/bin/sh
# Offline launcher for the Astroshots.app-managed tools payload.
set -eu
PAYLOAD_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)"
exec "$PAYLOAD_ROOT/runtime/bin/node" "$PAYLOAD_ROOT/astroshot/bin/astroshot.mjs" "$@"
EOF
chmod 755 "$OUTPUT/astroshot/astroshot" "$OUTPUT/runtime/bin/node"

# Payloads are copied out of the signed app bundle, so links (especially links
# escaping the bundle) are never acceptable.
if find "$OUTPUT" -type l -print -quit | grep -q .; then
  fail "staged payload contains a symlink"
fi

OUTPUT="$OUTPUT" ROOT="$ROOT" python3 - <<'PY'
import hashlib
import json
import os
import subprocess
from pathlib import Path

root = Path(os.environ["OUTPUT"])
files = []
for path in sorted(p for p in root.rglob("*") if p.is_file()):
    relative = path.relative_to(root).as_posix()
    if relative == "tools-payload-manifest.json":
        continue
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    files.append({
        "path": relative,
        "sha256": digest,
        "executable": bool(path.stat().st_mode & 0o111),
    })
manifest = {
    "schema": 1,
    "payload_version": 1,
    "launcher": "astroshot/astroshot",
    "node": {
        "path": "runtime/bin/node",
        "version": subprocess.run(
            [str(root / "runtime/bin/node"), "--version"],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
        ).stdout.strip(),
    },
    "architecture": subprocess.run(
        ["file", "-b", str(root / "runtime/bin/node")],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip(),
    "skills": ["agent-browser", "astroshot", "astroshots-review", "browser-ui-harness", "friction-log", "screenshot"],
    "files": files,
}
(root / "tools-payload-manifest.json").write_text(
    json.dumps(manifest, sort_keys=True, indent=2) + "\n"
)
PY

"$ROOT/macos/scripts/verify-tools-payload.sh" "$OUTPUT"
echo "Tools payload staged at $OUTPUT"
