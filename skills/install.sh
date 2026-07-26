#!/usr/bin/env bash
# Optional installer from a local clone. Preferred:
#   npx skills add ArchAstro/astroshots --skill <skill-name> -g -y
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  bash ./skills/install.sh <skill-name> <destination>

Example:
  bash ./skills/install.sh astroshots "$HOME/.agents/skills/astroshots"

The destination must not already exist. This script creates one symlink from
the explicit destination to this clone; it never scans or rewrites worktrees.
EOF
}

if [[ $# -ne 2 || "$1" == "-h" || "$1" == "--help" ]]; then
  usage
  [[ $# -eq 1 ]] && exit 0
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SKILL_NAME="$1"
DESTINATION="$2"
SOURCE="$ROOT/skills/$SKILL_NAME"

if [[ ! "$SKILL_NAME" =~ ^[a-z0-9][a-z0-9-]*$ || ! -f "$SOURCE/SKILL.md" ]]; then
  echo "error: unknown skill: $SKILL_NAME" >&2
  exit 1
fi

if [[ -e "$DESTINATION" || -L "$DESTINATION" ]]; then
  echo "error: destination already exists; refusing to replace it: $DESTINATION" >&2
  exit 1
fi

mkdir -p "$(dirname "$DESTINATION")"
ln -s "$SOURCE" "$DESTINATION"
printf 'linked %s -> %s\n' "$DESTINATION" "$SOURCE"
