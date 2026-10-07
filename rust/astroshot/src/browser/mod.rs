//! Real-browser automation over CDP (`chromiumoxide`), replacing the parts of
//! Playwright that drive an actual browser. See `rust/PORTING.md`, decision 2.
//!
//! Terminal frames do not go through here; `astroshot::raster` draws them.
//!
//! # What replaces what
//!
//! | Playwright | Here |
//! |---|---|
//! | `chromium.launch({headless})` + shared browser | [`Browser::shared`] / [`Browser::launch`] |
//! | `browser.newPage({viewport, deviceScaleFactor})` | [`Browser::new_page`] |
//! | `page.goto(url, {waitUntil, timeout})` | [`Page::goto`] |
//! | `page.setContent(html, {waitUntil: "load"})` | [`Page::set_content`] |
//! | `page.waitForSelector(sel, {state: "visible"})` | [`Page::wait_for_selector`] |
//! | `page.getByText(t).first().waitFor()` / `waitFor: "text=..."` | [`Page::wait_for_text`] |
//! | `page.waitForFunction(fn)` | [`Page::wait_for_function`] |
//! | `page.evaluate(fn)` | [`Page::evaluate`] |
//! | `page.locator(sel).count()` / `.boundingBox()` / `innerText()` | [`Page::count`] / [`Page::bounding_box`] / [`Page::inner_text`] |
//! | `page.setViewportSize` / `viewportSize` | [`Page::set_viewport`] / [`Page::viewport`] |
//! | `page.on("pageerror" / "console")` | [`Page::page_errors`] |
//! | `page.waitForTimeout(ms)` | [`Page::wait_for_timeout`] |
//! | `locator.screenshot({omitBackground})` | [`Page::screenshot_element`] |
//! | `page.screenshot({fullPage, omitBackground})` | [`Page::screenshot`] |
//! | `recordVideo` (browser movie source) | [`Page::start_recording`] + [`Page::finish_screencast`] + [`write_recorder_webm`] |
//! | `recordVideo` (encode fallback, no ffmpeg) | [`record_webm`] |
//! | `page.close()` / `browser.close()` | [`Page::close`] / [`Browser::close`] |
//!
//! # Selectors
//!
//! Selectors are CSS (`document.querySelector`), plus the `text=...` /
//! `text/...` form `react-shot` uses for `waitFor` (case-insensitive substring
//! of a visible element's text, like `getByText(..., {exact: false})`). Other
//! Playwright engines (`xpath=`, `>>` chains, `:has-text`) are not supported.
//!
//! # Runtime
//!
//! A [`Browser`] spawns its CDP handler on the tokio runtime that launched it.
//! The shared browser notices when that runtime has gone away and relaunches.

mod discover;
mod record;
mod webm;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use chromiumoxide::browser::{BrowserConfig, HeadlessMode};
use chromiumoxide::cdp::browser_protocol::browser::{
    Bounds, GetWindowForTargetParams, SetWindowBoundsParams,
};
use chromiumoxide::cdp::browser_protocol::dom::Rgba;
use chromiumoxide::cdp::browser_protocol::emulation::{
    ClearDeviceMetricsOverrideParams, MediaFeature, SetDefaultBackgroundColorOverrideParams,
    SetDeviceMetricsOverrideParams, SetEmulatedMediaParams,
};
use chromiumoxide::cdp::browser_protocol::log::{EventEntryAdded, LogEntryLevel};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams, EventScreencastFrame, GetLayoutMetricsParams,
    NavigateParams, ScreencastFrameAckParams, StartScreencastFormat, StartScreencastParams,
    StopScreencastParams, Viewport as CdpClip,
};
use chromiumoxide::cdp::js_protocol::runtime::{
    ConsoleApiCalledType, EvaluateParams, EventConsoleApiCalled, EventExceptionThrown,
};
use futures::StreamExt;
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;

pub use discover::{CHROME_ENV_VARS, find_chrome};
pub use record::{
    RECORDER_FPS, RecordedVideo, recorder_ffmpeg_args, recorder_repeats, write_recorder_webm,
};
pub use webm::record_webm;

/// Playwright's default action timeout is 30s; React shot overrides per call.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error(
        "No Chrome or Chromium found. Set ASTROSHOT_CHROME (or CHROME_PATH) to a browser executable, or install Chrome. Searched:\n{}",
        .searched.iter().map(|s| format!("  - {s}")).collect::<Vec<_>>().join("\n")
    )]
    ChromeNotFound { searched: Vec<String> },
    #[error("Could not launch Chromium.\n{0}")]
    Launch(String),
    #[error("Timed out after {ms}ms waiting for {what}")]
    Timeout { what: String, ms: u128 },
    #[error("Browser protocol error: {0}")]
    Cdp(String),
    #[error("Page script failed: {0}")]
    Script(String),
    #[error("Navigation to {url} failed: {reason}")]
    Navigation { url: String, reason: String },
    #[error("No element matches {0:?}")]
    NoElement(String),
    #[error("Screencast: {0}")]
    Screencast(String),
}

pub type Result<T> = std::result::Result<T, BrowserError>;

fn cdp<E: std::fmt::Display>(error: E) -> BrowserError {
    BrowserError::Cdp(error.to_string())
}

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Show the browser window (`--headed`).
    pub headed: bool,
    /// Explicit executable; otherwise [`find_chrome`].
    pub chrome_path: Option<PathBuf>,
    /// Extra command-line arguments.
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    /// Playwright's default.
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy)]
pub struct PageOptions {
    pub viewport: Size,
    /// `deviceScaleFactor`; screenshots are CSS size x this.
    pub device_scale_factor: f64,
    pub color_scheme: ColorScheme,
}

impl PageOptions {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            viewport: Size { width, height },
            device_scale_factor: 1.0,
            color_scheme: ColorScheme::Light,
        }
    }

    pub fn scale(mut self, device_scale_factor: f64) -> Self {
        self.device_scale_factor = device_scale_factor;
        self
    }

    pub fn color_scheme(mut self, color_scheme: ColorScheme) -> Self {
        self.color_scheme = color_scheme;
        self
    }
}

/// When [`Page::goto`] returns (`waitUntil`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitUntil {
    /// Navigation committed.
    Commit,
    DomContentLoaded,
    Load,
}

/// CSS-pixel rectangle in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScreenshotOptions {
    /// Capture the whole scrollable document.
    pub full_page: bool,
    /// Capture only this page-coordinate rectangle (ignored with `full_page`).
    pub clip: Option<BoundingBox>,
    /// Transparent background instead of white.
    pub omit_background: bool,
}

struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct BrowserInner {
    browser: tokio::sync::Mutex<chromiumoxide::Browser>,
    headed: bool,
    alive: Arc<AtomicBool>,
    handler: JoinHandle<()>,
    user_data_dir: PathBuf,
    ws_endpoint: String,
}

impl Drop for BrowserInner {
    fn drop(&mut self) {
        // The child is killed on drop by chromiumoxide; clean up its profile.
        self.handler.abort();
        let _ = std::fs::remove_dir_all(&self.user_data_dir);
    }
}

/// A launched Chrome. Cheap to clone; pages may be used concurrently.
#[derive(Clone)]
pub struct Browser {
    inner: Arc<BrowserInner>,
}

static SHARED: tokio::sync::Mutex<Option<Browser>> = tokio::sync::Mutex::const_new(None);

/// Flags Playwright passes that affect pixels or timing.
const EXTRA_ARGS: [&str; 4] = [
    "--force-color-profile=srgb",
    "--hide-scrollbars",
    "--disable-lcd-text",
    "--font-render-hinting=none",
];

impl Browser {
    /// Launch a new Chrome (not shared).
    pub async fn launch(options: LaunchOptions) -> Result<Browser> {
        let exe = match &options.chrome_path {
            Some(path) => path.clone(),
            None => find_chrome()?,
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        static LAUNCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = LAUNCHES.fetch_add(1, Ordering::SeqCst);
        let user_data_dir = std::env::temp_dir().join(format!(
            "astroshot-chrome-{}-{nanos}-{seq}",
            std::process::id()
        ));

        let mut builder = BrowserConfig::builder()
            .chrome_executable(&exe)
            .user_data_dir(&user_data_dir)
            .headless_mode(if options.headed {
                HeadlessMode::False
            } else {
                HeadlessMode::True
            })
            .viewport(None)
            .args(EXTRA_ARGS)
            .args(options.args.iter().cloned());
        if running_as_root() {
            builder = builder.no_sandbox();
        }
        let config = builder.build().map_err(BrowserError::Launch)?;

        let (browser, mut handler) = chromiumoxide::Browser::launch(config)
            .await
            .map_err(|e| BrowserError::Launch(format!("{e}\nExecutable: {}", exe.display())))?;
        let ws_endpoint = browser.websocket_address().clone();
        let alive = Arc::new(AtomicBool::new(true));
        let guard = AliveGuard(alive.clone());
        let handler = tokio::spawn(async move {
            let _guard = guard;
            while handler.next().await.is_some() {}
        });
        Ok(Browser {
            inner: Arc::new(BrowserInner {
                browser: tokio::sync::Mutex::new(browser),
                headed: options.headed,
                alive,
                handler,
                user_data_dir,
                ws_endpoint,
            }),
        })
    }

    /// The process-wide browser, launched on first use. Relaunches when the
    /// previous one died or `headed` differs (react-shot's `getBrowser`).
    pub async fn shared(headed: bool) -> Result<Browser> {
        let mut slot = SHARED.lock().await;
        if let Some(existing) = slot.as_ref()
            && existing.is_connected()
            && existing.inner.headed == headed
        {
            return Ok(existing.clone());
        }
        if let Some(old) = slot.take() {
            old.close().await.ok();
        }
        let browser = Browser::launch(LaunchOptions {
            headed,
            ..Default::default()
        })
        .await?;
        *slot = Some(browser.clone());
        Ok(browser)
    }

    /// Close the shared browser, if any (`closeSharedBrowser`).
    pub async fn close_shared() -> Result<()> {
        let taken = SHARED.lock().await.take();
        match taken {
            Some(browser) => browser.close().await,
            None => Ok(()),
        }
    }

    /// The browser's CDP websocket endpoint (`ws://127.0.0.1:port/devtools/browser/..`),
    /// for attaching another client such as `playwright-core` in the Node helper.
    pub fn ws_endpoint(&self) -> &str {
        &self.inner.ws_endpoint
    }

    pub fn is_connected(&self) -> bool {
        self.inner.alive.load(Ordering::SeqCst)
    }

    /// Open a page with the given viewport, scale, and color scheme.
    pub async fn new_page(&self, options: PageOptions) -> Result<Page> {
        let page = {
            let browser = self.inner.browser.lock().await;
            browser.new_page("about:blank").await.map_err(cdp)?
        };

        let errors = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        // Subscribe before enabling domains so nothing is missed.
        let mut console = page
            .event_listener::<EventConsoleApiCalled>()
            .await
            .map_err(cdp)?;
        let mut exceptions = page
            .event_listener::<EventExceptionThrown>()
            .await
            .map_err(cdp)?;
        let mut log = page
            .event_listener::<EventEntryAdded>()
            .await
            .map_err(cdp)?;
        page.enable_runtime().await.map_err(cdp)?;
        page.enable_log().await.map_err(cdp)?;

        let sink = errors.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = console.next().await {
                if event.r#type == ConsoleApiCalledType::Error {
                    let text = event
                        .args
                        .iter()
                        .map(|arg| match &arg.value {
                            Some(serde_json::Value::String(s)) => s.clone(),
                            Some(other) => other.to_string(),
                            None => arg.description.clone().unwrap_or_default(),
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    sink.lock().unwrap().push(text);
                }
            }
        }));
        let sink = errors.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = exceptions.next().await {
                let details = &event.exception_details;
                let text = details
                    .exception
                    .as_ref()
                    .and_then(|e| e.description.clone())
                    .map(|d| d.split("\n    at ").next().unwrap_or("").to_string())
                    .unwrap_or_else(|| details.text.clone());
                sink.lock().unwrap().push(text);
            }
        }));
        let sink = errors.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = log.next().await {
                // New headless Chrome requests /favicon.ico; Playwright's headless
                // shell does not, so its 404 is not a page error.
                let favicon = event
                    .entry
                    .url
                    .as_deref()
                    .is_some_and(|url| url.ends_with("/favicon.ico"));
                if event.entry.level == LogEntryLevel::Error && !favicon {
                    sink.lock().unwrap().push(event.entry.text.clone());
                }
            }
        }));

        let result = Page {
            page,
            errors,
            tasks,
            options: Mutex::new(options),
            screencast: Mutex::new(None),
        };
        result.apply_metrics().await?;
        let scheme = match options.color_scheme {
            ColorScheme::Light => "light",
            ColorScheme::Dark => "dark",
        };
        result
            .page
            .execute(SetEmulatedMediaParams {
                media: None,
                features: Some(vec![MediaFeature {
                    name: "prefers-color-scheme".into(),
                    value: scheme.into(),
                }]),
            })
            .await
            .map_err(cdp)?;
        Ok(result)
    }

    /// Close Chrome and delete its profile directory.
    pub async fn close(&self) -> Result<()> {
        let mut browser = self.inner.browser.lock().await;
        let closed = browser.close().await.map_err(cdp);
        let _ = browser.wait().await;
        drop(browser);
        self.inner.alive.store(false, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(&self.inner.user_data_dir);
        closed.map(|_| ())
    }
}

fn running_as_root() -> bool {
    cfg!(target_os = "linux") && std::env::var("USER").is_ok_and(|u| u == "root")
}

/// Playwright's `recordVideo` screencast quality.
const RECORDER_JPEG_QUALITY: u8 = 90;

struct Screencast {
    frames: Arc<Mutex<Vec<ScreencastFrame>>>,
    task: JoinHandle<()>,
    started: Instant,
}

/// One frame from [`Page::stop_screencast`].
#[derive(Debug, Clone)]
pub struct ScreencastFrame {
    /// Encoded image (PNG or JPEG, per the requested format).
    pub data: Vec<u8>,
    /// Milliseconds since [`Page::start_screencast`] when the frame arrived.
    pub at_ms: u128,
    /// Chrome's frame swap time in seconds since the epoch
    /// (`metadata.timestamp`; the arrival wall clock when Chrome omits it).
    pub timestamp: f64,
}

/// What [`Page::finish_screencast`] returns.
#[derive(Debug, Clone)]
pub struct ScreencastRecording {
    pub frames: Vec<ScreencastFrame>,
    /// Milliseconds from [`Page::start_screencast`] to the stop, on the same
    /// clock as [`ScreencastFrame::at_ms`].
    pub stopped_at_ms: u128,
}

impl ScreencastRecording {
    /// Drop each frame whose image is byte-identical to the one before it.
    ///
    /// Chrome re-sends the unchanged surface when something forces a redraw
    /// (`Page.captureScreenshot` does); Playwright's headless shell does not.
    /// Left in, such a frame would count as page activity and move the start
    /// of the recorder's closing hold ([`recorder_repeats`]).
    pub fn drop_unchanged_frames(&mut self) {
        self.frames.dedup_by(|frame, kept| frame.data == kept.data);
    }

    /// Seconds between the last frame's arrival and the stop (the whole
    /// recording when no frame arrived).
    pub fn idle_secs(&self) -> f64 {
        let last = self.frames.last().map_or(0, |frame| frame.at_ms);
        self.stopped_at_ms.saturating_sub(last) as f64 / 1000.0
    }
}

/// A page (tab). Close it with [`Page::close`]; dropping leaves the tab open
/// until the browser closes.
pub struct Page {
    page: chromiumoxide::Page,
    errors: Arc<Mutex<Vec<String>>>,
    tasks: Vec<JoinHandle<()>>,
    options: Mutex<PageOptions>,
    screencast: Mutex<Option<Screencast>>,
}

fn epoch_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

impl Page {
    async fn apply_metrics(&self) -> Result<()> {
        let o = *self.options.lock().unwrap();
        self.page
            .execute(SetDeviceMetricsOverrideParams::new(
                o.viewport.width as i64,
                o.viewport.height as i64,
                o.device_scale_factor,
                false,
            ))
            .await
            .map_err(cdp)?;
        Ok(())
    }

    /// Grow or shrink the browser window so its content area is the viewport,
    /// and return the content size reached (the window has a minimum size, so
    /// a small viewport leaves it larger).
    ///
    /// The screencast captures the window's content area, not the emulated
    /// viewport: a larger viewport is cropped to the window and a smaller one
    /// shows the window around it. Playwright's headless shell has no window;
    /// headed, it sizes the window the same way (`Browser.setWindowBounds`).
    async fn fit_window_to_viewport(&self) -> Result<Size> {
        let viewport = self.viewport();
        let window = self
            .page
            .execute(GetWindowForTargetParams::default())
            .await
            .map_err(cdp)?;
        // Emulation hides the real content size; measure without it, and
        // restore it only once the resize has landed (a resize after the
        // override would put the view back at the window's size).
        self.page
            .execute(ClearDeviceMetricsOverrideParams::default())
            .await
            .map_err(cdp)?;
        let fitted = self.resize_window(&window.result, viewport).await;
        self.apply_metrics().await?;
        fitted
    }

    async fn resize_window(
        &self,
        window: &chromiumoxide::cdp::browser_protocol::browser::GetWindowForTargetReturns,
        viewport: Size,
    ) -> Result<Size> {
        let content = self.window_content_size().await?;
        if content == viewport {
            return Ok(content);
        }
        // The window's own chrome is its bounds minus its content.
        let grow = |outer: Option<i64>, inner: u32, want: u32| {
            let inner = i64::from(inner);
            (outer.unwrap_or(inner) - inner + i64::from(want)).max(1)
        };
        let bounds = Bounds::builder()
            .width(grow(window.bounds.width, content.width, viewport.width))
            .height(grow(window.bounds.height, content.height, viewport.height))
            .build();
        self.page
            .execute(SetWindowBoundsParams::new(window.window_id, bounds))
            .await
            .map_err(cdp)?;
        // The resize reaches the renderer a little later; wait for a content
        // size that covers the viewport (bounded: then record what we have).
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let now = self.window_content_size().await?;
            let changed = now != content;
            let covers = now.width >= viewport.width && now.height >= viewport.height;
            if (changed && covers) || Instant::now() >= deadline {
                return Ok(now);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn window_content_size(&self) -> Result<Size> {
        let inner = self
            .evaluate::<Vec<f64>>("[window.innerWidth, window.innerHeight]")
            .await?;
        match inner[..] {
            [width, height] => Ok(Size {
                width: width as u32,
                height: height as u32,
            }),
            _ => Err(BrowserError::Script("window size is unavailable".into())),
        }
    }

    /// The page's CDP target id, to find it again from another CDP client.
    pub fn target_id(&self) -> String {
        AsRef::<str>::as_ref(self.page.target_id()).to_string()
    }

    pub fn viewport(&self) -> Size {
        self.options.lock().unwrap().viewport
    }

    /// `page.setViewportSize`; keeps the device scale factor.
    pub async fn set_viewport(&self, width: u32, height: u32) -> Result<()> {
        self.options.lock().unwrap().viewport = Size { width, height };
        self.apply_metrics().await
    }

    /// `page.goto`. Errors on a navigation failure or after `timeout`.
    pub async fn goto(&self, url: &str, wait: WaitUntil, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let response = tokio::time::timeout(timeout, self.page.execute(NavigateParams::new(url)))
            .await
            .map_err(|_| BrowserError::Timeout {
                what: format!("navigation to {url}"),
                ms: timeout.as_millis(),
            })?
            .map_err(cdp)?;
        if let Some(reason) = &response.result.error_text {
            return Err(BrowserError::Navigation {
                url: url.to_string(),
                reason: reason.clone(),
            });
        }
        let ready: &[&str] = match wait {
            WaitUntil::Commit => return Ok(()),
            WaitUntil::DomContentLoaded => &["interactive", "complete"],
            WaitUntil::Load => &["complete"],
        };
        let states = serde_json::to_string(ready).unwrap();
        poll(
            deadline,
            &format!("document readyState after navigating to {url}"),
            || async {
                self.evaluate::<bool>(&format!("{states}.includes(document.readyState)"))
                    .await
            },
        )
        .await
    }

    /// `page.setContent(html, {waitUntil: "load"})`.
    pub async fn set_content(&self, html: &str) -> Result<()> {
        self.page.set_content(html).await.map_err(cdp)?;
        poll(
            Instant::now() + DEFAULT_TIMEOUT,
            "document load after setContent",
            || async {
                self.evaluate::<bool>("document.readyState === 'complete'")
                    .await
            },
        )
        .await
    }

    /// Evaluate a JS expression (promises are awaited) and deserialize the
    /// JSON-serializable result. Use an IIFE for statements.
    pub async fn evaluate<T: DeserializeOwned>(&self, expression: &str) -> Result<T> {
        let params = EvaluateParams::builder()
            .expression(expression)
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(BrowserError::Script)?;
        let response = self.page.execute(params).await.map_err(cdp)?.result;
        if let Some(details) = response.exception_details {
            let text = details
                .exception
                .and_then(|e| e.description)
                .unwrap_or(details.text);
            return Err(BrowserError::Script(text));
        }
        let value = response.result.value.unwrap_or(serde_json::Value::Null);
        serde_json::from_value(value).map_err(|e| BrowserError::Script(e.to_string()))
    }

    /// `page.waitForTimeout`.
    pub async fn wait_for_timeout(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    /// `page.waitForFunction(fn)`: polls `expression` until it is truthy.
    pub async fn wait_for_function(&self, expression: &str, timeout: Duration) -> Result<()> {
        let wrapped = format!("Boolean({expression})");
        poll(
            Instant::now() + timeout,
            &format!("function {expression}"),
            || async { self.evaluate::<bool>(&wrapped).await },
        )
        .await
    }

    /// `page.waitForSelector(selector, {state: "visible"})`.
    pub async fn wait_for_selector(&self, selector: &str, timeout: Duration) -> Result<()> {
        let expr = format!(
            "(() => {{ {HELPERS} const e = find({}); return !!e && visible(e); }})()",
            js_str(selector)
        );
        poll(
            Instant::now() + timeout,
            &format!("selector {selector:?} to be visible"),
            || async { self.evaluate::<bool>(&expr).await },
        )
        .await
    }

    /// `page.getByText(text, {exact: false}).first().waitFor({state: "visible"})`.
    pub async fn wait_for_text(&self, text: &str, timeout: Duration) -> Result<()> {
        self.wait_for_selector(&format!("text={text}"), timeout)
            .await
    }

    /// Number of elements matching a CSS selector.
    pub async fn count(&self, selector: &str) -> Result<usize> {
        self.evaluate(&format!(
            "document.querySelectorAll({}).length",
            js_str(selector)
        ))
        .await
    }

    /// `locator(selector).innerText()` for the first match.
    pub async fn inner_text(&self, selector: &str) -> Result<String> {
        let expr = format!(
            "(() => {{ {HELPERS} const e = find({}); return e ? e.innerText : null; }})()",
            js_str(selector)
        );
        self.evaluate::<Option<String>>(&expr)
            .await?
            .ok_or_else(|| BrowserError::NoElement(selector.to_string()))
    }

    /// `locator(selector).first().boundingBox()` in page coordinates; `None`
    /// when nothing matches or the element has no layout box.
    pub async fn bounding_box(&self, selector: &str) -> Result<Option<BoundingBox>> {
        let expr = format!(
            "(() => {{ {HELPERS} const e = find({}); if (!e) return null; const r = e.getBoundingClientRect(); \
             if (r.width === 0 && r.height === 0) return null; \
             return {{x: r.x + scrollX, y: r.y + scrollY, width: r.width, height: r.height}}; }})()",
            js_str(selector)
        );
        self.evaluate(&expr).await
    }

    /// Browser-side errors so far: uncaught exceptions (`pageerror`) and
    /// `console.error`/error log entries, in arrival order.
    pub fn page_errors(&self) -> Vec<String> {
        self.errors.lock().unwrap().clone()
    }

    /// `locator(selector).first().screenshot({omitBackground})` as PNG. Waits
    /// for nothing; call [`Page::wait_for_selector`] first. Errors when the
    /// selector matches nothing.
    pub async fn screenshot_element(
        &self,
        selector: &str,
        omit_background: bool,
    ) -> Result<Vec<u8>> {
        let bbox = self
            .bounding_box(selector)
            .await?
            .ok_or_else(|| BrowserError::NoElement(selector.to_string()))?;
        self.screenshot(ScreenshotOptions {
            clip: Some(bbox),
            omit_background,
            ..Default::default()
        })
        .await
    }

    /// `page.screenshot(...)` as PNG. Output is CSS size x device scale factor.
    pub async fn screenshot(&self, options: ScreenshotOptions) -> Result<Vec<u8>> {
        let mut params = CaptureScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .build();
        let clip = if options.full_page {
            let metrics = self
                .page
                .execute(GetLayoutMetricsParams::default())
                .await
                .map_err(cdp)?;
            let size = &metrics.result.css_content_size;
            Some(BoundingBox {
                x: 0.0,
                y: 0.0,
                width: size.width,
                height: size.height,
            })
        } else {
            options.clip
        };
        if let Some(c) = clip {
            params.clip = Some(CdpClip {
                x: c.x,
                y: c.y,
                width: c.width,
                height: c.height,
                scale: 1.0,
            });
            params.capture_beyond_viewport = Some(true);
        }
        if options.omit_background {
            self.page
                .execute(SetDefaultBackgroundColorOverrideParams {
                    color: Some(Rgba {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: Some(0.0),
                    }),
                })
                .await
                .map_err(cdp)?;
        }
        let shot = self.page.execute(params).await;
        if options.omit_background {
            let _ = self
                .page
                .execute(SetDefaultBackgroundColorOverrideParams { color: None })
                .await;
        }
        let shot = shot.map_err(cdp)?;
        base64::engine::general_purpose::STANDARD
            .decode(AsRef::<str>::as_ref(&shot.result.data))
            .map_err(cdp)
    }

    /// Start collecting `Page.screencastFrame` events (the stand-in for
    /// Playwright's `recordVideo`). Chrome only emits a frame when the page
    /// repaints, so a still page yields few frames; feed the result to
    /// [`resample_frames`] to get a constant-fps sequence.
    pub async fn start_screencast(&self, jpeg_quality: Option<u8>) -> Result<()> {
        self.start_screencast_within(jpeg_quality, self.viewport())
            .await
    }

    /// Start a screencast for [`write_recorder_webm`], the way Playwright
    /// starts `recordVideo`: JPEG frames at quality 90, at the window
    /// content's own size. The window is first fitted to the viewport so the
    /// frames show all of it; where the window cannot shrink that far, the
    /// encoder crops them to the viewport. Stop with
    /// [`Page::finish_screencast`].
    pub async fn start_recording(&self) -> Result<()> {
        if self.screencast.lock().unwrap().is_some() {
            return Err(BrowserError::Screencast("already running".into()));
        }
        let content = self.fit_window_to_viewport().await?;
        self.start_screencast_within(Some(RECORDER_JPEG_QUALITY), content)
            .await
    }

    /// `size` is the largest frame Chrome may send; larger surfaces are
    /// scaled down to fit it.
    async fn start_screencast_within(&self, jpeg_quality: Option<u8>, size: Size) -> Result<()> {
        if self.screencast.lock().unwrap().is_some() {
            return Err(BrowserError::Screencast("already running".into()));
        }
        let mut events = self
            .page
            .event_listener::<EventScreencastFrame>()
            .await
            .map_err(cdp)?;
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let page = self.page.clone();
        let started = Instant::now();
        let task = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if let Ok(data) = base64::engine::general_purpose::STANDARD
                    .decode(AsRef::<str>::as_ref(&event.data))
                {
                    let timestamp = event
                        .metadata
                        .timestamp
                        .as_ref()
                        .map(|time| *time.inner())
                        .filter(|secs| *secs > 0.0)
                        .unwrap_or_else(epoch_secs);
                    sink.lock().unwrap().push(ScreencastFrame {
                        data,
                        at_ms: started.elapsed().as_millis(),
                        timestamp,
                    });
                }
                let _ = page
                    .execute(ScreencastFrameAckParams::new(event.session_id))
                    .await;
            }
        });
        let mut params = StartScreencastParams::builder()
            .max_width(size.width as i64)
            .max_height(size.height as i64)
            .every_nth_frame(1);
        params = match jpeg_quality {
            Some(q) => params.format(StartScreencastFormat::Jpeg).quality(q as i64),
            None => params.format(StartScreencastFormat::Png),
        };
        if let Err(error) = self.page.execute(params.build()).await {
            task.abort();
            return Err(cdp(error));
        }
        // Chrome sends an initial frame on start; wait for it so a still page
        // still yields one (bounded: a hidden page may never paint).
        let first_frame = Instant::now() + Duration::from_secs(2);
        while frames.lock().unwrap().is_empty() && Instant::now() < first_frame {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        *self.screencast.lock().unwrap() = Some(Screencast {
            frames,
            task,
            started,
        });
        Ok(())
    }

    /// Make Chrome send the current surface as a screencast frame.
    ///
    /// A navigation that swaps renderers can paint the new document before
    /// the screencast follows it, and Chrome then sends nothing until the
    /// next repaint: the recording would show the old page meanwhile. A
    /// viewport capture forces a redraw, which the screencast picks up. (A
    /// clipped capture would not do: the frame it produces has the clip's
    /// size.)
    pub async fn refresh_screencast(&self) -> Result<()> {
        let params = CaptureScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Jpeg)
            .quality(1)
            .optimize_for_speed(true)
            .build();
        self.page.execute(params).await.map_err(cdp)?;
        Ok(())
    }

    /// Stop the screencast and return its frames.
    pub async fn stop_screencast(&self) -> Result<Vec<ScreencastFrame>> {
        Ok(self.finish_screencast().await?.frames)
    }

    /// Stop the screencast and return its frames with the stop time, which
    /// [`recorder_repeats`] needs to hold the last frame like Playwright.
    pub async fn finish_screencast(&self) -> Result<ScreencastRecording> {
        let running = self
            .screencast
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| BrowserError::Screencast("not running".into()))?;
        let stopped_at_ms = running.started.elapsed().as_millis();
        let stopped = self.page.execute(StopScreencastParams::default()).await;
        // Let frames already in flight reach the collector.
        tokio::time::sleep(Duration::from_millis(50)).await;
        running.task.abort();
        stopped.map_err(cdp)?;
        let frames = std::mem::take(&mut *running.frames.lock().unwrap());
        Ok(ScreencastRecording {
            frames,
            stopped_at_ms,
        })
    }

    pub async fn close(self) -> Result<()> {
        for task in &self.tasks {
            task.abort();
        }
        if let Some(running) = self.screencast.lock().unwrap().take() {
            running.task.abort();
        }
        self.page.clone().close().await.map_err(cdp)
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Turn irregular screencast frames into a constant-fps sequence covering
/// `duration_ms`: each output slot gets the latest frame at or before it (the
/// first frame fills any lead-in). Empty input gives an empty sequence.
pub fn resample_frames(frames: &[ScreencastFrame], fps: u32, duration_ms: u128) -> Vec<Vec<u8>> {
    if frames.is_empty() || fps == 0 {
        return Vec::new();
    }
    let step = 1000.0 / f64::from(fps);
    let count = ((duration_ms as f64 / step).ceil() as usize).max(1);
    let mut out = Vec::with_capacity(count);
    let mut index = 0;
    for slot in 0..count {
        let at = (slot as f64 * step) as u128;
        while index + 1 < frames.len() && frames[index + 1].at_ms <= at {
            index += 1;
        }
        out.push(frames[index].data.clone());
    }
    out
}

async fn poll<F, Fut>(deadline: Instant, what: &str, mut check: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let start = Instant::now();
    loop {
        // A transient script error (e.g. context destroyed mid-navigation) is
        // retried until the deadline, like Playwright's polling waits.
        let last = match check().await {
            Ok(true) => return Ok(()),
            Ok(false) => None,
            Err(error) => Some(error),
        };
        if Instant::now() >= deadline {
            return Err(match last {
                Some(error) => error,
                None => BrowserError::Timeout {
                    what: what.to_string(),
                    ms: start.elapsed().as_millis(),
                },
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn js_str(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

/// `visible(el)` and `find(selector)` (CSS, or `text=`/`text/` smallest
/// visible element containing the text, case-insensitive).
const HELPERS: &str = r#"
const visible = (el) => {
  const r = el.getBoundingClientRect();
  const s = getComputedStyle(el);
  return r.width > 0 && r.height > 0 && s.visibility !== 'hidden';
};
const find = (selector) => {
  const m = /^text[=\/](.*)$/s.exec(selector);
  if (!m) return document.querySelector(selector);
  const needle = m[1].trim().replace(/\s+/g, ' ').toLowerCase();
  const has = (el) => (el.innerText || el.textContent || '').replace(/\s+/g, ' ').toLowerCase().includes(needle);
  let fallback = null;
  for (const el of document.querySelectorAll('*')) {
    if (el.tagName === 'SCRIPT' || el.tagName === 'STYLE' || !has(el) || [...el.children].some(has)) continue;
    if (visible(el)) return el;
    fallback = fallback || el;
  }
  return fallback;
};
"#;
