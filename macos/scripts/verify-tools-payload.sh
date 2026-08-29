#!/usr/bin/env bash
# Verify the staged offline tools payload, optionally also proving that Xcode
# embedded the identical resource in an Astroshots.app bundle.
set -euo pipefail

PAYLOAD="${1:-}"
APP="${2:-}"
usage() {
  echo "Usage: verify-tools-payload.sh PAYLOAD_DIRECTORY [Astroshots.app]" >&2
  exit 2
}
[[ -n "$PAYLOAD" ]] || usage
[[ -d "$PAYLOAD" ]] || { echo "verify-tools-payload: missing payload: $PAYLOAD" >&2; exit 1; }
[[ -z "$APP" || -d "$APP" ]] || { echo "verify-tools-payload: missing app: $APP" >&2; exit 1; }

fail() { echo "verify-tools-payload: $*" >&2; exit 1; }
[[ -f "$PAYLOAD/tools-payload-manifest.json" ]] || fail "missing manifest"
if find "$PAYLOAD" -type l -print -quit | grep -q .; then
  fail "payload contains a symlink"
fi

PAYLOAD="$PAYLOAD" python3 - <<'PY'
import hashlib
import json
import os
import stat
from pathlib import Path

root = Path(os.environ["PAYLOAD"]).resolve()
manifest_path = root / "tools-payload-manifest.json"
try:
    manifest = json.loads(manifest_path.read_text())
except (OSError, json.JSONDecodeError) as error:
    raise SystemExit(f"invalid manifest: {error}")

if manifest.get("schema") != 1 or manifest.get("payload_version") != 1:
    raise SystemExit("unsupported payload schema/version")
if manifest.get("launcher") != "astroshot/astroshot":
    raise SystemExit("unexpected launcher path")
node = manifest.get("node")
if not isinstance(node, dict) or node.get("path") != "runtime/bin/node":
    raise SystemExit("manifest does not declare the embedded Node runtime")
if not isinstance(node.get("version"), str) or not node["version"].startswith("v"):
    raise SystemExit("manifest does not declare a Node version")
if not isinstance(manifest.get("architecture"), str) or not manifest["architecture"].strip():
    raise SystemExit("manifest does not declare runtime architecture")
expected_skills = ["agent-browser", "astroshot", "astroshots-review", "browser-ui-harness", "friction-log", "screenshot"]
if manifest.get("skills") != expected_skills:
    raise SystemExit("manifest skill list is not the six shipped skills")

listed = set()
for entry in manifest.get("files", []):
    relative = entry.get("path")
    if not isinstance(relative, str) or not relative or relative.startswith("/") or ".." in Path(relative).parts:
        raise SystemExit(f"unsafe manifest path: {relative!r}")
    if relative in listed:
        raise SystemExit(f"duplicate manifest path: {relative}")
    listed.add(relative)
    path = root / relative
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"missing or unsafe payload file: {relative}")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != entry.get("sha256"):
        raise SystemExit(f"integrity mismatch: {relative}")
    executable = bool(path.stat().st_mode & stat.S_IXUSR)
    if executable != bool(entry.get("executable")):
        raise SystemExit(f"executable-bit mismatch: {relative}")

actual = {p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()}
actual.remove("tools-payload-manifest.json")
if actual != listed:
    raise SystemExit("manifest file list does not exactly match payload")
for skill in expected_skills:
    if f"skills/{skill}/SKILL.md" not in listed:
        raise SystemExit(f"missing skill document: {skill}")
for required in ("runtime/bin/node", "astroshot/bin/astroshot.mjs", "astroshot/astroshot"):
    if required not in listed:
        raise SystemExit(f"missing required runtime artifact: {required}")
PY

LAUNCHER="$PAYLOAD/astroshot/astroshot"
[[ -x "$LAUNCHER" ]] || fail "launcher is not executable"
NODE_RUNTIME="$PAYLOAD/runtime/bin/node"
[[ -x "$NODE_RUNTIME" ]] || fail "Node runtime is not executable"
manifest_node_version="$(PAYLOAD="$PAYLOAD" python3 -c 'import json, os; print(json.load(open(os.path.join(os.environ["PAYLOAD"], "tools-payload-manifest.json")))["node"]["version"])')"
[[ "$("$NODE_RUNTIME" --version)" == "$manifest_node_version" ]] || fail "runtime version differs from manifest"
# Reject Homebrew or other host-prefix dylib dependencies. System frameworks,
# system libraries, and relocatable loader/rpath references are the only valid
# dependencies; executing the copied runtime below proves those resolve inside
# the staged payload rather than from a user's Node installation.
#
# otool -L prints an unindented header per file, and one header *per slice* for
# a universal binary ("node (architecture arm64):"). Dependencies are always the
# tab-indented lines, so select on that indentation instead of dropping a fixed
# count of leading lines: a fat runtime would otherwise leak its second slice
# header through and have its own path misread as a host-library dependency.
if command -v otool >/dev/null 2>&1; then
  while IFS= read -r runtime_file; do
    # Only Mach-O images carry load commands. Skipping anything else keeps the
    # "no dependencies parsed" guard below meaningful instead of tripping over a
    # text stub that merely happens to be named like a library.
    file -b "$runtime_file" | grep -q 'Mach-O' || continue
    dependency_count=0
    while IFS= read -r dependency; do
      dependency_count=$((dependency_count + 1))
      case "$dependency" in
        /usr/lib/*|/System/Library/*|@loader_path/*|@executable_path/*|@rpath/*) ;;
        *) fail "runtime depends on host library: $dependency ($runtime_file)" ;;
      esac
    done < <(otool -L "$runtime_file" | grep -E '^[[:space:]]' | sed -E 's/^[[:space:]]*([^[:space:]]+).*/\1/')
    # Every Mach-O image links at least one library, so parsing nothing means
    # this inspection silently stopped working rather than genuinely passing.
    [[ "$dependency_count" -gt 0 ]] || fail "could not inspect runtime dependencies: $runtime_file"
  done < <(find "$PAYLOAD/runtime" -type f \( -name '*.dylib' -o -path '*/bin/node' \))
fi
# The launcher is deliberately a direct local exec, never an npm/npx install
# shim or a downloader. Keep this inspection narrow so CLI documentation and
# dependencies may legitimately mention network functionality.
if grep -Eiq '(npm|npx|curl|wget|https?://)' "$LAUNCHER"; then
  fail "launcher invokes an installer or network"
fi
"$LAUNCHER" --help >/dev/null || fail "offline launcher does not execute"

if [[ -n "$APP" ]]; then
  EMBEDDED="$APP/Contents/Resources/ToolsPayload"
  [[ -d "$EMBEDDED" ]] || fail "app does not embed ToolsPayload resource"
  if ! cmp -s "$PAYLOAD/tools-payload-manifest.json" "$EMBEDDED/tools-payload-manifest.json"; then
    fail "embedded manifest differs from staged payload"
  fi
  "$0" "$EMBEDDED"
fi

echo "Tools payload verification passed: $PAYLOAD"
