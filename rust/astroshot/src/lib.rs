//! The `astroshot` command line: argument parsers, help text, argv[0]
//! personalities, `doctor`, `demo` and `init`, on top of `astroshot-engine`
//! (capture, stills, movies, review store) and `astroshot-review` (the tray).
//! Module layout mirrors `packages/*/src/cli.ts` and `packages/astroshot/bin`;
//! see `rust/PORTING.md`.

pub mod bin;
pub mod cli;
