# Porting astroshots to Rust

The TypeScript packages under `packages/` are being ported to one Rust crate,
`rust/astroshot`, module by module with [rustify](https://github.com/ArchAstro/rustify).
`rustify.toml` at the repository root defines the scope; port state lives in
`rust/port/`.

```bash
rustify status            # progress
rustify next --brief      # what to port next, and how
rustify done <ts file> --test <ts test>
rustify check
cd rust && cargo test && cargo clippy --all-targets -- -D warnings
```

## Scope

| Package | Ports to | Notes |
|---|---|---|
| `@archastro/astroshot` (`bin/*.mjs`) | `astroshot::bin::*` | The `astroshot` CLI. Lands in `src/bin/` as ordinary modules (`autobins = false`). |
| `@archastro/astroshot-review` | `astroshot::astroshot_review` | Terminal review tray. Ink → ratatui. |
| `@archastro/movie-harness` | `astroshot::movie_harness` | Movie capture and encoding. |
| `@archastro/tui-shot` | `astroshot::tui_shot` | Terminal screenshots. |
| `@archastro/react-shot` | `astroshot::react_shot` | Browser component screenshots. |
| `astroshot-unscoped`, `macos/`, `scripts/`, `skills/` | not ported | npm packaging, the Swift app, repo tooling. |

One binary, `astroshot`, replaces the five npm bins (`astroshot`,
`astroshot-review`, `astroshot-movie`, `react-shot`, `tui-shot`) as
subcommands. Each subcommand's flags, output, and exit codes match its TS bin.

## Decisions

1. **User components still render in Node.** `react-shot` and Ink fixtures
   load the user's own TSX (vite, `tsx/esm/api`, `ink`). That needs a JS
   runtime, so a small Node helper (`rust/node-helper/`) does only that:
   load config and fixtures, serve React fixtures with vite, render Ink
   fixtures to ANSI frames. Rust spawns it and talks JSON over stdio
   (`astroshot::node_helper`). TS modules whose job moves into the helper are
   recorded with `rustify replace <file> --with "node helper: <command>"`.
   Everything else (CLI, orchestration, PTY capture, rasterizing, encoding,
   review tray, on-disk contract) is Rust. Node is required only for React
   and Ink shots.
2. **No Playwright.**
   - Real browser work (React shots, browser movie sources) uses
     `chromiumoxide` over CDP (`astroshot::browser`).
   - Terminal frames are rasterized natively (`astroshot::raster`): a
     `vt100` screen → glyphs with `cosmic-text` → `tiny-skia` → PNG, using a
     bundled monospace font. TS turned terminals into HTML and screenshotted
     them in Chromium; the Rust output is a different pixel image of the same
     cells. Keep the color tables (`ANSI_16`, 256-color, truecolor), cell
     metrics, padding, and backgrounds from `terminal-html.ts` /
     `terminal-paint.ts` exactly.
   - Encoding uses `ffmpeg` as today. The TS fallback (replay frames in
     Chromium's recorder) becomes the same replay over CDP.
3. **Terminal emulation:** `@xterm/headless` → `vt100`. **PTY:** `node-pty` →
   `portable-pty`. **Ink UI:** ratatui + crossterm; screen state is a struct,
   input goes through `handle(event)`, the app loop is
   `crossterm::event::EventStream` + `tokio::select!`. The kitty graphics and
   halfblock image code ports as written; it writes escape sequences itself.
4. **Runtime:** tokio multi-thread. Errors: `thiserror` enums where callers
   match on them, `anyhow` at command boundaries. `node:worker_threads` image
   work → `rayon` / `spawn_blocking`.

## Compatibility rules

- **On-disk formats are a contract.** The macOS app and agent skills read
  `.astroshot/**` (`review.json`, manifests, friction logs, movie metadata).
  Field names, key order, number formatting, and file layout must be
  byte-identical to what TS writes. serde_json writes `1.0` where JS writes
  `1`; use a JS-exact number writer where it matters. Declare struct fields in
  the TS insertion order (`serde_json` has `preserve_order` on).
- **CLI surface is a contract.** Flags, defaults, help-relevant names, stdout,
  stderr, and exit codes match the TS bins. The `*.e2e.test.ts` suites and
  `packages/astroshot/test/*.test.mjs` are the binary-level tests; they will be
  run against the Rust binary.
- **Port tests case for case:** same test names (snake_case), same
  assertions. Record them with `rustify done --test`.
- Platform checks (`process.platform`) → `cfg!(target_os = ...)` or
  `std::env::consts::OS`, matching the TS branches.

## Working rules for batch ports

- Do not edit `rust/astroshot/Cargo.toml`. If a crate is missing, say so in
  your report; the lead adds it.
- Add `mod` lines for your files to their parent `mod.rs` / `lib.rs`; the lead
  merges parallel edits.
- Shared infrastructure (`node_helper`, `browser`, `raster`) is owned by its
  module; call it, don't re-implement it.
- Write deliberate divergences from TS into the `notes` of `rustify done`.
- Before finishing: `cargo test -p astroshot`, `cargo clippy -p astroshot
  --all-targets -- -D warnings`, `cargo fmt --all`.
