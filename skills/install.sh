#!/usr/bin/env bash
# Install Astroshots skills into local agent skill directories.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/skills/astroshots"

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

mkdir -p "$ROOT/bin"
ln -sfn "$SRC/scripts/astroshot-capture" "$ROOT/bin/astroshot-capture"
echo "bin: $ROOT/bin/astroshot-capture"
echo "Done. Add to PATH if useful: export PATH=\"$ROOT/bin:\$PATH\""
