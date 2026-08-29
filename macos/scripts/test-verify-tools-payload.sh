#!/usr/bin/env bash
# Focused proof for verify-tools-payload.sh's Mach-O dependency inspection.
#
# The embedded runtime is a universal binary in release CI, and `otool -L`
# prints a separate unindented header for every slice of a fat file. A verifier
# that strips a fixed number of leading lines misreads the second slice header
# as a dependency and rejects a perfectly good payload. These cases pin both
# directions: universal runtimes pass, host-linked runtimes still fail.
#
# Synthetic payloads keep this hermetic and fast: it needs only clang and
# python3, never Xcode, npm, a network, or a real Node distribution.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERIFY="$ROOT/macos/scripts/verify-tools-payload.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/astroshots-payload-proof.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

[[ "$(uname -s)" == "Darwin" ]] || { echo "SKIP: Mach-O inspection is macOS-only"; exit 0; }
command -v clang >/dev/null 2>&1 || { echo "SKIP: clang is required to build fixtures"; exit 0; }

SKILLS=(agent-browser astroshot astroshots-review browser-ui-harness friction-log screenshot)

# A minimal Mach-O that answers `--version` like the real runtime, so payloads
# reach the dependency inspection instead of failing an earlier gate.
cat >"$WORK/runtime.c" <<'EOF'
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
  if (argc > 1 && strcmp(argv[1], "--version") == 0) { printf("v22.14.0\n"); return 0; }
  return 0;
}
EOF

# Regenerate the manifest exactly as build-tools-payload.sh does, so a fixture
# never fails on a stale hash when the rule under test is what matters.
write_manifest() {
  OUTPUT="$1" python3 - <<'PY'
import hashlib, json, os, subprocess
from pathlib import Path

root = Path(os.environ["OUTPUT"])
files = []
for path in sorted(p for p in root.rglob("*") if p.is_file()):
    relative = path.relative_to(root).as_posix()
    if relative == "tools-payload-manifest.json":
        continue
    files.append({
        "path": relative,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "executable": bool(path.stat().st_mode & 0o111),
    })
run = lambda cmd: subprocess.run(cmd, check=True, text=True, stdout=subprocess.PIPE).stdout.strip()
manifest = {
    "schema": 1,
    "payload_version": 1,
    "launcher": "astroshot/astroshot",
    "node": {"path": "runtime/bin/node", "version": run([str(root / "runtime/bin/node"), "--version"])},
    "architecture": run(["file", "-b", str(root / "runtime/bin/node")]),
    "skills": ["agent-browser", "astroshot", "astroshots-review", "browser-ui-harness", "friction-log", "screenshot"],
    "files": files,
}
(root / "tools-payload-manifest.json").write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
PY
}

# Stage a payload shaped like the real one; trailing args go to clang so a case
# can pick its slices or link an extra library.
build_payload() {
  local payload="$1"; shift
  rm -rf "$payload"
  mkdir -p "$payload/runtime/bin" "$payload/astroshot/bin" "$payload/skills"
  clang "$WORK/runtime.c" "$@" -o "$payload/runtime/bin/node"
  for skill in "${SKILLS[@]}"; do
    mkdir -p "$payload/skills/$skill"
    printf '# %s\n' "$skill" >"$payload/skills/$skill/SKILL.md"
  done
  printf 'process.exit(0)\n' >"$payload/astroshot/bin/astroshot.mjs"
  cat >"$payload/astroshot/astroshot" <<'EOF'
#!/bin/sh
set -eu
PAYLOAD_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)"
exec "$PAYLOAD_ROOT/runtime/bin/node" "$PAYLOAD_ROOT/astroshot/bin/astroshot.mjs" "$@"
EOF
  chmod 755 "$payload/astroshot/astroshot" "$payload/runtime/bin/node"
  write_manifest "$payload"
}

FAILURES=0
expect_pass() {
  local label="$1" payload="$2"
  if "$VERIFY" "$payload" >/dev/null 2>"$WORK/err"; then
    echo "ok: $label"
  else
    echo "FAIL: $label should have passed: $(cat "$WORK/err")" >&2
    FAILURES=$((FAILURES + 1))
  fi
}
expect_fail() {
  local label="$1" payload="$2" pattern="$3"
  if "$VERIFY" "$payload" >/dev/null 2>"$WORK/err"; then
    echo "FAIL: $label should have been rejected" >&2
    FAILURES=$((FAILURES + 1))
  elif grep -q "$pattern" "$WORK/err"; then
    echo "ok: $label"
  else
    echo "FAIL: $label was rejected for the wrong reason: $(cat "$WORK/err")" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

# A single-slice runtime prints one header and must pass.
build_payload "$WORK/thin" -arch "$(uname -m)"
expect_pass "thin runtime is accepted" "$WORK/thin"

# The regression: a universal runtime prints one header per slice. Guard the
# fixture really is fat, otherwise this case proves nothing.
build_payload "$WORK/universal" -arch arm64 -arch x86_64
lipo -archs "$WORK/universal/runtime/bin/node" | grep -qw x86_64
lipo -archs "$WORK/universal/runtime/bin/node" | grep -qw arm64
expect_pass "universal runtime is accepted" "$WORK/universal"

# The rule must still bite. install_name points at a real, loadable path
# outside the payload and outside the allowlist, so the runtime still executes
# and the dependency inspection is the gate under test.
printf 'int helper(void){return 7;}\n' >"$WORK/helper.c"
mkdir -p "$WORK/hostlib"
HOST_DYLIB="$WORK/hostlib/libhost.dylib"
clang -arch arm64 -arch x86_64 -dynamiclib "$WORK/helper.c" -o "$HOST_DYLIB" -install_name "$HOST_DYLIB"
build_payload "$WORK/host-linked" -arch arm64 -arch x86_64 "$HOST_DYLIB"
"$WORK/host-linked/runtime/bin/node" --version >/dev/null
expect_fail "host-linked runtime is rejected" "$WORK/host-linked" "runtime depends on host library"

# A non-Mach-O file under runtime/ (some Node distributions ship .dylib-named
# text stubs) must be skipped rather than tripping the "parsed nothing" guard.
build_payload "$WORK/stub" -arch "$(uname -m)"
mkdir -p "$WORK/stub/runtime/lib"
printf 'not a mach-o image\n' >"$WORK/stub/runtime/lib/libstub.dylib"
write_manifest "$WORK/stub"
expect_pass "non-Mach-O file under runtime is skipped" "$WORK/stub"

if [[ "$FAILURES" -ne 0 ]]; then
  echo "Tools payload verifier proof failed: $FAILURES case(s)" >&2
  exit 1
fi
echo "Tools payload verifier proof passed"
