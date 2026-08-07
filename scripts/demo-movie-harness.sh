#!/usr/bin/env bash
# End-to-end demo: frames + browser + truecolor pty-demo → .astroshot/
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLI="$ROOT/packages/movie-harness/bin/astroshot-movie.mjs"
DEMO_ROOT="${1:-$ROOT/.movie-demo-root}"
mkdir -p "$DEMO_ROOT"

echo "== build =="
npm run build --workspace @archastro/movie-harness

RUN_ID="multi-context-demo-$$"

ASTROSHOT="$ROOT/packages/astroshot/bin/astroshot.mjs"

echo "== which-source (agent help) =="
node "$ASTROSHOT" movie which-source "ratatui truecolor dashboard"
node "$ASTROSHOT" movie which-source "SwiftUI native window"

echo "== frames source =="
node "$ASTROSHOT" movie run --source frames --feature multi-context --slug frames-flow \
  --root "$DEMO_ROOT" --run-id "$RUN_ID" --demo-frames 6 --fps 8 --status running

echo "== browser source =="
node "$ASTROSHOT" movie run --source browser --feature multi-context --slug browser-flow \
  --root "$DEMO_ROOT" --run-id "$RUN_ID" --url "https://example.com" --settle-ms 400 --status running

echo "== pty truecolor demo =="
node "$ASTROSHOT" movie run --source pty-demo --feature multi-context --slug pty-color \
  --root "$DEMO_ROOT" --run-id "$RUN_ID" --fps 8 --status running

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "== desktop.window (largest on-screen window, short) =="
  # Prefer Ghostty/Terminal if present; else first large on-screen window via list.
  WIN_JSON="$(node "$ASTROSHOT" movie list-windows 2>/dev/null || true)"
  WIN_ID="$(printf '%s' "$WIN_JSON" | node -e '
    let s=""; process.stdin.on("data",d=>s+=d); process.stdin.on("end",()=>{
      try {
        const w=JSON.parse(s).filter(x=>x.onScreen && x.width>=200 && x.height>=200);
        const pref=w.find(x=>/ghostty|terminal|iterm/i.test(x.owner)) || w[0];
        if (pref) process.stdout.write(String(pref.id));
      } catch {}
    });
  ')"
  if [[ -n "${WIN_ID:-}" ]]; then
    node "$ASTROSHOT" movie run --source desktop.window --feature multi-context --slug desktop-window \
      --root "$DEMO_ROOT" --run-id "$RUN_ID" --window-id "$WIN_ID" --duration-ms 600 --fps 5 --status running \
      || echo "(desktop.window skipped — grant Screen Recording to this terminal if captures fail)"
  else
    echo "(no window id resolved; skip desktop.window)"
  fi
fi

echo "== finalize =="
node "$ASTROSHOT" movie finalize --feature multi-context --run-id "$RUN_ID" --root "$DEMO_ROOT" --status pass

echo "== artifacts =="
find "$DEMO_ROOT/.astroshot" -type f | sort
echo "Demo root: $DEMO_ROOT"
echo "Run id: $RUN_ID"
