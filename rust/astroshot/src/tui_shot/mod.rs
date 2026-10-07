//! Port of `packages/tui-shot/src`.
//!
//! `index.ts` only re-exports; the same names are re-exported here.
//! `TuiShotFixture` is the serializable half of the TS interface (see
//! [`types::TuiShotFixture`]).

pub mod batch_paths;
pub mod cli;
pub mod kitty_graphics;
pub mod pty_exit_wrapper;
pub mod pty_shot;
pub mod shot;
pub mod types;

pub use kitty_graphics::{GraphicsOverlay, KittyGraphicsTracker, encode_png, overlays_to_html};
pub use pty_shot::take_pty_shot;
pub use shot::{close_shared_browser, take_tui_shot};
pub use types::{
    BatchEntry, BatchManifest, PtyAction, PtyKey, PtyShotFixture, PtyShotRequest, TuiShotFixture,
    TuiShotRequest,
};
