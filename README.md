# Astroshots

Menu-bar macOS app that live-watches harness screenshots under `.astroshot/`,
flashes new frames as desktop overlays, and streams every worktree in one list.

## Status

**v0.1 app is buildable.** Mock is the design source of truth.

| Artifact | Path |
|----------|------|
| Native macOS app | [`macos/`](macos/) |
| Interactive mock | [`docs/mocks/astroshots-menubar.html`](docs/mocks/astroshots-menubar.html) |
| Sibling UX reference | `../agent-rooms` |
| Harness reference | `../firstlanding-wt1/services/agent_network/test-harness/agent-browser` |

```bash
cd macos
./scripts/bootstrap.sh
open Astroshots.xcodeproj
# or:
xcodebuild -project Astroshots.xcodeproj -scheme Astroshots -destination 'platform=macOS' build
```

## Product shape

- **Two surfaces**: desktop overlay (automatic) + menu-bar stream (on demand).
- **Multi-worktree by default**: recursive watch on `~/archastro` (configurable). No worktree picker.
- **Tray**: stream → click for detail → gear for settings. Pin keeps a floating window.

## Harness write contract

```
<worktree>/.astroshot/<feature>/
  manifest.json
  0001-signed-in.png
  0002-configure.png
```

See [`macos/README.md`](macos/README.md) for the full manifest shape. Today’s
agent-browser harness still writes under `/tmp/archagents-browser-smoke/…`;
pointing it at `.astroshot/` is the next harness change.

## Skills (for coding agents)

Install once so Claude / Grok / monorepo agents load the write contract:

```bash
./skills/install.sh
```

| Skill | Purpose |
|-------|---------|
| `astroshots` | Write `.astroshot/<feature>/` frames, run the app, wire harnesses |

Helper on PATH after install:

```bash
export PATH="$HOME/archastro/astroshots/bin:$PATH"
astroshot-capture --feature my-journey --slug step --source ./shot.png
```

## CI / DMG

| Workflow | When | What |
|----------|------|------|
| **CI** | PRs / pushes | Unit tests only |
| **Release DMG** | tags `v*` | Developer ID sign + notarize → GitHub Release |

```bash
git tag v0.1.0 && git push origin v0.1.0
# → Release DMG workflow attaches Astroshots.dmg to the release
```

One-time Apple/GitHub setup: [`docs/SIGNING.md`](docs/SIGNING.md).
