# Install

Prerequisites: **macOS 14+** and **Node.js 22.14+** for the npm capture tools.

- [App with Homebrew (recommended)](#app-with-homebrew-recommended)
- [App from a release](#app-from-a-release)
- [App from source](#app-from-source)
- [First launch](#first-launch)
- [Agent skills](skills.md)
- [Capture tools](capture.md)

---

## App with Homebrew (recommended)

```bash
brew install --cask ArchAstro/tools/astroshots
open -a Astroshots
```

The cask installs the same Developer ID signed and notarized build into
`/Applications`, straight from the release DMG. It declares `auto_updates true`,
so Sparkle keeps handling updates and Homebrew does not fight the app's own
self-update. To remove it: `brew uninstall --cask astroshots` (add `--zap` to
also delete preferences and caches).

> **Requires the cask to be merged in the tap.** `Casks/astroshots.rb` lives in
> [ArchAstro/homebrew-tools](https://github.com/ArchAstro/homebrew-tools), a
> separate repository. Until that change merges, `brew install --cask` reports
> `No available cask` — use [App from a release](#app-from-a-release) in the
> meantime. Verify with `brew info --cask ArchAstro/tools/astroshots`. The exact
> files and steps to land it are in
> [`plans/2026-08-17-homebrew-cask-tap-changes.md`](plans/2026-08-17-homebrew-cask-tap-changes.md).

---

## App from a release

Prefer a direct download, or not using Homebrew?

1. Download the latest versioned **Astroshots-x.y.z.dmg** from
   [Releases](https://github.com/ArchAstro/astroshots/releases).
2. Open the DMG and drag **Astroshots** into **Applications**.
3. Launch Astroshots and follow the same first-launch steps as below.

Builds are Developer ID signed and notarized so Gatekeeper accepts a normal
open. Installed copies can **Check for Updates…** (Sparkle) against the latest
GitHub Release appcast, however they were installed.

---

## App from source

macOS 14+, Xcode 26+, [XcodeGen](https://github.com/yonaskolb/XcodeGen)
(`brew install xcodegen`).

```bash
git clone https://github.com/ArchAstro/astroshots.git
cd astroshots/macos
./scripts/bootstrap.sh
open Astroshots.xcodeproj   # ⌘R
```

Details: [`macos/README.md`](../macos/README.md).

---

## First launch

1. Astroshots lives in the **menu bar** (no Dock icon). There is **no default
   watch folder**: first launch **asks which folders to watch** (the open panel
   starts in `~/Projects` when that folder exists). Choose the folder that
   contains your coding projects.
2. After setup it **warms from a local index** of known `.astroshot` folders and
   replays newer filesystem events, avoiding another full workspace walk.
3. Click the Astroshots icon → gear → **Add folders…** to watch more locations
   later.
4. **Right-click** the Astroshots icon → **Quit Astroshots** to exit (left-click
   opens the tray).

---

## Verify the whole path

From any project inside a watched folder — one command, no assets of your own:

```bash
cd /path/to/your/project
npx astroshot demo
```

`astroshot demo` writes real stills, a movie poster+video pair, and a
`manifest.json` into `.astroshot/astroshot-demo/`. It needs no Chromium and
works even with the app closed. Open the Astroshots menu-bar icon → **Shots**:
an **astroshot-demo** entry with a movie badge confirms the app and the on-disk
contract are connected.

If nothing appears — or before filing a bug — diagnose it in one line:

```bash
npx astroshot doctor
```

`doctor` reports Node version, whether this project is inside a folder
Astroshots actually watches, whether the app is installed and running, whether
the managed Chromium runtime is present, and macOS Screen Recording state — each
failing line carries the exact command that fixes it. It exits non-zero when a
required check fails, and it never installs anything or changes app state.

---

## Optional runtimes

| Need | Install |
|------|---------|
| Browser-backed stills and movies | `npx astroshot install-browser` |
| Native-window movie capture | macOS Screen Recording permission |
| Narrated friction-log videos | Apple Silicon; enabling Settings → Narration downloads the Qwen3-TTS model locally |
