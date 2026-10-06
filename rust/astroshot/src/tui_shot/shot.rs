//! Port of `packages/tui-shot/src/shot.ts`: render an Ink fixture to a PNG.
//!
//! Divergences from TS:
//! - `loadFixture` and the Ink render run in the Node helper (`ink-render`,
//!   PORTING.md decision 1). The helper validates and resolves `cols`/`rows`;
//!   everything else (defaults, validation, `expectText`, output file) is here.
//! - `captureTerminalHtml` (HTML + Playwright screenshot) is
//!   [`crate::raster::render_ansi_png`]. There is no browser, so
//!   `closeSharedBrowser` only waits for queued shots and `headed` is ignored.
//!   `fontFamily` is validated as a string but the bundled font is used.
//! - The helper is spawned per shot and shut down afterwards.

use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, bail};
use regex::Regex;
use tokio::sync::oneshot;

use super::kitty_graphics::js_number_string;
use super::types::TuiShotRequest;
use crate::node_helper::{InkRenderReply, NodeHelper};
use crate::raster::{RasterOptions, render_ansi_png};

/// Tail of the shot queue: resolves when the most recently queued shot ends.
static SHOT_QUEUE: LazyLock<Mutex<Option<oneshot::Receiver<()>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Port of `closeSharedBrowser`. No browser is shared in Rust; this still
/// waits for every queued shot, like the TS did.
pub fn close_shared_browser() -> impl Future<Output = Result<()>> {
    queue_terminal_shot(|| async { Ok(()) })
}

/// Options for [`valid_positive`].
#[derive(Clone, Copy, Debug)]
pub struct ValidPositiveOptions {
    pub integer: bool,
    pub maximum: f64,
}

pub fn valid_positive(value: f64, name: &str, options: ValidPositiveOptions) -> Result<f64> {
    if !value.is_finite()
        || value <= 0.0
        || value > options.maximum
        || (options.integer && value.fract() != 0.0)
    {
        bail!(
            "{name} must be a positive{} no greater than {}",
            if options.integer { " integer" } else { "" },
            js_number_string(options.maximum)
        );
    }
    Ok(value)
}

/// Port of `queueTerminalShot`: run `task` after every earlier queued task
/// has finished, whether it succeeded or not. The slot in the queue is taken
/// when this function is called, as in TS, not when the future is first
/// polled.
pub fn queue_terminal_shot<T, F, Fut>(task: F) -> impl Future<Output = T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let (done, next) = oneshot::channel();
    let previous = SHOT_QUEUE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .replace(next);
    async move {
        if let Some(previous) = previous {
            // Resolves on completion or on the predecessor being dropped.
            let _ = previous.await;
        }
        let result = task().await;
        // Dropping `done` (here or on cancellation) releases the next shot.
        drop(done);
        result
    }
}

/// Port of `takeTuiShot`.
pub fn take_tui_shot(request: &TuiShotRequest) -> impl Future<Output = Result<String>> {
    let request = request.clone();
    queue_terminal_shot(move || async move { take_isolated_tui_shot(&request).await })
}

fn valid_dimension(value: Option<f64>) -> Option<u32> {
    let value = value?;
    valid_positive(
        value,
        "",
        ValidPositiveOptions {
            integer: true,
            maximum: 1_000.0,
        },
    )
    .ok()
    .map(|value| value as u32)
}

async fn take_isolated_tui_shot(request: &TuiShotRequest) -> Result<String> {
    let mut helper = NodeHelper::spawn().await?;
    // An invalid request size is reported by `render_tui_shot`, after the
    // fixture has loaded (so a missing fixture is reported first, as in TS).
    let rendered = helper
        .ink_render(
            Path::new(&request.fixture_path),
            valid_dimension(request.cols),
            valid_dimension(request.rows),
        )
        .await;
    let _ = helper.shutdown().await;
    let frame = rendered?;
    render_tui_shot(request, &frame)
}

fn finite_in_range(value: f64, name: &str) -> Result<f64> {
    if !value.is_finite() || !(0.0..=1_000.0).contains(&value) {
        bail!("{name} must be between 0 and 1000");
    }
    Ok(value)
}

static OSC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\][^\x07]*(?:\x07|\x1b\\)").expect("valid regex"));
static CSI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").expect("valid regex"));
static ESC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b[@-_]").expect("valid regex"));

/// The frame text with OSC, CSI and two-byte escapes removed.
fn plain_frame(ansi: &str) -> String {
    let text = OSC.replace_all(ansi, "");
    let text = CSI.replace_all(&text, "");
    ESC.replace_all(&text, "").into_owned()
}

/// First `count` UTF-16 code units, like `String#slice(0, count)`.
fn slice_utf16(text: &str, count: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(count).collect();
    String::from_utf16_lossy(&units)
}

/// `path.resolve(outPath)`: absolute, lexically normalized, no filesystem
/// access.
fn resolve_path(path: &str) -> PathBuf {
    let joined = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

/// Port of `renderTuiShot` over an already rendered Ink frame: defaults and
/// validation, the `expectText` check, rasterization, and the PNG write.
/// Returns the absolute output path.
pub fn render_tui_shot(request: &TuiShotRequest, frame: &InkRenderReply) -> Result<String> {
    let cols = request.cols.unwrap_or(f64::from(frame.cols));
    let cols = valid_positive(
        cols,
        "cols",
        ValidPositiveOptions {
            integer: true,
            maximum: 1_000.0,
        },
    )?;
    let rows = request.rows.unwrap_or(f64::from(frame.rows));
    let rows = valid_positive(
        rows,
        "rows",
        ValidPositiveOptions {
            integer: true,
            maximum: 1_000.0,
        },
    )?;
    let scale = valid_positive(
        request.scale.or(frame.scale).unwrap_or(2.0),
        "scale",
        ValidPositiveOptions {
            integer: false,
            maximum: 4.0,
        },
    )?;
    let background = frame.background.as_deref().unwrap_or("#090a12");
    let foreground = frame.foreground.as_deref().unwrap_or("#e8e8f2");
    let font_size = valid_positive(
        frame.font_size.unwrap_or(15.0),
        "fontSize",
        ValidPositiveOptions {
            integer: false,
            maximum: 200.0,
        },
    )?;
    let line_height = valid_positive(
        frame.line_height.unwrap_or(1.32),
        "lineHeight",
        ValidPositiveOptions {
            integer: false,
            maximum: 10.0,
        },
    )?;
    let padding = finite_in_range(frame.padding.unwrap_or(22.0), "padding")?;
    let border_radius = finite_in_range(frame.border_radius.unwrap_or(12.0), "borderRadius")?;

    let plain = plain_frame(&frame.ansi);
    for expected in frame.expect_text.iter().flatten() {
        if !plain.contains(expected.as_str()) {
            bail!(
                "Fixture did not render expected text {}. Visible frame:\n{}",
                serde_json::to_string(expected).expect("string serializes"),
                slice_utf16(&plain, 1_200)
            );
        }
    }

    let mut options =
        RasterOptions::new(cols as u16, rows as u16).with_css_colors(foreground, background)?;
    options.font_size = font_size as f32;
    options.line_height = line_height as f32;
    options.padding = padding as f32;
    options.border_radius = border_radius as f32;
    options.scale = scale as f32;

    let out_path = resolve_path(&request.out_path);
    let is_png = out_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !is_png {
        bail!("Output must use a .png extension: {}", out_path.display());
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let png = render_ansi_png(frame.ansi.as_bytes(), &options)?;
    std::fs::write(&out_path, png)
        .with_context(|| format!("could not write {}", out_path.display()))?;
    Ok(out_path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ansi: &str) -> InkRenderReply {
        InkRenderReply {
            ansi: ansi.to_string(),
            cols: 20,
            rows: 3,
            ..Default::default()
        }
    }

    fn request(out: &Path) -> TuiShotRequest {
        TuiShotRequest {
            fixture_path: "unused.tsx".to_string(),
            out_path: out.display().to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn valid_positive_messages_match_ts() {
        let options = ValidPositiveOptions {
            integer: true,
            maximum: 1_000.0,
        };
        assert_eq!(valid_positive(5.0, "cols", options).unwrap(), 5.0);
        for bad in [0.0, -1.0, 1.5, 1_001.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                valid_positive(bad, "cols", options)
                    .unwrap_err()
                    .to_string(),
                "cols must be a positive integer no greater than 1000"
            );
        }
        let error = valid_positive(
            0.0,
            "scale",
            ValidPositiveOptions {
                integer: false,
                maximum: 4.0,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "scale must be a positive no greater than 4"
        );
    }

    #[test]
    fn renders_defaults_and_writes_the_png_creating_directories() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("nested/deep/shot.png");
        let written = render_tui_shot(&request(&out), &frame("hi\r\n")).unwrap();
        assert_eq!(written, out.to_string_lossy());
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        // cols 20 rows 3, scale 2: ceil(20*15*.62+44)=230, ceil(3*15*1.32+44)=104.
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (460, 208));
    }

    #[test]
    fn rejects_a_missing_expected_text_with_the_visible_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mut reply = frame("\x1b[31mhello\x1b[0m \x1b]0;title\x07world");
        reply.expect_text = Some(vec!["hello world".to_string(), "nope \"x\"".to_string()]);
        let error = render_tui_shot(&request(&dir.path().join("a.png")), &reply).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Fixture did not render expected text \"nope \\\"x\\\"\". Visible frame:\nhello world"
        );
        assert!(!dir.path().join("a.png").exists());
    }

    #[test]
    fn validates_fixture_appearance_fields_and_request_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.png");
        let mut reply = frame("x");
        reply.padding = Some(1_001.0);
        assert_eq!(
            render_tui_shot(&request(&out), &reply)
                .unwrap_err()
                .to_string(),
            "padding must be between 0 and 1000"
        );
        let mut reply = frame("x");
        reply.border_radius = Some(-1.0);
        assert_eq!(
            render_tui_shot(&request(&out), &reply)
                .unwrap_err()
                .to_string(),
            "borderRadius must be between 0 and 1000"
        );
        let mut reply = frame("x");
        reply.font_size = Some(0.0);
        assert_eq!(
            render_tui_shot(&request(&out), &reply)
                .unwrap_err()
                .to_string(),
            "fontSize must be a positive no greater than 200"
        );
        let mut bad = request(&out);
        bad.rows = Some(2.5);
        assert_eq!(
            render_tui_shot(&bad, &frame("x")).unwrap_err().to_string(),
            "rows must be a positive integer no greater than 1000"
        );
        // Request scale wins over the fixture's, which wins over 2.
        let mut reply = frame("x");
        reply.scale = Some(1.0);
        let mut scaled = request(&out);
        scaled.scale = Some(1.0);
        render_tui_shot(&scaled, &reply).unwrap();
        let decoded = image::open(&out).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (230, 104));
    }

    #[test]
    fn requires_a_png_extension_after_rendering_checks() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("shot.jpg");
        let error = render_tui_shot(&request(&out), &frame("x")).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("Output must use a .png extension: {}", out.display())
        );
    }

    #[test]
    fn resolves_relative_output_paths_lexically() {
        let resolved = resolve_path("a/./b/../c.png");
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("a/c.png"));
    }

    #[tokio::test]
    async fn queued_shots_run_one_at_a_time_in_call_order_even_after_a_failure() {
        use std::sync::Arc;
        let log = Arc::new(Mutex::new(Vec::new()));
        let make = |name: &'static str, fail: bool| {
            let log = Arc::clone(&log);
            queue_terminal_shot(move || async move {
                log.lock().unwrap().push(format!("{name} start"));
                tokio::task::yield_now().await;
                log.lock().unwrap().push(format!("{name} end"));
                if fail {
                    Err(anyhow::anyhow!(name))
                } else {
                    Ok(name)
                }
            })
        };
        // Created in order a, b, c; polled in reverse.
        let (a, b, c) = (make("a", true), make("b", false), make("c", false));
        let (rc, rb, ra) = tokio::join!(c, b, a);
        assert_eq!((ra.is_err(), rb.unwrap(), rc.unwrap()), (true, "b", "c"));
        assert_eq!(
            *log.lock().unwrap(),
            ["a start", "a end", "b start", "b end", "c start", "c end"]
        );
    }
}
