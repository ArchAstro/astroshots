#!/usr/bin/env bash
# Canonical documentation/skill proof for the public screenshot CLI.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

fail() {
  echo "verify-skills: $*" >&2
  exit 1
}

require_reference() {
  local file="$1"
  local text="$2"
  [[ -f "$file" ]] || fail "missing $file"
  grep -Fq "$text" "$file" ||
    fail "$file does not document: $text"
}

# Keep the one public npx entry point discoverable in the README and capture
# skills.
require_reference "packages/astroshot/package.json" '"name": "@archastro/astroshot"'
for file in README.md packages/astroshot/README.md \
  skills/react-shot/SKILL.md skills/tui-shot/SKILL.md; do
  require_reference "$file" "npx --@archastro:registry=https://registry.npmjs.org"
  require_reference "$file" "@archastro/astroshot"
done

for skill in astroshots screenshot react-shot tui-shot agent-browser browser-ui-harness; do
  require_reference "skills/$skill/SKILL.md" "name: $skill"
done

for reference in \
  "@archastro/astroshot" \
  "agent-browser" \
  "astroshot-capture" \
  "documentation asset directory" \
  "alt text"; do
  require_reference "skills/screenshot/SKILL.md" "$reference"
done

# Exercise the same single CLI entry point that npx exposes. Build the internal
# engines only when a fresh checkout has no generated CLI.
for tool in react-shot tui-shot; do
  if [[ ! -f "packages/$tool/dist/cli.js" ]]; then
    npm run build --workspace "@archastro/$tool"
  fi
done
help_output="$(node packages/astroshot/bin/astroshot.mjs --help)"
grep -Fq "astroshot react" <<<"$help_output" ||
  fail "astroshot --help did not document React capture"
grep -Fq "astroshot ink" <<<"$help_output" ||
  fail "astroshot --help did not document Ink capture"
grep -Fq "astroshot pty" <<<"$help_output" ||
  fail "astroshot --help did not document PTY capture"
grep -Fq "install-browser" <<<"$help_output" ||
  fail "astroshot --help did not document browser installation"
node packages/astroshot/bin/astroshot.mjs react --help >/dev/null
node packages/astroshot/bin/astroshot.mjs ink --help >/dev/null
node packages/astroshot/bin/astroshot.mjs pty --help >/dev/null

if [[ "${ASTROSHOTS_VERIFY_INTEGRATION:-0}" != "1" ]]; then
  echo "verify-skills: command help and skill references pass"
  echo "verify-skills: set ASTROSHOTS_VERIFY_INTEGRATION=1 for PNG-to-Astroshots proof"
  exit 0
fi

command -v jq >/dev/null 2>&1 || fail "integration proof requires jq"

VERIFY_TMP="$(mktemp -d "$REPO_ROOT/.verify-skills.XXXXXX")"
cleanup() {
  rm -rf "$VERIFY_TMP"
}
trap cleanup EXIT

cat >"$VERIFY_TMP/package.json" <<'EOF'
{
  "private": true,
  "type": "module"
}
EOF

DOCS_ASSETS="$VERIFY_TMP/docs/public/screenshots"
DOCS_CONTENT="$VERIFY_TMP/docs/content"
mkdir -p "$DOCS_ASSETS" "$DOCS_CONTENT"

cat >"$VERIFY_TMP/react-fixture.tsx" <<'EOF'
import React from "react";

export default {
  width: 420,
  height: 180,
  selector: "[data-proof]",
  waitFor: "text=React fixture proof",
  component: React.createElement(
    "div",
    {
      "data-proof": true,
      style: {
        background: "#111827",
        color: "#f9fafb",
        font: "600 20px system-ui",
        padding: "24px",
      },
    },
    "React fixture proof",
  ),
};
EOF

cat >"$VERIFY_TMP/ink-fixture.tsx" <<'EOF'
import React from "react";
import { Box, Text } from "ink";

export default {
  cols: 44,
  rows: 6,
  scale: 1,
  expectText: ["Terminal fixture proof"],
  component: React.createElement(
    Box,
    { borderStyle: "round", paddingX: 1 },
    React.createElement(Text, { color: "cyan" }, "Terminal fixture proof"),
  ),
};
EOF

cat >"$VERIFY_TMP/pty-program.mjs" <<'EOF'
process.stdin.setEncoding("utf8");
process.stdin.setRawMode(true);
function render(status = "Choose a terminal state") {
  process.stdout.write(
    "\u001b[?1049h\u001b[2J\u001b[H\u001b[36;1mPTY documentation proof\u001b[0m\r\n" + status,
  );
}
process.stdin.on("data", (data) => {
  if (data.includes("\r")) render("Ready to document");
});
process.on("SIGHUP", () => process.exit(0));
render();
process.stdin.resume();
EOF

cat >"$VERIFY_TMP/pty-fixture.yaml" <<'EOF'
version: 1
command: node
args: [pty-program.mjs]
cols: 44
rows: 6
scale: 1
actions:
  - waitFor: Choose a terminal state
  - key: enter
  - waitFor: Ready to document
expectText:
  - PTY documentation proof
  - Ready to document
EOF

node packages/astroshot/bin/astroshot.mjs \
  react "$VERIFY_TMP/react-fixture.tsx" \
  -o "$DOCS_ASSETS/react-component.png"
node packages/astroshot/bin/astroshot.mjs \
  ink "$VERIFY_TMP/ink-fixture.tsx" \
  -o "$DOCS_ASSETS/ink-state.png"
node packages/astroshot/bin/astroshot.mjs \
  pty "$VERIFY_TMP/pty-fixture.yaml" \
  -o "$DOCS_ASSETS/pty-state.png"

for asset in \
  "$DOCS_ASSETS/react-component.png" \
  "$DOCS_ASSETS/ink-state.png" \
  "$DOCS_ASSETS/pty-state.png"; do
  [[ -s "$asset" ]] || fail "documentation asset is empty: $asset"
  [[ "$(od -An -tx1 -N8 "$asset" | tr -d ' \n')" == "89504e470d0a1a0a" ]] ||
    fail "documentation asset is not a PNG: $asset"
done

cat >"$DOCS_CONTENT/component-guide.md" <<'EOF'
# Component guide

The account panel exposes the editable fields and primary action.

![Account panel with editable fields and the Save action](/screenshots/react-component.png)

The Ink confirmation state shows the final command before execution.

![Ink confirmation state with the command ready to run](/screenshots/ink-state.png)

The PTY state proves the real terminal program handled input.

![Interactive terminal program ready for documentation](/screenshots/pty-state.png)
EOF

cat >"$VERIFY_TMP/docs/screenshots.yaml" <<'EOF'
shots:
  - asset: public/screenshots/react-component.png
    source: react-fixture.tsx
    page: content/component-guide.md
    regenerate: astroshot react
  - asset: public/screenshots/ink-state.png
    source: ink-fixture.tsx
    page: content/component-guide.md
    regenerate: astroshot ink
  - asset: public/screenshots/pty-state.png
    source: pty-fixture.yaml
    page: content/component-guide.md
    regenerate: astroshot pty
EOF

node --input-type=module - "$VERIFY_TMP/docs" <<'EOF'
import fs from "node:fs";
import path from "node:path";

const docsRoot = process.argv[2];
const markdown = fs.readFileSync(
  path.join(docsRoot, "content/component-guide.md"),
  "utf8",
);
const images = [...markdown.matchAll(/!\[([^\]]+)\]\(\/screenshots\/([^)]+)\)/g)];
if (images.length !== 3) throw new Error("docs proof must embed all three images");
for (const [, alt, filename] of images) {
  if (!alt.trim() || /^screenshot of\b/i.test(alt)) {
    throw new Error(`docs proof has non-instructional alt text: ${alt}`);
  }
  const asset = path.join(docsRoot, "public/screenshots", filename);
  if (!fs.existsSync(asset)) {
    throw new Error(`docs proof references a missing or case-mismatched asset: ${asset}`);
  }
}
const inventory = fs.readFileSync(path.join(docsRoot, "screenshots.yaml"), "utf8");
for (const [, , filename] of images) {
  if (!inventory.includes(`public/screenshots/${filename}`)) {
    throw new Error(`docs inventory is missing ${filename}`);
  }
}
EOF

STREAM_ROOT="$VERIFY_TMP/project"
mkdir -p "$STREAM_ROOT"

bash bin/astroshot-capture \
  --root "$STREAM_ROOT" \
  --feature skill-proof \
  --slug react \
  --title "React proof" \
  --description "astroshot react generated this frame." \
  --source "$DOCS_ASSETS/react-component.png" \
  >/dev/null
bash bin/astroshot-capture \
  --root "$STREAM_ROOT" \
  --feature skill-proof \
  --slug ink \
  --title "Ink proof" \
  --description "astroshot ink generated this frame." \
  --source "$DOCS_ASSETS/ink-state.png" \
  >/dev/null
bash bin/astroshot-capture \
  --root "$STREAM_ROOT" \
  --feature skill-proof \
  --slug pty \
  --title "PTY proof" \
  --description "astroshot pty generated this frame." \
  --source "$DOCS_ASSETS/pty-state.png" \
  >/dev/null
MANIFEST="$STREAM_ROOT/.astroshot/skill-proof/manifest.json"
[[ "$(jq -r '.status' "$MANIFEST")" == "running" ]] ||
  fail "automated proof must leave visual review status running"
[[ "$(jq '.shots | length' "$MANIFEST")" == "3" ]] ||
  fail "integration manifest did not contain all package screenshots"
for file in react ink pty; do
  jq -e --arg slug "$file" \
    '.shots[] | select(.slug == $slug) | .file' "$MANIFEST" >/dev/null ||
    fail "integration manifest missing $file screenshot"
done

echo "verify-skills: documentation PNGs are embedded, inventoried, and streamed for human review"
