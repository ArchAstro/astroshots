//! Astroshots engine, ported from the TypeScript packages under `packages/`.
//! Module layout mirrors the TS tree; see `rust/PORTING.md`.
//!
//! This crate is a library for programs that capture and review shots. It
//! parses no argv, prints no CLI output, never calls `process::exit` and
//! knows no program name; operations return typed results and errors, and the
//! one process-level hook is [`self_exec`]. Async functions need a tokio
//! runtime (multi-thread recommended).

pub mod browser;
pub mod movie_harness;
pub mod node_helper;
pub mod raster;
pub mod react_shot;
pub mod review_data;
pub mod self_exec;
pub mod tui_shot;
pub mod video_encode;
