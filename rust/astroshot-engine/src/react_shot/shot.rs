//! Port of `packages/react-shot/src/shot.ts`.
//!
//! Rust owns the shot flow and drives Chrome through [`crate::browser`]. The
//! Node helper (`react-serve` / `react-stop`) loads the config, resolves the
//! package root, imports the fixture for its metadata (`readFixtureMeta`, the
//! reply's `nodeMeta`), and serves it with vite.
//!
//! Divergences from TS:
//! - The text after `Fixture page never became ready:` comes from
//!   [`BrowserError`] instead of Playwright's message.
//! - The TS promise queue becomes one async mutex shared by [`take_shot`] and
//!   [`close_shared_browser`].
//! - [`take_shot_detailed`] additionally returns the resolved capture controls.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};
use tokio::sync::Mutex;

use super::batch_paths::resolve;
use super::meta::{CliViewport, ResolvedShotMeta, ShotMeta, resolve_shot_meta};
use super::types::ShotRequest;
use crate::browser::{Browser, BrowserError, Page, PageOptions, ScreenshotOptions, WaitUntil};
use crate::node_helper::NodeHelper;

/// `shotQueue`: shots and browser closes run one at a time, in call order.
static SHOT_QUEUE: Mutex<()> = Mutex::const_new(());

/// A finished shot: where the PNG went and the capture controls it used.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotOutcome {
    pub out_path: PathBuf,
    pub meta: ResolvedShotMeta,
}

/// Chrome could not be started for a shot. The message is neutral; a program
/// that knows how to install a browser appends its own hint.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Could not launch Chromium.\n{detail}")]
pub struct BrowserLaunchError {
    pub detail: String,
}

async fn get_browser(headed: bool) -> Result<Browser> {
    Browser::shared(headed).await.map_err(|error| {
        // `Launch` already reads "Could not launch Chromium.\n<detail>".
        let detail = match error {
            BrowserError::Launch(message) => message,
            other => other.to_string(),
        };
        anyhow::Error::new(BrowserLaunchError { detail })
    })
}

/// `closeSharedBrowser`.
pub async fn close_shared_browser() -> Result<()> {
    let _turn = SHOT_QUEUE.lock().await;
    Browser::close_shared().await?;
    Ok(())
}

/// JS `String(number)` for the whole and fractional values this module prints.
fn js_num(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn dimension_ok(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && (1.0..=10_000.0).contains(&value)
}

/// `[...new Set(items)].slice(0, limit)`.
fn dedupe_first(items: &[String], limit: usize) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for item in items {
        if !seen.contains(&item.as_str()) {
            seen.push(item);
        }
    }
    seen.truncate(limit);
    seen
}

/// `str.slice(0, units)` on UTF-16 code units, without splitting a character.
fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, ch) in text.char_indices() {
        used += ch.len_utf16();
        if used > units {
            return &text[..index];
        }
    }
    text
}

fn is_serious_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    !lower.contains("download the react devtools")
        && !lower.contains("react does not recognize")
        && !lower.contains("[vite]")
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

const BOOT_ERROR_EXPR: &str =
    "(() => { const e = window.__REACT_SHOT_ERROR__; return e == null ? null : String(e); })()";

/// The `stripOverlay` page script; called with the selector as `selector`.
const STRIP_OVERLAY_SCRIPT: &str = r#"(selector) => {
  const target = document.querySelector(selector);
  if (!(target instanceof HTMLElement)) return;
  const clone = target.cloneNode(true);
  document.documentElement.style.cssText = "background:transparent !important;";
  document.body.replaceChildren();
  document.body.style.cssText =
    "margin:0;padding:0;background:transparent !important;display:block;width:fit-content;height:fit-content;";
  for (const [property, value] of [
    ["position", "relative"],
    ["inset", "auto"],
    ["transform", "none"],
    ["margin", "0"],
    ["max-height", "none"],
    ["left", "auto"],
    ["top", "auto"],
    ["box-shadow", "none"],
    ["filter", "none"],
  ]) {
    clone.style.setProperty(property, value, "important");
  }
  clone.setAttribute("data-react-shot-isolated", "true");
  document.body.appendChild(clone);
}"#;

async fn wait_until_ready(page: &Page) -> Result<()> {
    let ready = page
        .wait_for_function(
            "window.__REACT_SHOT_READY__ === true",
            Duration::from_secs(45),
        )
        .await;
    let Err(error) = ready else {
        return Ok(());
    };
    let boot_error = page
        .evaluate::<Option<String>>(BOOT_ERROR_EXPR)
        .await
        .unwrap_or(None)
        .filter(|text| !text.is_empty());
    let body = page.inner_text("body").await.unwrap_or_default();
    let page_errors = page.page_errors();
    let mut lines = vec![format!("Fixture page never became ready: {error}")];
    if let Some(boot_error) = boot_error {
        lines.push(format!("Boot error: {boot_error}"));
    }
    if !page_errors.is_empty() {
        let items: Vec<String> = dedupe_first(&page_errors, 10)
            .into_iter()
            .map(|item| format!("  - {item}"))
            .collect();
        lines.push(format!("Browser errors:\n{}", items.join("\n")));
    }
    if !body.is_empty() {
        lines.push(format!("Rendered body:\n{}", utf16_prefix(&body, 800)));
    }
    bail!("{}", lines.join("\n"))
}

async fn capture(
    page: &Page,
    url: &str,
    node_meta: &ShotMeta,
    request: &ShotRequest,
    out_path: &Path,
) -> Result<ResolvedShotMeta> {
    page.goto(url, WaitUntil::DomContentLoaded, Duration::from_secs(60))
        .await?;
    wait_until_ready(page).await?;

    let boot_error = page
        .evaluate::<Option<String>>(BOOT_ERROR_EXPR)
        .await?
        .filter(|text| !text.is_empty());
    if let Some(boot_error) = boot_error {
        bail!("Fixture threw while mounting:\n{boot_error}");
    }

    let browser_meta: ShotMeta = page.evaluate("window.__REACT_SHOT_META__ ?? {}").await?;
    let meta = resolve_shot_meta(
        node_meta,
        &browser_meta,
        CliViewport {
            width: request.width.map(f64::from),
            height: request.height.map(f64::from),
        },
    );
    let (width, height) = (meta.width, meta.height);
    if !dimension_ok(width) || !dimension_ok(height) {
        bail!(
            "Fixture dimensions must be integers between 1 and 10000; received {}x{}",
            js_num(width),
            js_num(height)
        );
    }
    let (width_px, height_px) = (width as u32, height as u32);

    let viewport = page.viewport();
    if viewport.width != width_px || viewport.height != height_px {
        page.set_viewport(width_px, height_px).await?;
    }

    if let Some(wait_for) = meta.wait_for.as_deref().filter(|w| !w.is_empty()) {
        if wait_for.starts_with("text=") || wait_for.starts_with("text/") {
            page.wait_for_text(&wait_for[5..], Duration::from_secs(15))
                .await?;
        } else {
            page.wait_for_selector(wait_for, Duration::from_secs(15))
                .await?;
        }
    } else {
        page.wait_for_selector(&meta.selector, Duration::from_secs(15))
            .await?;
    }

    if meta.settle_ms > 0.0 {
        page.wait_for_timeout(meta.settle_ms as u64).await;
    }

    let serious: Vec<String> = page
        .page_errors()
        .into_iter()
        .filter(|error| is_serious_error(error))
        .collect();
    if !serious.is_empty() {
        let items: Vec<String> = dedupe_first(&serious, 8)
            .into_iter()
            .map(|error| format!("  - {error}"))
            .collect();
        bail!(
            "Fixture rendered with browser errors:\n{}",
            items.join("\n")
        );
    }

    if meta.strip_overlay {
        if page.count(&meta.selector).await? == 0 {
            bail!(
                "stripOverlay is enabled but {} matched nothing",
                js_string(&meta.selector)
            );
        }
        page.evaluate::<serde_json::Value>(&format!(
            "({STRIP_OVERLAY_SCRIPT})({})",
            js_string(&meta.selector)
        ))
        .await?;
        page.wait_for_timeout(80).await;
    }

    if meta.full_page {
        let png = page
            .screenshot(ScreenshotOptions {
                full_page: true,
                omit_background: meta.omit_background,
                ..Default::default()
            })
            .await?;
        std::fs::write(out_path, png)?;
        return Ok(meta);
    }

    let target = if meta.strip_overlay {
        "[data-react-shot-isolated]"
    } else {
        meta.selector.as_str()
    };
    page.wait_for_selector(target, Duration::from_secs(10))
        .await?;
    let bounding = page.bounding_box(target).await?;
    let Some(bounding) = bounding.filter(|b| b.width >= 2.0 && b.height >= 2.0) else {
        bail!(
            "Screenshot target has an empty bounding box: {}",
            meta.selector
        );
    };
    if meta.strip_overlay && bounding.width >= width * 0.9 && bounding.height >= height * 0.9 {
        bail!(
            "Overlay removal produced a viewport-sized target ({}x{}); check selector {}",
            js_num((bounding.width + 0.5).floor()),
            js_num((bounding.height + 0.5).floor()),
            js_string(&meta.selector)
        );
    }

    let png = page
        .screenshot_element(target, meta.omit_background)
        .await?;
    std::fs::write(out_path, png)?;
    Ok(meta)
}

async fn take_isolated_shot(request: &ShotRequest) -> Result<ShotOutcome> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let fixture_path = resolve(&cwd, &request.fixture_path);
    if !fixture_path.exists() {
        bail!("Fixture not found: {}", fixture_path.display());
    }
    let is_png = Path::new(&request.out_path)
        .extension()
        .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("png"));
    if !is_png {
        bail!("Output must use a .png extension: {}", request.out_path);
    }
    for (name, value) in [("width", request.width), ("height", request.height)] {
        if let Some(value) = value
            && !dimension_ok(f64::from(value))
        {
            bail!("{name} must be an integer between 1 and 10000");
        }
    }

    let out_path = resolve(&cwd, &request.out_path);
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let mut helper = NodeHelper::spawn().await?;
    let served = helper
        .react_serve(
            &fixture_path,
            request.root.as_deref().map(Path::new),
            request.config_path.as_deref().map(Path::new),
        )
        .await;
    let served = match served {
        Ok(served) => served,
        Err(error) => {
            let _ = helper.shutdown().await;
            return Err(error.into());
        }
    };
    let node_meta = served.node_meta.clone();

    let mut page: Option<Page> = None;
    let outcome = async {
        let browser = get_browser(request.headed.unwrap_or(false)).await?;
        let initial_width = request
            .width
            .map(f64::from)
            .or(node_meta.width)
            .unwrap_or(1280.0);
        let initial_height = request
            .height
            .map(f64::from)
            .or(node_meta.height)
            .unwrap_or(800.0);
        let opened = browser
            .new_page(PageOptions::new(initial_width as u32, initial_height as u32).scale(1.0))
            .await?;
        let page = page.insert(opened);
        capture(page, &served.url, &node_meta, request, &out_path).await
    }
    .await;

    // `finally`: close the page, then stop the server, on every path.
    if let Some(page) = page {
        let _ = page.close().await;
    }
    let _ = helper.react_stop(served.server_id).await;
    let _ = helper.shutdown().await;

    Ok(ShotOutcome {
        out_path,
        meta: outcome?,
    })
}

/// `takeShot` plus the resolved capture controls.
pub async fn take_shot_detailed(request: &ShotRequest) -> Result<ShotOutcome> {
    let _turn = SHOT_QUEUE.lock().await;
    take_isolated_shot(request).await
}

/// `takeShot`: returns the absolute PNG path.
pub async fn take_shot(request: &ShotRequest) -> Result<String> {
    Ok(take_shot_detailed(request)
        .await?
        .out_path
        .to_string_lossy()
        .into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_and_limits_like_a_set_slice() {
        let items: Vec<String> = ["a", "b", "a", "c"].map(str::to_string).to_vec();
        assert_eq!(dedupe_first(&items, 2), vec!["a", "b"]);
    }

    #[test]
    fn filters_known_noise_from_browser_errors() {
        assert!(!is_serious_error(
            "Download the React DevTools for a better"
        ));
        assert!(!is_serious_error("Warning: React does not recognize the x"));
        assert!(!is_serious_error("[vite] connecting"));
        assert!(is_serious_error("TypeError: boom"));
    }

    #[test]
    fn slices_by_utf16_units_without_splitting_characters() {
        assert_eq!(utf16_prefix("abcdef", 3), "abc");
        assert_eq!(utf16_prefix("a😀b", 2), "a");
        assert_eq!(utf16_prefix("short", 800), "short");
    }

    #[test]
    fn prints_numbers_like_js() {
        assert_eq!(js_num(800.0), "800");
        assert_eq!(js_num(1.5), "1.5");
    }

    #[tokio::test]
    async fn rejects_bad_requests_before_touching_node_or_chrome() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("f.tsx");
        std::fs::write(&fixture, "export default {}").unwrap();
        let request = |out: &str, width: Option<u32>| ShotRequest {
            fixture_path: fixture.to_string_lossy().into_owned(),
            out_path: out.to_string(),
            width,
            ..ShotRequest::default()
        };

        let missing = ShotRequest {
            fixture_path: dir.path().join("nope.tsx").to_string_lossy().into_owned(),
            out_path: "x.png".into(),
            ..ShotRequest::default()
        };
        let error = take_shot(&missing).await.unwrap_err().to_string();
        assert_eq!(
            error,
            format!(
                "Fixture not found: {}",
                dir.path().join("nope.tsx").display()
            )
        );
        let error = take_shot(&request("x.jpg", None)).await.unwrap_err();
        assert_eq!(error.to_string(), "Output must use a .png extension: x.jpg");
        let error = take_shot(&request("x.png", Some(10_001)))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "width must be an integer between 1 and 10000"
        );
    }
}
