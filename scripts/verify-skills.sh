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

# Keep the one public npx entry point discoverable in the README and both
# mode-specific agent skills.
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
grep -Fq "astroshot tui" <<<"$help_output" ||
  fail "astroshot --help did not document TUI capture"
grep -Fq "install-browser" <<<"$help_output" ||
  fail "astroshot --help did not document browser installation"
node packages/astroshot/bin/astroshot.mjs react --help >/dev/null
node packages/astroshot/bin/astroshot.mjs tui --help >/dev/null

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

cat >"$VERIFY_TMP/tui-fixture.tsx" <<'EOF'
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

node packages/astroshot/bin/astroshot.mjs \
  react "$VERIFY_TMP/react-fixture.tsx" \
  -o "$DOCS_ASSETS/react-component.png"
node packages/astroshot/bin/astroshot.mjs \
  tui "$VERIFY_TMP/tui-fixture.tsx" \
  -o "$DOCS_ASSETS/terminal-state.png"

for asset in "$DOCS_ASSETS/react-component.png" "$DOCS_ASSETS/terminal-state.png"; do
  [[ -s "$asset" ]] || fail "documentation asset is empty: $asset"
  [[ "$(od -An -tx1 -N8 "$asset" | tr -d ' \n')" == "89504e470d0a1a0a" ]] ||
    fail "documentation asset is not a PNG: $asset"
done

cat >"$DOCS_CONTENT/component-guide.md" <<'EOF'
# Component guide

The account panel exposes the editable fields and primary action.

![Account panel with editable fields and the Save action](/screenshots/react-component.png)

The terminal confirmation state shows the final command before execution.

![Terminal confirmation state with the command ready to run](/screenshots/terminal-state.png)
EOF

cat >"$VERIFY_TMP/docs/screenshots.yaml" <<'EOF'
shots:
  - asset: public/screenshots/react-component.png
    source: react-fixture.tsx
    page: content/component-guide.md
    regenerate: astroshot react
  - asset: public/screenshots/terminal-state.png
    source: tui-fixture.tsx
    page: content/component-guide.md
    regenerate: astroshot tui
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
if (images.length !== 2) throw new Error("docs proof must embed both images");
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
  --slug terminal \
  --title "Terminal proof" \
  --description "astroshot tui generated this frame." \
  --source "$DOCS_ASSETS/terminal-state.png" \
  >/dev/null
MANIFEST="$STREAM_ROOT/.astroshot/skill-proof/manifest.json"
[[ "$(jq -r '.status' "$MANIFEST")" == "running" ]] ||
  fail "automated proof must leave visual review status running"
[[ "$(jq '.shots | length' "$MANIFEST")" == "2" ]] ||
  fail "integration manifest did not contain both package screenshots"
for file in react terminal; do
  jq -e --arg slug "$file" \
    '.shots[] | select(.slug == $slug) | .file' "$MANIFEST" >/dev/null ||
    fail "integration manifest missing $file screenshot"
done

echo "verify-skills: documentation PNGs are embedded, inventoried, and streamed for human review"
