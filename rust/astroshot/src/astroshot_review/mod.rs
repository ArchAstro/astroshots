//! Port of `packages/astroshot-review/src`.
//!
//! The `pub use` lines below are `index.ts`: the package's public surface.

pub mod cli;
pub mod data;
pub mod images;
pub mod terminal;
pub mod ui;
pub mod video;

pub use cli::{ParsedArgs, parse_args, review_help, run};
pub use data::model::{FrictionLog, FrictionRun, FrictionStep, ReviewSnapshot, Shot};
pub use data::review_store::{add_comment, mark_seen, read_review_document, snapshot_from_entry};
pub use data::scan::{find_astroshot_dirs, scan_tree};
pub use data::store::{ReviewStore, StoreState};
pub use images::service::{ImageService, ImageServiceImpl};
pub use terminal::image_layer::ImageLayer;
pub use terminal::kitty::{encode_delete, encode_place, encode_transmit, parse_graphics_command};
pub use terminal::probe::{TerminalCapabilities, parse_probe_response, probe_terminal};
pub use video::ffmpeg::{FramePlayer, detect_ffmpeg, probe_video, split_png_stream};
