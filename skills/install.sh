#!/usr/bin/env bash
# Optional local installer. Preferred:
#   npx skills add ArchAstro/astroshots --skill astroshots -g -y
#
# This script only re-links from a local clone of this repository.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/skills/astroshots"
echo "Note: preferred install is: npx skills add ArchAstro/astroshots --skill astroshots -g -y"
echo ""

install_link() {
  local dest="$1"
  mkdir -p "$(dirname "$dest")"
  ln -sfn "$SRC" "$dest"
  echo "linked $dest -> $SRC"
}

install_link "$HOME/.claude/skills/astroshots"
install_link "$HOME/.grok/skills/astroshots"

# Monorepo worktree (if present)
for wt in "$HOME/archastro"/firstlanding-wt*; do
  [[ -d "$wt/.claude/skills" ]] || continue
  dest="$wt/.claude/skills/astroshots"
  # Prefer a real copy so worktrees are self-contained when offline
  rm -rf "$dest"
  mkdir -p "$dest"
  cp -R "$SRC/SKILL.md" "$SRC/scripts" "$SRC/references" "$dest/"
  chmod +x "$dest/scripts/astroshot-capture"
  echo "copied $dest"
done

# Relative link so the repo works on every machine (not /Users/you/...).
mkdir -p "$ROOT/bin"
ln -sfn ../skills/astroshots/scripts/astroshot-capture "$ROOT/bin/astroshot-capture"
echo "bin: $ROOT/bin/astroshot-capture -> $(readlink "$ROOT/bin/astroshot-capture")"
echo "Done. Add to PATH if useful: export PATH=\"$ROOT/bin:\$PATH\""

