//! Port of `packages/movie-harness/src`.
//!
//! The re-exports below are `index.ts`. Its `terminal-paint.ts` exports
//! (`createHeadlessTerminal`, `terminalPlainText`, `terminalToHtml`,
//! `writeTerminal`) are replaced by `crate::raster::HeadlessTerminal`.

pub mod capture_lock;
pub mod encode;
pub mod paths;
pub mod png;
pub mod session;
pub mod sink;
pub mod sources;
pub mod types;

pub use capture_lock::{CaptureLock, DEFAULT_LOCK_TIMEOUT, parse_lock_timeout_seconds};
pub use encode::{encode_frames, poster_from_frames};
pub use paths::{
    assert_kebab_case, assert_slug, assert_still_slug, default_run_id, feature_dir, humanize,
    resolve_root,
};
pub use png::{encode_rgb_png, encode_solid_png};
pub use session::MovieSession;
pub use sink::{
    FinalizeCurrentRunResult, SinkStillRequest, SinkStillResult, finalize_current_run,
    finalize_manifest, finalize_manifest_with_lock_timeout, sink_movie,
    sink_movie_with_lock_timeout, sink_still,
};
pub use sources::browser::record_browser_movie;
pub use sources::desktop_macos::{
    DesktopError, DesktopWindowInfo, DesktopWindowMatch, DesktopWindowMovieOptions,
    ScreenAccessReport, assert_desktop_toolchain, check_screen_recording_access,
    describe_desktop_window, desktop_match_from_flags, ensure_screen_recording_access,
    is_nearly_blank_png, list_desktop_windows, match_desktop_window,
    open_screen_recording_settings, record_desktop_window_movie,
};
pub use sources::frames_store::{
    load_frame_session, mark_frame_session, push_frame_to_session, start_frame_session,
    stop_frame_session,
};
pub use sources::pty::{load_pty_movie_fixture, record_pty_movie, record_truecolor_demo_movie};
pub use types::{
    BrowserMovieOptions, EncodeFramesRequest, ManifestStatus, MovieArtifact, MovieChapter,
    MovieFormat, MovieSessionOptions, MovieSourceKind, PersistedFrameSession, PtyAction, PtyKey,
    PtyMovieFixture, SinkMovieRequest, SinkMovieResult, Size,
};
