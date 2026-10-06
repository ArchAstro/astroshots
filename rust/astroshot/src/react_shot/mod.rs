//! Port of `packages/react-shot/src`.
//!
//! `index.ts` only re-exports; the Rust paths are:
//! `closeSharedBrowser`/`takeShot` -> [`shot::close_shared_browser`]/[`shot::take_shot`],
//! `isDialogSelector`/`resolveShotMeta` -> [`meta::is_dialog_selector`]/[`meta::resolve_shot_meta`],
//! `BatchEntry`/`BatchManifest`/`ReactShotConfig`/`ShotRequest` -> [`types`] under the same names,
//! `ReactShotFixture` -> [`types::ReactShotFixtureMeta`].

pub mod batch_paths;
pub mod cli;
pub mod meta;
pub mod shot;
pub mod types;
