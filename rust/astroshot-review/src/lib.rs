//! Port of `packages/astroshot-review/src` (minus `cli.ts`, which is in the
//! `astroshot` crate): the review tray as a library.
//!
//! A host calls [`run_tray`] (or [`run_tray_blocking`]) with
//! [`TrayOptions`]. The tray prints nothing itself; argument parsing, help and
//! error messages belong to the program that embeds it.

pub mod data;
pub mod images;
pub mod mac_preferences;
pub mod terminal;
pub mod tray;
pub mod ui;
pub mod video;

pub use astroshot_engine::review_data::model::{
    FrictionLog, FrictionRun, FrictionStep, ReviewSnapshot, Shot,
};
pub use astroshot_engine::review_data::review_store::{
    add_comment, mark_seen, read_review_document, snapshot_from_entry,
};
pub use astroshot_engine::review_data::scan::{find_astroshot_dirs, scan_tree};
pub use data::store::{ReviewStore, StoreState};
pub use images::service::{ImageService, ImageServiceImpl};
pub use terminal::image_layer::ImageLayer;
pub use terminal::kitty::{encode_delete, encode_place, encode_transmit, parse_graphics_command};
pub use terminal::probe::{TerminalCapabilities, parse_probe_response, probe_terminal};
pub use tray::{ResolvedRoots, TrayError, TrayOptions, resolve_roots, run_tray, run_tray_blocking};
pub use ui::context::RootsSource;
pub use video::ffmpeg::{FramePlayer, detect_ffmpeg, probe_video, split_png_stream};
