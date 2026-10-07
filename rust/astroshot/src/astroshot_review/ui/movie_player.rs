//! Port of `packages/astroshot-review/src/ui/movie-player.tsx`.
//!
//! Plays a movie into a cell box using ffmpeg frames and the graphics layer.
//! The poster stays underneath until the first frame lands.
//!
//! # Mapping
//!
//! - `<MoviePlayer>` is the [`MoviePlayer`] `StatefulWidget`; its Ink
//!   `width`/`height` props are the `Rect` it renders into. Refs, state and
//!   effects live in [`MoviePlayerState`], one per mounted player (drop = the
//!   effect cleanups: the frame handle unregisters, the decoder stops, the
//!   probe is cancelled). Effects run inside `render` through
//!   [`MoviePlayerState::sync`], diffed against the last applied
//!   dependencies, so each fires exactly when its React dependency array
//!   would have changed. They need a tokio runtime.
//! - The parent-owned `playback` prop and `onPlayback(patch)` callback become
//!   [`Playback`]: a cloneable shared handle. The parent reads
//!   [`Playback::snapshot`] and writes with [`Playback::patch`] (bump
//!   `seek_token` to request a seek); the player patches it from decoder and
//!   probe tasks. Every patch calls `wake` so the loop redraws.
//! - ffprobe and the ffmpeg decoder sit behind [`MovieBackend`] /
//!   [`FrameSource`] so tests inject a fake frame source. [`FfmpegBackend`]
//!   is the real one (`probe_video` and `FramePlayer`).
//!
//! Divergence: the TS closures read a stale `playback` captured when the
//! effect ran (`playback.durationMs`) or `positionRef` (last rendered
//! position). Here probe and decoder callbacks read the live [`Playback`]
//! snapshot, which is at least as fresh.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{StatefulWidget, Widget};
use tokio::task::JoinHandle;

use crate::astroshot_review::data::model::Chapter;
use crate::astroshot_review::images::png::ImageSize;
use crate::astroshot_review::terminal::image_layer::{CellBox, FrameHandle, ImageLayer};
use crate::astroshot_review::terminal::probe::GraphicsProtocol;
use crate::astroshot_review::ui::context::AppServices;
use crate::astroshot_review::ui::hooks::Wake;
use crate::astroshot_review::ui::picture::{Picture, PictureProps, PictureState};
use crate::astroshot_review::ui::put_spans;
use crate::astroshot_review::ui::theme::THEME;
use crate::astroshot_review::video::ffmpeg::{
    FfmpegInfo, FramePlayer, FramePlayerOptions, VideoInfo, probe_video,
};

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackState {
    pub playing: bool,
    pub position_ms: f64,
    pub duration_ms: Option<f64>,
    /// Bumped by the parent to request a seek to `position_ms`.
    pub seek_token: u64,
    pub error: Option<String>,
    pub ended: bool,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            playing: false,
            position_ms: 0.0,
            duration_ms: None,
            seek_token: 0,
            error: None,
            ended: false,
        }
    }
}

/// `Partial<PlaybackState>`: `None` leaves a field alone; the nested `Option`s
/// set a nullable field to a value or to `null`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackPatch {
    pub playing: Option<bool>,
    pub position_ms: Option<f64>,
    pub duration_ms: Option<Option<f64>>,
    pub seek_token: Option<u64>,
    pub error: Option<Option<String>>,
    pub ended: Option<bool>,
}

/// Shared playback state plus the `onPlayback` wake-up.
#[derive(Clone)]
pub struct Playback {
    state: Arc<Mutex<PlaybackState>>,
    wake: Wake,
}

impl Playback {
    pub fn new(initial: PlaybackState, wake: Wake) -> Self {
        Self {
            state: Arc::new(Mutex::new(initial)),
            wake,
        }
    }

    pub fn snapshot(&self) -> PlaybackState {
        self.lock().clone()
    }

    pub fn patch(&self, patch: &PlaybackPatch) {
        {
            let mut state = self.lock();
            if let Some(playing) = patch.playing {
                state.playing = playing;
            }
            if let Some(position_ms) = patch.position_ms {
                state.position_ms = position_ms;
            }
            if let Some(duration_ms) = patch.duration_ms {
                state.duration_ms = duration_ms;
            }
            if let Some(seek_token) = patch.seek_token {
                state.seek_token = seek_token;
            }
            if let Some(error) = &patch.error {
                state.error = error.clone();
            }
            if let Some(ended) = patch.ended {
                state.ended = ended;
            }
        }
        (self.wake)();
    }

    fn lock(&self) -> MutexGuard<'_, PlaybackState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub type ProbeFuture = Pin<Box<dyn Future<Output = Option<VideoInfo>> + Send>>;

/// A running decoder (`FramePlayer`): frames arrive through the callbacks it
/// was built with.
pub trait FrameSource: Send {
    fn start(&self);
    fn stop(&self);
}

impl FrameSource for FramePlayer {
    fn start(&self) {
        FramePlayer::start(self);
    }

    fn stop(&self) {
        FramePlayer::stop(self);
    }
}

/// ffprobe and the decoder factory.
pub trait MovieBackend: Send + Sync {
    fn probe(&self, video_path: &str) -> ProbeFuture;
    fn frame_player(&self, options: FramePlayerOptions) -> Box<dyn FrameSource>;
}

/// The real backend: `probeVideo` and `new FramePlayer`.
pub struct FfmpegBackend {
    pub ffmpeg: FfmpegInfo,
}

impl MovieBackend for FfmpegBackend {
    fn probe(&self, video_path: &str) -> ProbeFuture {
        let info = self.ffmpeg.clone();
        let path = video_path.to_string();
        Box::pin(async move { probe_video(&path, Some(&info)).await })
    }

    fn frame_player(&self, options: FramePlayerOptions) -> Box<dyn FrameSource> {
        Box::new(FramePlayer::new(options))
    }
}

const MAX_FRAME_WIDTH: u32 = 1024;
const POSTER_HINT: &str = "Poster shown · press O to open the movie";

/// `Math.round` for non-negative-ish values.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

#[derive(Default)]
struct Shared {
    info: Option<VideoInfo>,
    /// Bumped on every `setInfo`, so the decoder effect sees an info change.
    info_version: u64,
    /// Bumped per probe; a finished probe from an older one is dropped.
    probe_generation: u64,
    has_frame: bool,
}

/// What the decoder effect depends on.
#[derive(Debug, Clone, PartialEq)]
struct DecoderKey {
    info_version: u64,
    playing: bool,
    seek_token: u64,
    width: u16,
    height: u16,
    video_path: String,
}

/// Per-player component state (`useRef`/`useState` slots and effect bookkeeping).
pub struct MoviePlayerState {
    services: AppServices,
    layer: ImageLayer,
    backend: Arc<dyn MovieBackend>,
    wake: Wake,
    playback: Playback,
    shared: Arc<Mutex<Shared>>,
    enabled: bool,
    frames: Option<FrameHandle>,
    probed_path: Option<String>,
    probe_task: Option<JoinHandle<()>>,
    decoder_key: Option<DecoderKey>,
    player: Option<Box<dyn FrameSource>>,
    poster: Option<PictureState>,
}

impl MoviePlayerState {
    /// Uses the real ffmpeg backend.
    pub fn new(services: &AppServices, playback: Playback, wake: Wake) -> Self {
        let backend = Arc::new(FfmpegBackend {
            ffmpeg: services.ffmpeg.clone(),
        });
        Self::with_backend(services, playback, wake, backend)
    }

    pub fn with_backend(
        services: &AppServices,
        playback: Playback,
        wake: Wake,
        backend: Arc<dyn MovieBackend>,
    ) -> Self {
        let enabled = matches!(
            services.capabilities.graphics,
            GraphicsProtocol::Kitty | GraphicsProtocol::Herdr
        );
        Self {
            services: services.clone(),
            layer: services.layer.clone(),
            backend,
            wake,
            playback,
            shared: Arc::default(),
            enabled,
            frames: None,
            probed_path: None,
            probe_task: None,
            decoder_key: None,
            player: None,
            poster: None,
        }
    }

    pub fn playback(&self) -> &Playback {
        &self.playback
    }

    /// True once a decoded frame has been pushed (the poster is gone).
    pub fn has_frame(&self) -> bool {
        self.shared().has_frame
    }

    /// The probed video, `None` while probing or when it failed.
    pub fn info(&self) -> Option<VideoInfo> {
        self.shared().info.clone()
    }

    /// Whether the graphics layer plays frames (Kitty or herdr).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run the TS effects against this frame's props and box, in declaration order.
    pub fn sync(&mut self, video_path: &str, area: Rect) {
        let snapshot = self.playback.snapshot();

        // Effect 1: register the frame entry once (deps: enabled, layer).
        if self.enabled && self.frames.is_none() {
            let handle = self.layer.register_frames(Some(1));
            self.frames = Some(handle);
            self.layer.schedule_flush();
        }
        if let Some(frames) = &self.frames {
            frames.set_node(Some(CellBox {
                x: u32::from(area.x),
                y: u32::from(area.y),
                width: u32::from(area.width),
                height: u32::from(area.height),
            }));
        }

        // Effect 2: probe the file (deps: videoPath).
        if self.probed_path.as_deref() != Some(video_path) {
            self.probed_path = Some(video_path.to_string());
            self.start_probe(video_path);
        }

        // Effect 3: start/stop/seek the decoder.
        let key = DecoderKey {
            info_version: self.shared().info_version,
            playing: snapshot.playing,
            seek_token: snapshot.seek_token,
            width: area.width,
            height: area.height,
            video_path: video_path.to_string(),
        };
        if self.decoder_key.as_ref() != Some(&key) {
            self.decoder_key = Some(key);
            self.restart_decoder(video_path, area, &snapshot);
        }
    }

    fn start_probe(&mut self, video_path: &str) {
        if let Some(task) = self.probe_task.take() {
            task.abort();
        }
        let generation = {
            let mut shared = self.shared();
            shared.probe_generation += 1;
            shared.info = None;
            shared.info_version += 1;
            shared.probe_generation
        };
        let future = self.backend.probe(video_path);
        let shared = self.shared.clone();
        let playback = self.playback.clone();
        let has_ffmpeg = self.services.ffmpeg.ffmpeg.is_some();
        self.probe_task = Some(tokio::spawn(async move {
            let result = future.await;
            {
                let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                if shared.probe_generation != generation {
                    return;
                }
                shared.info = result.clone();
                shared.info_version += 1;
            }
            match result {
                Some(info) => {
                    let known = playback.snapshot().duration_ms.is_some_and(|d| d != 0.0);
                    if let Some(duration) = info.duration_ms.filter(|d| *d != 0)
                        && !known
                    {
                        playback.patch(&PlaybackPatch {
                            duration_ms: Some(Some(duration as f64)),
                            ..PlaybackPatch::default()
                        });
                    } else {
                        (playback.wake)();
                    }
                }
                None => playback.patch(&PlaybackPatch {
                    error: Some(Some(
                        if has_ffmpeg {
                            "Could not read this movie"
                        } else {
                            "Install ffmpeg to play movies"
                        }
                        .to_string(),
                    )),
                    ..PlaybackPatch::default()
                }),
            }
        }));
    }

    fn restart_decoder(&mut self, video_path: &str, area: Rect, snapshot: &PlaybackState) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
        let info = self.shared().info.clone();
        let Some(info) = info else { return };
        if !self.enabled || !snapshot.playing {
            return;
        }
        let Some(ffmpeg_path) = self.services.ffmpeg.ffmpeg.clone() else {
            self.playback.patch(&PlaybackPatch {
                playing: Some(false),
                error: Some(Some(
                    "Install ffmpeg to play movies (brew install ffmpeg)".to_string(),
                )),
                ..PlaybackPatch::default()
            });
            return;
        };
        let caps = &self.services.capabilities;
        let bounds = ImageSize {
            width: MAX_FRAME_WIDTH.min(u32::from(area.width) * caps.cell_width),
            height: (js_round(f64::from(MAX_FRAME_WIDTH) * 3.0 / 4.0) as u32)
                .min(u32::from(area.height) * caps.cell_height),
        };

        let last_report = Arc::new(Mutex::new(0.0_f64));
        let frames = self.frames.clone();
        let shared = self.shared.clone();
        let playback = self.playback.clone();
        let wake = self.wake.clone();
        let on_frame = {
            let playback = playback.clone();
            Box::new(
                move |frame: crate::astroshot_review::video::ffmpeg::VideoFrame| {
                    if let Some(frames) = &frames {
                        frames.push_frame(frame.png, frame.width, frame.height);
                    }
                    let first = {
                        let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                        !std::mem::replace(&mut shared.has_frame, true)
                    };
                    let mut last = last_report.lock().unwrap_or_else(|e| e.into_inner());
                    if frame.t_ms - *last >= 250.0 {
                        *last = frame.t_ms;
                        drop(last);
                        playback.patch(&PlaybackPatch {
                            position_ms: Some(frame.t_ms),
                            ..PlaybackPatch::default()
                        });
                    } else if first {
                        wake();
                    }
                },
            )
        };
        let on_end = {
            let playback = playback.clone();
            Box::new(move || {
                let live = playback.snapshot();
                playback.patch(&PlaybackPatch {
                    playing: Some(false),
                    ended: Some(true),
                    position_ms: Some(live.duration_ms.unwrap_or(live.position_ms)),
                    ..PlaybackPatch::default()
                });
            })
        };
        let on_error = Box::new(move |error: anyhow::Error| {
            playback.patch(&PlaybackPatch {
                playing: Some(false),
                error: Some(Some(error.to_string())),
                ..PlaybackPatch::default()
            });
        });

        let player = self.backend.frame_player(FramePlayerOptions {
            video_path: video_path.to_string(),
            bounds,
            source_size: ImageSize {
                width: info.width,
                height: info.height,
            },
            fps: None,
            start_ms: Some(snapshot.position_ms),
            on_frame,
            on_end,
            on_error,
            ffmpeg_path: Some(PathBuf::from(ffmpeg_path)),
        });
        player.start();
        self.player = Some(player);
    }
}

impl Drop for MoviePlayerState {
    fn drop(&mut self) {
        if let Some(task) = self.probe_task.take() {
            task.abort();
        }
        if let Some(player) = self.player.take() {
            player.stop();
        }
        if let Some(frames) = self.frames.take() {
            frames.unregister();
        }
    }
}

/// `<MoviePlayer>`: renders into the `Rect` it is given (the Ink `width`/`height`).
pub struct MoviePlayer<'a> {
    pub video_path: &'a str,
    pub poster_path: &'a str,
    pub poster_version: f64,
}

impl<'a> MoviePlayer<'a> {
    pub fn new(video_path: &'a str, poster_path: &'a str) -> Self {
        Self {
            video_path,
            poster_path,
            poster_version: 0.0,
        }
    }
}

impl StatefulWidget for MoviePlayer<'_> {
    type State = MoviePlayerState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut MoviePlayerState) {
        state.sync(self.video_path, area);
        if area.width == 0 || area.height == 0 {
            return;
        }
        if state.has_frame() {
            // The poster unmounts once a frame lands; dropping it unregisters its image.
            state.poster = None;
        } else {
            let wake = state.wake.clone();
            let services = &state.services;
            let poster = state
                .poster
                .get_or_insert_with(|| PictureState::new(services, wake));
            Picture::new(PictureProps {
                src: Some(self.poster_path),
                version: self.poster_version,
                label: Some("▶ movie"),
                ..PictureProps::default()
            })
            .render(area, buf, poster);
        }
        if !state.enabled {
            // Absolutely positioned and centered in the stage. Yoga rounds a
            // box's half-cell offset up (text nodes, like the poster caption,
            // round down), so on an even height the hint sits one row below it.
            let width = crate::astroshot_review::ui::text_width(POSTER_HINT) as u16;
            let x = area.x + area.width.saturating_sub(width).div_ceil(2);
            let y = area.y + area.height / 2;
            put_spans(
                buf,
                area,
                x,
                y,
                &[Span::styled(POSTER_HINT, Style::new().fg(THEME.muted))],
            );
        }
    }
}

/// `m:ss` (minutes are not capped at 59).
pub fn format_clock(ms: f64) -> String {
    let total = js_round(ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Position bar with chapter markers: `▶ ━━━●───┼── 0:05 / 1:00`.
pub struct ProgressBar<'a> {
    pub position_ms: f64,
    pub duration_ms: Option<f64>,
    pub chapters: &'a [Chapter],
    pub playing: bool,
}

impl Widget for ProgressBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = i64::from(area.width);
        let duration = self.duration_ms.filter(|d| *d != 0.0);
        let clock = format!(
            "{} / {}",
            format_clock(self.position_ms),
            duration.map_or_else(|| "--:--".to_string(), format_clock)
        );
        let bar_width = 4.max(width - clock.chars().count() as i64 - 6);
        let ratio = duration.map_or(0.0, |d| (self.position_ms / d).min(1.0));
        let filled = js_round(ratio * bar_width as f64) as i64;
        let markers: Vec<i64> = match duration {
            Some(d) => self
                .chapters
                .iter()
                .filter_map(|chapter| chapter.t_ms)
                .map(|t| (bar_width - 1).min(js_round(t / d * bar_width as f64) as i64))
                .collect(),
            None => Vec::new(),
        };
        let mut bar = String::new();
        for index in 0..bar_width {
            bar.push(if index == filled {
                '●'
            } else if markers.contains(&index) {
                '┼'
            } else if index < filled {
                '━'
            } else {
                '─'
            });
        }
        let (glyph, color) = if self.playing {
            ("▶ ", THEME.green)
        } else {
            ("⏸ ", THEME.muted)
        };
        put_spans(
            buf,
            area,
            area.x,
            area.y,
            &[
                Span::styled(glyph, Style::new().fg(color)),
                Span::styled(bar, Style::new().fg(THEME.purple)),
                Span::styled(format!(" {clock}"), Style::new().fg(THEME.muted)),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::terminal::image_layer::ImageLayerOptions;
    use crate::astroshot_review::ui::context::test_support::{FakeService, capabilities, services};
    use crate::astroshot_review::ui::testing::{fg_at, render_stateful, row, row_raw, rows};
    use crate::astroshot_review::video::ffmpeg::VideoFrame;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // ---- format_clock / ProgressBar -------------------------------------

    #[test]
    fn format_clock_is_minutes_and_padded_seconds_rounded_to_the_second() {
        assert_eq!(format_clock(0.0), "0:00");
        assert_eq!(format_clock(499.0), "0:00");
        assert_eq!(format_clock(500.0), "0:01");
        assert_eq!(format_clock(65_000.0), "1:05");
        assert_eq!(format_clock(3_600_000.0), "60:00");
        assert_eq!(format_clock(-5000.0), "0:00");
    }

    fn bar_buf(
        position_ms: f64,
        duration_ms: Option<f64>,
        chapters: &[Chapter],
        playing: bool,
        width: u16,
    ) -> Buffer {
        crate::astroshot_review::ui::testing::render(
            ProgressBar {
                position_ms,
                duration_ms,
                chapters,
                playing,
            },
            width,
            1,
        )
    }

    fn chapter(t_ms: Option<f64>) -> Chapter {
        Chapter {
            t_ms,
            ..Chapter::default()
        }
    }

    #[test]
    fn progress_bar_shows_head_filled_track_chapter_marker_and_clock() {
        let chapters = [chapter(Some(15_000.0)), chapter(None)];
        let buf = bar_buf(30_000.0, Some(60_000.0), &chapters, true, 40);
        // bar width = 40 - len("0:30 / 1:00") - 6 = 23; head at round(11.5) = 12; marker at round(5.75) = 6.
        assert_eq!(
            row(&buf, 0),
            format!("▶ {}●{} 0:30 / 1:00", "━━━━━━┼━━━━━", "─".repeat(10))
        );
        assert_eq!(fg_at(&buf, 0, 0), THEME.green);
        assert_eq!(fg_at(&buf, 2, 0), THEME.purple);
        assert_eq!(fg_at(&buf, 26, 0), THEME.muted);
    }

    #[test]
    fn progress_bar_paused_without_duration_has_head_at_start_and_placeholder_clock() {
        let chapters = [chapter(Some(5_000.0))];
        let buf = bar_buf(12_000.0, None, &chapters, false, 30);
        // bar width = 30 - len("0:12 / --:--") - 6 = 12; no duration: no ratio, no markers.
        assert_eq!(row(&buf, 0), format!("⏸ ●{} 0:12 / --:--", "─".repeat(11)));
        assert_eq!(fg_at(&buf, 0, 0), THEME.muted);
    }

    #[test]
    fn progress_bar_clamps_to_end_and_keeps_a_minimum_width() {
        let end = bar_buf(
            90_000.0,
            Some(60_000.0),
            &[chapter(Some(60_000.0))],
            true,
            30,
        );
        // bar width 13: the head (13) is past the end so the whole track is filled;
        // the marker clamps to the last cell.
        assert_eq!(row(&end, 0), format!("▶ {}┼ 1:30 / 1:00", "━".repeat(12)));
        // Tiny width: the bar never drops below 4 cells.
        let tiny = bar_buf(0.0, Some(60_000.0), &[], true, 8);
        // The row is clipped to the 8 columns it was given.
        assert_eq!(row_raw(&tiny, 0), "▶ ●─── 0");
    }

    // ---- fakes -----------------------------------------------------------

    struct FakePlayer {
        options: FramePlayerOptions,
        started: AtomicBool,
        stopped: AtomicBool,
    }

    impl FakePlayer {
        fn frame(&self, t_ms: f64, png: &[u8]) {
            (self.options.on_frame)(VideoFrame {
                png: png.to_vec(),
                width: 16,
                height: 8,
                t_ms,
                index: 0,
            });
        }
    }

    struct FakeHandle(Arc<FakePlayer>);

    impl FrameSource for FakeHandle {
        fn start(&self) {
            self.0.started.store(true, Ordering::SeqCst);
        }

        fn stop(&self) {
            self.0.stopped.store(true, Ordering::SeqCst);
        }
    }

    struct FakeBackend {
        info: Mutex<Option<VideoInfo>>,
        probed: Mutex<Vec<String>>,
        players: Mutex<Vec<Arc<FakePlayer>>>,
    }

    impl FakeBackend {
        fn new(info: Option<VideoInfo>) -> Arc<Self> {
            Arc::new(Self {
                info: Mutex::new(info),
                probed: Mutex::new(Vec::new()),
                players: Mutex::new(Vec::new()),
            })
        }

        fn player(&self, index: usize) -> Arc<FakePlayer> {
            self.players.lock().unwrap()[index].clone()
        }

        fn player_count(&self) -> usize {
            self.players.lock().unwrap().len()
        }
    }

    impl MovieBackend for FakeBackend {
        fn probe(&self, video_path: &str) -> ProbeFuture {
            self.probed.lock().unwrap().push(video_path.to_string());
            let info = self.info.lock().unwrap().clone();
            Box::pin(async move { info })
        }

        fn frame_player(&self, options: FramePlayerOptions) -> Box<dyn FrameSource> {
            let player = Arc::new(FakePlayer {
                options,
                started: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
            });
            self.players.lock().unwrap().push(player.clone());
            Box::new(FakeHandle(player))
        }
    }

    fn movie_info(duration_ms: Option<u64>) -> VideoInfo {
        VideoInfo {
            width: 1600,
            height: 900,
            duration_ms,
        }
    }

    struct Harness {
        state: MoviePlayerState,
        backend: Arc<FakeBackend>,
        writes: Arc<Mutex<Vec<String>>>,
        wakes: Arc<AtomicUsize>,
        services: AppServices,
    }

    fn harness(graphics: GraphicsProtocol, ffmpeg: bool, info: Option<VideoInfo>) -> Harness {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let sink = writes.clone();
        let mut services = services(graphics, FakeService::new(false));
        services.layer = ImageLayer::new(ImageLayerOptions::new(
            capabilities(graphics),
            services.service.clone(),
            move |data| sink.lock().unwrap().push(data.to_string()),
        ));
        if ffmpeg {
            services.ffmpeg.ffmpeg = Some("/usr/bin/ffmpeg".to_string());
            services.ffmpeg.ffprobe = Some("/usr/bin/ffprobe".to_string());
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let wake: Wake = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let backend = FakeBackend::new(info);
        let playback = Playback::new(PlaybackState::default(), wake.clone());
        let state = MoviePlayerState::with_backend(&services, playback, wake, backend.clone());
        Harness {
            state,
            backend,
            writes,
            wakes,
            services,
        }
    }

    impl Harness {
        fn draw(&mut self, width: u16, height: u16) -> Buffer {
            render_stateful(
                MoviePlayer::new("/shots/demo.mov", "/shots/demo.png"),
                &mut self.state,
                width,
                height,
            )
        }

        fn patch(&self, patch: PlaybackPatch) {
            self.state.playback().patch(&patch);
        }

        fn playback(&self) -> PlaybackState {
            self.state.playback().snapshot()
        }

        fn frame_writes(&self, png: &[u8]) -> usize {
            let encoded = STANDARD.encode(png);
            self.writes
                .lock()
                .unwrap()
                .iter()
                .filter(|w| w.contains(&encoded))
                .count()
        }
    }

    async fn settle() {
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }

    // ---- probe -----------------------------------------------------------

    #[tokio::test]
    async fn probe_reports_the_duration_once_and_keeps_a_known_one() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(61_500))),
        );
        h.draw(40, 10);
        assert_eq!(*h.backend.probed.lock().unwrap(), ["/shots/demo.mov"]);
        assert_eq!(h.state.info(), None);
        settle().await;
        assert_eq!(h.state.info(), Some(movie_info(Some(61_500))));
        assert_eq!(h.playback().duration_ms, Some(61_500.0));
        assert_eq!(h.playback().error, None);

        // A duration the parent already knows is not overwritten.
        let mut known = harness(GraphicsProtocol::Kitty, true, Some(movie_info(Some(9_000))));
        known.patch(PlaybackPatch {
            duration_ms: Some(Some(5_000.0)),
            ..PlaybackPatch::default()
        });
        known.draw(40, 10);
        settle().await;
        assert_eq!(known.playback().duration_ms, Some(5_000.0));
    }

    #[tokio::test]
    async fn probe_failure_asks_to_install_ffmpeg_or_reports_an_unreadable_movie() {
        let mut missing = harness(GraphicsProtocol::Kitty, false, None);
        missing.draw(40, 10);
        settle().await;
        assert_eq!(
            missing.playback().error.as_deref(),
            Some("Install ffmpeg to play movies")
        );

        let mut unreadable = harness(GraphicsProtocol::Kitty, true, None);
        unreadable.draw(40, 10);
        settle().await;
        assert_eq!(
            unreadable.playback().error.as_deref(),
            Some("Could not read this movie")
        );
    }

    #[tokio::test]
    async fn changing_the_video_path_probes_again_and_clears_the_info() {
        let mut h = harness(GraphicsProtocol::Kitty, true, Some(movie_info(None)));
        h.draw(40, 10);
        settle().await;
        assert!(h.state.info().is_some());
        render_stateful(
            MoviePlayer::new("/shots/other.mov", "/shots/demo.png"),
            &mut h.state,
            40,
            10,
        );
        assert_eq!(h.state.info(), None);
        assert_eq!(
            *h.backend.probed.lock().unwrap(),
            ["/shots/demo.mov", "/shots/other.mov"]
        );
        settle().await;
        assert!(h.state.info().is_some());
    }

    // ---- decoder state machine -------------------------------------------

    #[tokio::test]
    async fn nothing_decodes_until_the_probe_lands_and_playback_starts() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        // Probe still pending: no decoder.
        assert_eq!(h.backend.player_count(), 0);
        settle().await;
        h.draw(40, 10);
        assert_eq!(h.backend.player_count(), 1);
        // Redrawing with the same dependencies keeps the same decoder.
        h.draw(40, 10);
        assert_eq!(h.backend.player_count(), 1);

        let player = h.backend.player(0);
        assert!(player.started.load(Ordering::SeqCst));
        assert_eq!(player.options.video_path, "/shots/demo.mov");
        // 40x10 cells at 10x20 px; source 1600x900; 1024 and 768 caps.
        assert_eq!(
            player.options.bounds,
            ImageSize {
                width: 400,
                height: 200
            }
        );
        assert_eq!(
            player.options.source_size,
            ImageSize {
                width: 1600,
                height: 900
            }
        );
        assert_eq!(player.options.start_ms, Some(0.0));
        assert_eq!(
            player.options.ffmpeg_path,
            Some(PathBuf::from("/usr/bin/ffmpeg"))
        );
    }

    #[tokio::test]
    async fn frame_bounds_are_capped_at_1024_by_768_pixels() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(200, 80);
        settle().await;
        h.draw(200, 80);
        assert_eq!(
            h.backend.player(0).options.bounds,
            ImageSize {
                width: 1024,
                height: 768
            }
        );
    }

    #[tokio::test]
    async fn pausing_stops_the_decoder_and_resuming_restarts_from_the_position() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);
        h.backend.player(0).frame(1_000.0, b"F1");

        h.patch(PlaybackPatch {
            playing: Some(false),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        assert!(h.backend.player(0).stopped.load(Ordering::SeqCst));
        assert_eq!(h.backend.player_count(), 1);

        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        assert_eq!(h.backend.player_count(), 2);
        assert_eq!(h.backend.player(1).options.start_ms, Some(1_000.0));
        assert!(h.backend.player(1).started.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn bumping_the_seek_token_restarts_the_decoder_at_the_new_position() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);

        h.patch(PlaybackPatch {
            position_ms: Some(42_000.0),
            seek_token: Some(1),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        assert!(h.backend.player(0).stopped.load(Ordering::SeqCst));
        assert_eq!(h.backend.player_count(), 2);
        assert_eq!(h.backend.player(1).options.start_ms, Some(42_000.0));
    }

    #[tokio::test]
    async fn resizing_restarts_the_decoder_with_new_bounds() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);
        h.draw(60, 20);
        assert_eq!(h.backend.player_count(), 2);
        assert_eq!(
            h.backend.player(1).options.bounds,
            ImageSize {
                width: 600,
                height: 400
            }
        );
    }

    #[tokio::test]
    async fn playing_without_ffmpeg_stops_and_reports_how_to_install_it() {
        // The probe is faked, so it succeeds even though ffmpeg is "missing".
        let mut h = harness(
            GraphicsProtocol::Kitty,
            false,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);
        assert_eq!(h.backend.player_count(), 0);
        let state = h.playback();
        assert!(!state.playing);
        assert_eq!(
            state.error.as_deref(),
            Some("Install ffmpeg to play movies (brew install ffmpeg)")
        );
    }

    #[tokio::test]
    async fn without_kitty_graphics_nothing_decodes_and_the_poster_hint_shows() {
        let mut h = harness(GraphicsProtocol::None, true, Some(movie_info(Some(60_000))));
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(50, 5);
        settle().await;
        let buf = h.draw(50, 5);
        assert_eq!(h.backend.player_count(), 0);
        assert!(!h.state.enabled());
        // 40 columns centered in 50: 5 left; centered on row (5 - 1) / 2 = 2.
        assert_eq!(row_raw(&buf, 2).trim_end(), format!("     {POSTER_HINT}"));
        assert_eq!(fg_at(&buf, 5, 2), THEME.muted);
        // Too narrow to fit: clipped at the left edge of the stage.
        let narrow = h.draw(12, 3);
        assert_eq!(row(&narrow, 1), "Poster shown");
    }

    // ---- frames ----------------------------------------------------------

    #[tokio::test]
    async fn frames_are_pushed_to_the_layer_and_progress_is_reported_every_250ms() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);
        let player = h.backend.player(0);

        assert!(!h.state.has_frame());
        player.frame(100.0, b"PNG-A");
        assert!(h.state.has_frame());
        assert_eq!(h.frame_writes(b"PNG-A"), 1);
        // 100ms after the (zero) last report: below the 250ms threshold.
        assert_eq!(h.playback().position_ms, 0.0);

        player.frame(250.0, b"PNG-B");
        assert_eq!(h.playback().position_ms, 250.0);
        player.frame(400.0, b"PNG-C");
        assert_eq!(h.playback().position_ms, 250.0);
        player.frame(500.0, b"PNG-D");
        assert_eq!(h.playback().position_ms, 500.0);
        for png in [b"PNG-A", b"PNG-B", b"PNG-C", b"PNG-D"] {
            assert_eq!(h.frame_writes(png), 1);
        }
    }

    #[tokio::test]
    async fn the_poster_is_replaced_by_the_frame_once_the_first_frame_lands() {
        let mut h = harness(GraphicsProtocol::None, true, Some(movie_info(Some(60_000))));
        let before = h.draw(50, 5);
        assert!(h.state.poster.is_some());
        // The poster caption and the hint share the centre row; the hint draws on top.
        assert_eq!(row(&before, 2), format!("     {POSTER_HINT}"));
        // This mode cannot play movies, so set the flag the decoder callback would.
        h.state.shared().has_frame = true;
        let after = h.draw(50, 5);
        assert!(h.state.poster.is_none());
        assert!(h.state.has_frame());
        assert_eq!(row(&after, 2), format!("     {POSTER_HINT}"));
        assert_eq!(rows(&after).iter().filter(|r| !r.is_empty()).count(), 1);
    }

    #[tokio::test]
    async fn reaching_the_end_pauses_and_parks_at_the_duration() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);
        (h.backend.player(0).options.on_end)();
        let state = h.playback();
        assert!(!state.playing && state.ended);
        assert_eq!(state.position_ms, 60_000.0);

        // Unknown duration: keep the current position.
        let mut open = harness(GraphicsProtocol::Kitty, true, Some(movie_info(None)));
        open.patch(PlaybackPatch {
            playing: Some(true),
            position_ms: Some(7_000.0),
            ..PlaybackPatch::default()
        });
        open.draw(40, 10);
        settle().await;
        open.draw(40, 10);
        (open.backend.player(0).options.on_end)();
        assert_eq!(open.playback().position_ms, 7_000.0);
    }

    #[tokio::test]
    async fn a_decoder_error_pauses_and_carries_the_message() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);
        (h.backend.player(0).options.on_error)(anyhow::anyhow!("ffmpeg exited with code 1"));
        let state = h.playback();
        assert!(!state.playing);
        assert_eq!(state.error.as_deref(), Some("ffmpeg exited with code 1"));
        assert!(h.wakes.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test]
    async fn dropping_the_state_stops_the_decoder_and_unregisters_the_frame_entry() {
        let mut h = harness(
            GraphicsProtocol::Kitty,
            true,
            Some(movie_info(Some(60_000))),
        );
        h.patch(PlaybackPatch {
            playing: Some(true),
            ..PlaybackPatch::default()
        });
        h.draw(40, 10);
        settle().await;
        h.draw(40, 10);
        let player = h.backend.player(0);
        let frames_id = h.state.frames.as_ref().unwrap().id;
        let Harness {
            state, services, ..
        } = h;
        drop(state);
        assert!(player.stopped.load(Ordering::SeqCst));
        // A new registration after the drop does not reuse the live entry.
        assert_ne!(services.layer.register_frames(None).id, frames_id);
    }

    #[test]
    fn playback_patch_updates_only_the_named_fields_and_wakes() {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let playback = Playback::new(
            PlaybackState {
                position_ms: 10.0,
                error: Some("old".into()),
                ..PlaybackState::default()
            },
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        playback.patch(&PlaybackPatch {
            playing: Some(true),
            error: Some(None),
            duration_ms: Some(Some(5.0)),
            ..PlaybackPatch::default()
        });
        assert_eq!(
            playback.snapshot(),
            PlaybackState {
                playing: true,
                position_ms: 10.0,
                duration_ms: Some(5.0),
                seek_token: 0,
                error: None,
                ended: false,
            }
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
