//! Port of `packages/movie-harness/src/sources/browser.ts`: record a browser
//! journey as a movie.
//!
//! Deliberate divergences from the TS (PORTING.md decision 2):
//! - TS records with Playwright `recordVideo` and hands the `.webm` to
//!   `session.stop`. Here the page's CDP screencast is resampled to the
//!   session fps ([`resample_frames`]) and pushed as frames; `session.stop`
//!   encodes them, so the reported duration is the encoded video's.
//! - `scriptPath` runs in the Node helper: `playwright-core` attaches to this
//!   Chrome over CDP and passes the same page to the user's module.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::browser::{
    Browser, LaunchOptions, PageOptions, ScreenshotOptions, WaitUntil, resample_frames,
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
    let browser = Browser::launch(LaunchOptions {
        headed: opts.headed.unwrap_or(false),
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
        .new_page(PageOptions::new(size.width, size.height).scale(1.0))
        .await?;
    page.start_screencast(None).await?;
    let started = Instant::now();

    let url = opts.url.as_deref().filter(|url| !url.is_empty());
    if let Some(url) = url {
        page.goto(url, WaitUntil::DomContentLoaded, Duration::from_secs(60))
            .await?;
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

    let frames = page.stop_screencast().await?;
    let duration_ms = started.elapsed().as_millis();
    let fps = movie.fps.round().max(1.0) as u32;
    for frame in resample_frames(&frames, fps, duration_ms) {
        movie.push_frame(&frame, FrameExtension::Png)?;
    }
    // The final state is always the last frame, and the poster.
    movie.push_frame(&poster, FrameExtension::Png)?;
    let _ = page.close().await;

    movie
        .stop(StopOptions {
            status,
            poster_path: Some(path_string(poster_path)),
            ..Default::default()
        })
        .await
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
