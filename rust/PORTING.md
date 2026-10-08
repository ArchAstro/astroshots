# Porting astroshots to Rust

The TypeScript packages under `packages/` are being ported to three Rust
crates in `rust/`, module by module. `rustify.toml` at the repository root
defines the scope; port state lives in `rust/port/`.

```bash
cd rust && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

Port state convention: in `rust/port/index.toml`, `rust =` is a path relative
to `rust/` that starts with the crate directory (for example
`astroshot-engine/src/movie_harness/sink.rs`), and `module =` and symbol
values start with the crate name (`astroshot_engine::`, `astroshot_review::`
or `astroshot::`).

## Crates

The engine and the tray are libraries that other Rust programs link (archdev
exposes them as `archdev shots …` with its own commands and help). The
`astroshot` CLI is a consumer of those libraries, so everything that depends on
a program name lives there.

| Crate | Directory | Holds | Rule |
|---|---|---|---|
| `astroshot-engine` | `rust/astroshot-engine` | rasterizer, PTY and Ink stills, React shots and batches, browser driver, movie session, encode and sources, manifest sink (`sink_movie`, `sink_still`), `.astroshot/` readers and the review store (`review_data`), user-story readers, Node helper, `self_exec` | No argv parsing, no help text, no CLI output, no `process::exit`, no program name. Typed results and errors. |
| `astroshot-review` | `rust/astroshot-review` | the ratatui tray (`run_tray`, `TrayOptions`), terminal graphics, watcher, index cache, video playback, `mac_preferences` | Depends on the engine. Prints nothing. |
| `astroshot` | `rust/astroshot` | the `astroshot` binary: argument parsers, `const HELP` text, argv[0] personalities, `doctor`, `demo`, `init` | Output bytes and exit codes are a contract (see below). |

The one process-level hook is `astroshot_engine::self_exec`: on Windows the PTY
stills run the target under the current program re-executed with a hidden
`__pty-exit-wrapper` argument. A host sets `set_self_exec_prefix` and routes
that subcommand to `run_pty_exit_wrapper`; the `astroshot` binary uses the
defaults.

## Rust-only additions

These have no TS original. Their reference is the bash helper
`skills/astroshots-review/scripts/astroshot-capture` and the contract in
`docs/contract.md`; `port/index.toml` has no entries for them.

| Item | Where | Reference and differences |
|---|---|---|
| `sink_still`, `SinkStillRequest`, `SinkStillResult` | `movie_harness::sink` | `--source` of the helper. Same lock, sequence, run rules, in-place manifest edit and key order. Image check: the helper's signature/header/terminator checks, plus a full decode for PNG (`png` crate); the helper also decodes every format through `sips`/`magick`/`identify` when installed, which the engine does not (JPEG, GIF and WebP are structural only). Rejects `stories` and `friction-logs` (the helper does not). A generated run id that equals the manifest's current one gets a `-2`, `-3` suffix. |
| `finalize_current_run`, `FinalizeCurrentRunResult` | `movie_harness::sink` | `--status <s> --finalize` without `--run-id`. |
| `CaptureLock`, `parse_lock_timeout_seconds`, `DEFAULT_LOCK_TIMEOUT` | `movie_harness::capture_lock` | The helper's `.capture.lock` directory, `pid` file, 100 ms poll, stale-owner report. The engine never reads `ASTROSHOT_LOCK_TIMEOUT_SECONDS`; a host maps it with `parse_lock_timeout_seconds`. `sink_movie`, `finalize_manifest` and the two above take the lock too (`*_with_lock_timeout` variants set the wait). |
| `assert_still_slug` | `movie_harness::paths` | Slug rule of stills, `^[a-z0-9][a-z0-9_-]*$` (the helper's). Movies keep `assert_slug`, which rejects `_`. |
| `merge_reviews`, `ReviewMergeRequest`, `ReviewMergeEntry`, `ReviewMergeOutcome` | `review_data::review_store` | Merge of reviews made elsewhere: later `reviewed_at` wins, comments union by id, run id change resets. Uses the existing writer. |

Left as is: `sink_movie` (like `sink.ts`) writes a fresh manifest when a movie
starts a new run, while the bash helper edits the old manifest in place. A movie
that starts a new run therefore drops the previous manifest's `description` and
unknown keys; a still that starts a new run keeps them.

## Scope

| Package | Ports to | Notes |
|---|---|---|
| `@archastro/astroshot` (`bin/*.mjs`) | `astroshot::bin::*` | The `astroshot` CLI. Lands in `src/bin/` as ordinary modules (`autobins = false`). |
| `@archastro/astroshot-review` | `astroshot_review::*`, `astroshot_engine::review_data`, `astroshot::cli::review` | Terminal review tray (Ink → ratatui). `src/data` splits: the `.astroshot/` readers and writers are the engine's `review_data`; store, watcher and index cache stay with the tray. `cli.ts` is `astroshot::cli::review`. |
| `@archastro/movie-harness` | `astroshot_engine::movie_harness`, `astroshot::cli::movie` | Movie capture and encoding. `cli.ts` and `source-help.ts` are in the CLI crate. |
| `@archastro/tui-shot` | `astroshot_engine::tui_shot`, `astroshot::cli::tui_shot` | Terminal screenshots. Batch manifests are `tui_shot::batch`. |
| `@archastro/react-shot` | `astroshot_engine::react_shot`, `astroshot::cli::react_shot` | Browser component screenshots. Batch manifests are `react_shot::batch`. |
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
   (`astroshot_engine::node_helper`). TS modules whose job moves into the helper are
   recorded with `rustify replace <file> --with "node helper: <command>"`.
   Everything else (CLI, orchestration, PTY capture, rasterizing, encoding,
   review tray, on-disk contract) is Rust. Node is required only for React
   and Ink shots.
2. **No Playwright.**
   - Real browser work (React shots, browser movie sources) uses
     `chromiumoxide` over CDP (`astroshot_engine::browser`).
   - Terminal frames are rasterized natively (`astroshot_engine::raster`): a
     `alacritty_terminal` grid → glyphs with `cosmic-text` → `tiny-skia` → PNG, using a
     bundled monospace font. TS turned terminals into HTML and screenshotted
     them in Chromium; the Rust output is a different pixel image of the same
     cells. Keep the color tables (`ANSI_16`, 256-color, truecolor), cell
     metrics, padding, and backgrounds from `terminal-html.ts` /
     `terminal-paint.ts` exactly.
   - Browser movie `scriptPath` (a user JS module that drives a Playwright
     `Page`) keeps working: the Node helper runs it with `playwright-core`
     attached to the Rust-launched Chrome over CDP (`connectOverCDP`). Rust
     still owns the browser; `playwright-core` is needed only for scripted
     browser movies.
   - Encoding uses `ffmpeg` as today. The TS fallback (replay frames in
     Chromium's recorder) becomes the same replay over CDP. Known gap: the
     fallback WebM comes from `MediaRecorder`, so it has one frame per pushed
     frame and no container duration (TS: 25 fps with a duration).
   - Terminal PNGs use the bundled JetBrains Mono (`fontFamily` is ignored).
     Text it has no glyph for (CJK, emoji, some symbols) is drawn from the
     machine's fonts, as Chromium's fallback did.
   - Capture quality is deliberately above TS (measured with colour bars
     played in Chrome's `<video>`: TS encodes were off by 25-47 levels per
     channel and showed dark UI darker; these are within 1):
     - every ffmpeg encode converts to BT.709 limited range explicitly and
       tags the stream BT.709 primaries/matrix with the sRGB transfer
       (`video_encode.rs`); TS left it to ffmpeg's defaults, untagged;
     - constant quality (VP9 `-crf 15`, H.264 `-crf 14 -preset slow`)
       instead of VP9 at 2 Mbit/s, x264 defaults, and realtime VP8 at
       1 Mbit/s for browser recordings; Lanczos instead of nearest-neighbour
       when frames are resized; odd sizes are made even;
     - browser movies are recorded at 2 device pixels per CSS pixel from
       lossless PNG screencast frames (TS: 1x, quality-90 JPEG), so the video
       and poster are twice `--size`;
     - PTY movies are the frames' own pixel size (TS shrank each 2x frame to
       the CSS size plus margin, unevenly, with nearest neighbour), so the
       manifest `viewport` is that pixel size;
     - desktop captures are converted from the display's colour profile to
       sRGB with `sips`; the blank-frame check decodes the PNG instead of
       reading filtered bytes;
     - review playback scales with Lanczos instead of `fast_bilinear`.
   - Chrome discovery: `ASTROSHOT_CHROME`/`CHROME_PATH`, then an installed
     Chrome, then Playwright's cache. In the cache a headless launch takes
     `chromium_headless_shell-<rev>` first (what Playwright launched; Chrome
     for Testing takes about 1.5 s longer to start), a headed launch
     `chromium-<rev>`.
   - Chrome is driven over a websocket, not Playwright's pipe, so a `sh`
     watchdog (`browser/watchdog.rs`) kills it when the process dies.
3. **Terminal emulation:** `@xterm/headless` → `alacritty_terminal` (replacing
   the initial `vt100`, which drops dim, strikethrough, hidden, and
   autowrap-off). **PTY:** `node-pty` →
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

- Do not edit the `Cargo.toml` files. If a crate is missing, say so in
  your report; the lead adds it.
- Add `mod` lines for your files to their parent `mod.rs` / `lib.rs`; the lead
  merges parallel edits.
- Shared infrastructure (`node_helper`, `browser`, `raster`) is owned by its
  module; call it, don't re-implement it.
- Write deliberate divergences from TS into the `notes` of `rustify done`.
- Before finishing: `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --all`.
- A function with logic goes in a library crate with a typed request, result
  and error; the CLI crate parses, prints and maps the result to an exit code.
