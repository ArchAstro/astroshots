//! Port of `packages/movie-harness/src/sources/browser.ts`: record a browser
//! journey as a movie.
//!
//! Deliberate divergences from the TS (PORTING.md decision 2):
//! - TS records with Playwright `recordVideo` and hands the `.webm` to
//!   `session.stop` with `durationMs: session.elapsedMs()`. Here the page's
//!   CDP screencast (`Page::start_recording`) goes through
//!   [`write_recorder_webm`], which keeps Playwright's timeline: fixed 25 fps
//!   whatever the session fps, the last frame held for at least a second.
//!   The manifest duration is the session wall clock, as in TS, not the
//!   video's length.
//! - Picture quality is deliberately higher than TS: the page is recorded at
//!   2 device pixels per CSS pixel from lossless screencast frames, as
//!   constant-quality VP9 with explicit BT.709 colour. The video and poster
//!   are therefore twice `size` in pixels. (Playwright: 1x, quality-90 JPEG
//!   frames, realtime VP8 at 1 Mbit/s.)
//! - Playwright bundles its own ffmpeg; this uses the one on PATH. Without
//!   it, the screencast is resampled to the session fps ([`resample_frames`])
//!   and `session.stop` encodes it with the Chromium fallback, so the duration
//!   is then the encoded video's.
//! - Screencast frames identical to their predecessor are dropped: Chrome
//!   re-sends the unchanged surface for the poster screenshot, which
//!   Playwright's headless shell does not, and that would restart the hold.
//! - A page that produced no screencast frame is recorded from the poster;
//!   Playwright writes a white frame.
//! - `scriptPath` runs in the Node helper: `playwright-core` attaches to this
//!   Chrome over CDP and passes the same page to the user's module.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::browser::{
    Browser, LaunchOptions, PageOptions, RECORDER_FPS, ScreencastFrame, ScreenshotOptions,
    WaitUntil, recorder_repeats, resample_frames, write_recorder_webm,
};
use crate::movie_harness::paths::resolve_lexically;
use crate::movie_harness::session::{FrameExtension, MovieSession, StopOptions, mkdtemp};
use crate::movie_harness::types::{
    BrowserMovieOptions, ManifestStatus, MovieArtifact, MovieSessionOptions, MovieSourceKind, Size,
};
use crate::node_helper::NodeHelper;

/// `Omit<MovieSessionOptions, "source"> & BrowserMovieOptions`. The
/// `session.source` field is ignored: it is always `browser`.
#[derive(Debug, Clone)]
pub struct BrowserMovieSessionOptions {
    pub session: MovieSessionOptions,
    pub browser: BrowserMovieOptions,
}

/// Default demo surface so a bare call still produces a movie.
const DEMO_HTML: &str = r#"<!doctype html>
        <html><body style="margin:0;display:grid;place-items:center;height:100vh;background:#090a12;color:#b9a8ff;font:28px ui-monospace,monospace">
          <div>astroshot-movie browser</div>
        </body></html>"#;

/// Device pixels per CSS pixel for the recording and its poster. TS recorded
/// at 1; stills are captured at 2, and text in a 1x video is soft on any
/// high-density display.
const RECORDING_SCALE: f64 = 2.0;

/// Scripts have no time limit in TS; give the helper a generous one.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

/// Record a headless (or headed) browser journey. Optional `script_path`
/// should export `default async function(page)` (or `run`).
pub async fn record_browser_movie(options: BrowserMovieSessionOptions) -> Result<MovieArtifact> {
    let BrowserMovieSessionOptions {
        mut session,
        browser: opts,
    } = options;
    let size = session.size.unwrap_or(Size {
        width: 1280,
        height: 720,
    });
    session.size = Some(size);
    session.source = MovieSourceKind::Browser;
    let status = session.status;
    let mut movie = MovieSession::create(session)?;

    let video_dir = mkdtemp("astroshot-bw-")?;
    let result = record(&mut movie, &opts, size, status, &video_dir).await;
    let _ = std::fs::remove_dir_all(&video_dir); // `finally { rmSync(force) }`
    result
}

async fn record(
    movie: &mut MovieSession,
    opts: &BrowserMovieOptions,
    size: Size,
    status: Option<ManifestStatus>,
    video_dir: &Path,
) -> Result<MovieArtifact> {
    // The screencast carries the window's real surface, so the density has
    // to be the browser's own: an emulated scale only affects screenshots.
    let browser = Browser::launch(LaunchOptions {
        headed: opts.headed.unwrap_or(false),
        args: vec![format!("--force-device-scale-factor={RECORDING_SCALE}")],
        ..Default::default()
    })
    .await?;
    let outcome = journey(&browser, movie, opts, size, status, video_dir).await;
    let _ = browser.close().await; // `.catch(() => undefined)`
    outcome
}

async fn journey(
    browser: &Browser,
    movie: &mut MovieSession,
    opts: &BrowserMovieOptions,
    size: Size,
    status: Option<ManifestStatus>,
    video_dir: &Path,
) -> Result<MovieArtifact> {
    let page = browser
        .new_page(PageOptions::new(size.width, size.height).scale(RECORDING_SCALE))
        .await?;
    let page_pixels = page.device_pixels(crate::browser::Size {
        width: size.width,
        height: size.height,
    });
    page.start_recording().await?;

    let url = opts.url.as_deref().filter(|url| !url.is_empty());
    if let Some(url) = url {
        page.goto(url, WaitUntil::DomContentLoaded, Duration::from_secs(60))
            .await?;
        page.refresh_screencast().await?;
    }

    if let Some(script_path) = opts.script_path.as_deref().filter(|path| !path.is_empty()) {
        let absolute = resolve_lexically(Path::new(script_path));
        run_script(browser, &page.target_id(), url, &absolute).await?;
    } else if url.is_none() {
        page.set_content(DEMO_HTML).await?;
        page.wait_for_timeout(400).await;
    }

    if let Some(settle_ms) = opts.settle_ms.filter(|ms| *ms > 0) {
        page.wait_for_timeout(settle_ms).await;
    }

    let poster = page.screenshot(ScreenshotOptions::default()).await?;
    let poster_path = video_dir.join("poster.png");
    std::fs::write(&poster_path, &poster)?;

    // `context.close()`: the recording ends here, after the poster.
    let mut recording = page.finish_screencast().await?;
    recording.drop_unchanged_frames();
    let idle_secs = recording.idle_secs();
    let mut frames = recording.frames;
    if frames.is_empty() {
        frames.push(ScreencastFrame {
            data: poster.clone(),
            at_ms: recording.stopped_at_ms,
            timestamp: 0.0,
        });
    }
    let _ = page.close().await;

    if which::which("ffmpeg").is_err() {
        return stop_without_ffmpeg(movie, &frames, idle_secs, status, poster_path).await;
    }
    // Keep a frame so stop() has a poster fallback even if video path is set.
    movie.push_frame(&poster, FrameExtension::Png)?;
    let video_path = path_string(video_dir.join("movie.webm"));
    write_recorder_webm(&frames, idle_secs, page_pixels, &video_path).await?;

    let duration_ms = movie.elapsed_ms();
    movie
        .stop(StopOptions {
            status,
            video_path: Some(video_path),
            poster_path: Some(path_string(poster_path)),
            duration_ms: Some(duration_ms),
        })
        .await
}

/// No ffmpeg: push the recording at the session fps, over the length
/// Playwright's recorder would have given it, and let `stop` encode.
async fn stop_without_ffmpeg(
    movie: &mut MovieSession,
    frames: &[ScreencastFrame],
    idle_secs: f64,
    status: Option<ManifestStatus>,
    poster_path: PathBuf,
) -> Result<MovieArtifact> {
    let timestamps: Vec<f64> = frames.iter().map(|frame| frame.timestamp).collect();
    let recorded: usize = recorder_repeats(&timestamps, idle_secs).iter().sum();
    let duration_ms = (recorded as u128 * 1000) / u128::from(RECORDER_FPS);
    let fps = movie.fps.round().max(1.0) as u32;
    for frame in resample_frames(frames, fps, duration_ms) {
        movie.push_frame(&frame, frame_extension(&frame))?;
    }
    // The final state is the last frame.
    movie.push_frame(&std::fs::read(&poster_path)?, FrameExtension::Png)?;
    movie
        .stop(StopOptions {
            status,
            poster_path: Some(path_string(poster_path)),
            ..Default::default()
        })
        .await
}

fn frame_extension(frame: &[u8]) -> FrameExtension {
    if frame.starts_with(&[0x89, b'P', b'N', b'G']) {
        FrameExtension::Png
    } else {
        FrameExtension::Jpg
    }
}

fn path_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

/// `runScript`: the user's module runs in the Node helper against this Chrome.
async fn run_script(
    browser: &Browser,
    target_id: &str,
    url: Option<&str>,
    absolute: &Path,
) -> Result<()> {
    if !absolute.exists() {
        bail!("browser script not found: {}", absolute.display());
    }
    let mut helper = NodeHelper::spawn()
        .await
        .context("start the Node helper for the browser script")?
        .with_request_timeout(SCRIPT_TIMEOUT);
    let ran = helper
        .browser_script(browser.ws_endpoint(), Some(target_id), url, absolute)
        .await;
    let _ = helper.shutdown().await;
    ran.map_err(Into::into)
}
