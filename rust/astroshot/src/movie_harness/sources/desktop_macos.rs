//! The `desktop.window` source: record a native macOS window by sampling
//! `screencapture -l <windowId>`, with CoreGraphics lookups done by the
//! shipped Swift helper (`native/macos/WindowTools.swift`).
//!
//! Port of `packages/movie-harness/src/sources/desktop-macos.ts`. Argv, error
//! texts, quality thresholds and the permission-check flow are the TS ones.
//!
//! Deliberate divergences:
//! - `WindowTools.swift` is embedded in the binary (`include_str!`) and
//!   written to a content-addressed temp dir before `swift` interprets it,
//!   instead of resolving `../../native/macos` next to the module. The TS
//!   "WindowTools.swift missing" error therefore cannot occur.
//! - `titleRegex` uses the `regex` crate (no lookaround/backreferences) and an
//!   invalid pattern reports `Invalid regular expression: /<src>/i: <detail>`
//!   with the regex crate's detail text.
//! - JSON parse failures carry serde_json's message instead of V8's.
//!
//! `bin::doctor` re-execs `astroshot movie check-screen-access`; that command
//! prints [`ScreenAccessReport::to_json_pretty`] on stdout (see its docs).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Result, anyhow, bail};
use flate2::read::ZlibDecoder;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::astroshot_review::data::review_store::js_trim;
use crate::movie_harness::session::{
    FrameExtension, MovieSession, StopOptions, now_ms, random_bytes,
};
use crate::movie_harness::types::{
    ManifestStatus, MovieArtifact, MovieFormat, MovieSessionOptions, MovieSourceKind, Size,
    js_number,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopWindowInfo {
    pub id: i64,
    pub pid: i64,
    pub owner: String,
    pub title: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    pub width: i64,
    pub height: i64,
    pub x: i64,
    pub y: i64,
    pub on_screen: bool,
    /// CGWindow layer; 0 = normal, >0 = floating/popover chrome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopWindowPick {
    Largest,
    First,
}

/// Field order is the order `desktopMatchFromFlags` assigns keys, which is the
/// order `JSON.stringify(match)` prints in the "No desktop window matched" error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopWindowMatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub window_id: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub pid: Option<f64>,
    /// When multiple match, pick largest (default) or first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<DesktopWindowPick>,
    /// Prefer on-screen windows when several match (default true).
    /// Off-screen / empty host windows often produce black frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_on_screen: Option<bool>,
}

/// `Omit<MovieSessionOptions, "source"> & { match, durationMs?, cursor?, allowBlank? }`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DesktopWindowMovieOptions {
    pub feature: String,
    pub slug: String,
    pub root: Option<String>,
    pub run_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub size: Option<Size>,
    pub fps: Option<f64>,
    pub format: Option<MovieFormat>,
    pub status: Option<ManifestStatus>,
    pub r#match: DesktopWindowMatch,
    /// How long to sample. Default 3000.
    pub duration_ms: Option<f64>,
    /// Include cursor in frames (screencapture -C). Default false.
    pub cursor: bool,
    /// Allow mostly-blank / black posters to encode (default false).
    pub allow_blank: bool,
}

const SCREENCAPTURE: &str = "/usr/sbin/screencapture";

/// Known deep-links to the Screen Recording privacy pane (varies by macOS).
const SCREEN_RECORDING_SETTINGS_URLS: [&str; 2] = [
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
    "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture",
];

const SECURITY_ROOT_URL: &str = "x-apple.systempreferences:com.apple.preference.security";

const WINDOW_TOOLS_SWIFT: &str =
    include_str!("../../../../../packages/movie-harness/native/macos/WindowTools.swift");

/// Key order is the `JSON.stringify` order of `checkScreenRecordingAccess`'s
/// return value: `granted, requested, hostApp, hostBundleId, enableApp,
/// settingsHint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenAccessReport {
    pub granted: bool,
    pub requested: bool,
    pub host_app: String,
    pub host_bundle_id: Option<String>,
    /// Best-effort name of the app the user should enable (terminal/IDE).
    pub enable_app: String,
    pub settings_hint: String,
}

impl ScreenAccessReport {
    /// `JSON.stringify(report, null, 2)` (no trailing newline). This is what
    /// `astroshot movie check-screen-access` prints to stdout (followed by
    /// `\n`) and what `doctor` parses: `granted` and `enableApp` are the
    /// fields it reads.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("report serializes")
    }
}

/// Options of `checkScreenRecordingAccess`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckScreenAccessOptions {
    /// Call CGRequestScreenCaptureAccess when not already granted.
    pub request: bool,
}

/// Options of `ensureScreenRecordingAccess`; `None` means the TS default
/// (`request` true, `openSettings` true).
#[derive(Debug, Clone, Copy, Default)]
pub struct EnsureScreenAccessOptions {
    pub request: Option<bool>,
    /// Open System Settings when denied (default true).
    pub open_settings: Option<bool>,
}

/// Output of one process run, like `spawnSync` with `encoding: "utf8"`.
struct RunOutput {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

/// `${result.status}`: a number, or `null` when killed by a signal.
fn status_text(status: Option<i32>) -> String {
    status.map_or_else(|| "null".to_string(), |code| code.to_string())
}

fn assert_macos_on(os: &str) -> Result<()> {
    if os != "macos" {
        bail!(
            "desktop.window is only implemented on macOS (CGWindowList + screencapture). \
             On other platforms use --source frames and push your own captures. \
             See: astroshot movie which-source"
        );
    }
    Ok(())
}

fn assert_macos() -> Result<()> {
    assert_macos_on(std::env::consts::OS)
}

fn assert_screencapture() -> Result<()> {
    if !Path::new(SCREENCAPTURE).exists() {
        bail!("screencapture not found at {SCREENCAPTURE}");
    }
    Ok(())
}

/// Write the embedded Swift helper to a content-addressed temp path (once per
/// content) and return it.
fn window_tools_script() -> Result<PathBuf> {
    let digest = Sha256::digest(WINDOW_TOOLS_SWIFT.as_bytes());
    let tag: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    let dir = std::env::temp_dir().join(format!("astroshot-window-tools-{tag}"));
    let script = dir.join("WindowTools.swift");
    if fs::read_to_string(&script).is_ok_and(|text| text == WINDOW_TOOLS_SWIFT) {
        return Ok(script);
    }
    fs::create_dir_all(&dir)?;
    // Write-then-rename so concurrent processes never see a partial file.
    let suffix: String = random_bytes(4).iter().map(|b| format!("{b:02x}")).collect();
    let partial = dir.join(format!("WindowTools.swift.{suffix}.tmp"));
    fs::write(&partial, WINDOW_TOOLS_SWIFT)?;
    fs::rename(&partial, &script)?;
    Ok(script)
}

fn run_window_tools(args: &[&str]) -> Result<RunOutput> {
    let script = window_tools_script()?;
    let output = Command::new("swift").arg(&script).args(args).output();
    match output {
        Ok(output) => Ok(RunOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Err(error) => {
            let message = if error.kind() == std::io::ErrorKind::NotFound {
                "spawnSync swift ENOENT".to_string()
            } else {
                error.to_string()
            };
            bail!(
                "Could not run Swift window tools ({message}). \
                 Install Xcode Command Line Tools (`xcode-select --install`)."
            )
        }
    }
}

/// `spawnSync(cmd, args).status === 0`; a failed spawn is `false`.
fn spawn_succeeds(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.code() == Some(0))
}

/// Open System Settings to the Screen Recording privacy list.
/// Best-effort: URL schemes differ slightly across macOS versions.
pub fn open_screen_recording_settings() -> Result<bool> {
    assert_macos()?;
    for url in SCREEN_RECORDING_SETTINGS_URLS {
        if spawn_succeeds("open", &[url]) {
            return Ok(true);
        }
    }
    // Fallback: open the Privacy & Security root.
    Ok(spawn_succeeds("open", &[SECURITY_ROOT_URL]))
}

fn term_program_name(term: &str) -> Option<&'static str> {
    Some(match term {
        "ghostty" => "Ghostty",
        "iTerm.app" => "iTerm",
        "Apple_Terminal" => "Terminal",
        "vscode" => "Code", // VS Code / Cursor often still set vscode
        "WarpTerminal" => "Warp",
        "WezTerm" => "WezTerm",
        "Alacritty" => "Alacritty",
        _ => return None,
    })
}

/// `resolveEnableAppName` with an injectable environment lookup.
fn resolve_enable_app_name_with(
    env: &dyn Fn(&str) -> Option<String>,
    swift_host_app: Option<&str>,
) -> String {
    if let Some(term) = env("TERM_PROGRAM")
        .map(|value| js_trim(&value).to_string())
        .filter(|value| !value.is_empty())
    {
        return term_program_name(&term).map_or(term, str::to_string);
    }
    let cursor_trace = env("CURSOR_TRACE_ID").is_some_and(|value| !value.is_empty());
    let vscode_pid = env("VSCODE_PID").is_some_and(|value| !value.is_empty());
    if cursor_trace || vscode_pid {
        return if cursor_trace { "Cursor" } else { "Code" }.to_string();
    }
    if let Some(host) = swift_host_app
        && !host.is_empty()
        && !host.to_ascii_lowercase().contains("swift")
    {
        return host.to_string();
    }
    "your terminal or IDE (the app that launched this command)".to_string()
}

/// Name the app the human should toggle in Screen Recording settings.
/// Prefer `$TERM_PROGRAM` (Ghostty, iTerm, vscode...) over the Swift runner process.
pub fn resolve_enable_app_name(swift_host_app: Option<&str>) -> String {
    resolve_enable_app_name_with(&|name| std::env::var(name).ok(), swift_host_app)
}

/// JS truthiness of a parsed JSON value (`Boolean(x)`).
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

/// Build a report from the Swift tool's `screen-access` JSON text.
fn parse_screen_access(text: &str) -> Result<ScreenAccessReport> {
    let parsed: Value = serde_json::from_str(text).map_err(|error| anyhow!("{error}"))?;
    let field = |name: &str| parsed.get(name);
    let host_app = match field("hostApp") {
        Some(Value::String(text)) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    };
    let enable_app = resolve_enable_app_name(host_app);
    Ok(ScreenAccessReport {
        granted: truthy(field("granted")),
        requested: truthy(field("requested")),
        host_app: host_app.unwrap_or("unknown").to_string(),
        host_bundle_id: match field("hostBundleId") {
            Some(Value::String(text)) => Some(text.clone()),
            _ => None,
        },
        settings_hint: format!(
            "System Settings → Privacy & Security → Screen Recording → enable {enable_app}, then quit & reopen it"
        ),
        enable_app,
    })
}

/// Detect Screen Recording TCC (best-effort).
///
/// Uses CoreGraphics preflight via Swift. Note: the Swift process identity may
/// differ from `screencapture`'s responsible app (your terminal). Capture
/// failure remains authoritative; this steers the human to Settings early.
pub fn check_screen_recording_access(
    options: CheckScreenAccessOptions,
) -> Result<ScreenAccessReport> {
    assert_macos()?;
    let mut args = vec!["screen-access"];
    if options.request {
        args.push("--request");
    }
    let result = run_window_tools(&args)?;
    let text = js_trim(&result.stdout);
    if text.is_empty() {
        bail!(
            "screen-access returned no JSON (status {}): {}",
            status_text(result.status),
            result.stderr.chars().take(400).collect::<String>()
        );
    }
    parse_screen_access(text)
}

pub fn format_screen_recording_denied_help(report: Option<&ScreenAccessReport>) -> String {
    let app = report.map_or_else(|| resolve_enable_app_name(None), |r| r.enable_app.clone());
    let lines = [
        "Screen Recording permission is required for --source desktop.window.".to_string(),
        String::new(),
        "Fix:".to_string(),
        "  1. Open System Settings → Privacy & Security → Screen Recording".to_string(),
        "     (or: astroshot movie open-screen-settings)".to_string(),
        format!("  2. Enable \"{app}\""),
        format!("  3. Quit and reopen {app} completely (TCC applies on next launch)"),
        "  4. Re-run: astroshot movie check-screen-access".to_string(),
        String::new(),
        "Note: macOS may not always show an automatic prompt; the Settings toggle is the reliable path."
            .to_string(),
        "browser / pty sources do not need this permission.".to_string(),
    ];
    lines.join("\n")
}

/// Best-effort open of Settings, then the denied-help error.
fn screen_recording_denied(report: Option<&ScreenAccessReport>) -> anyhow::Error {
    // Best-effort: open Settings so the human does not have to hunt.
    let _ = open_screen_recording_settings();
    anyhow!("{}", format_screen_recording_denied_help(report))
}

/// Preflight Screen Recording; optionally request + open Settings on deny.
/// Call before desktop.window capture.
pub fn ensure_screen_recording_access(
    options: EnsureScreenAccessOptions,
) -> Result<ScreenAccessReport> {
    let report = check_screen_recording_access(CheckScreenAccessOptions {
        request: options.request.unwrap_or(true),
    })?;
    if report.granted {
        return Ok(report);
    }
    if options.open_settings != Some(false) {
        let _ = open_screen_recording_settings();
    }
    bail!("{}", format_screen_recording_denied_help(Some(&report)))
}

/// List layer-0 windows as JSON via shipped Swift tool (interpreted by `swift`).
pub fn list_desktop_windows() -> Result<Vec<DesktopWindowInfo>> {
    assert_macos()?;
    let result = run_window_tools(&["list"])?;
    if result.status != Some(0) {
        bail!(
            "Window list failed ({}): {}",
            status_text(result.status),
            result.stderr.chars().take(500).collect::<String>()
        );
    }
    parse_window_list(&result.stdout)
}

fn parse_window_list(stdout: &str) -> Result<Vec<DesktopWindowInfo>> {
    let text = js_trim(stdout);
    if text.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(text).map_err(|error| anyhow!("{error}"))
}

pub fn match_desktop_window(
    windows: &[DesktopWindowInfo],
    r#match: &DesktopWindowMatch,
) -> Result<DesktopWindowInfo> {
    let mut candidates: Vec<&DesktopWindowInfo> = windows.iter().collect();

    if let Some(window_id) = r#match.window_id {
        candidates.retain(|w| w.id as f64 == window_id);
    }
    if let Some(bundle_id) = r#match.bundle_id.as_deref().filter(|v| !v.is_empty()) {
        let want = bundle_id.to_lowercase();
        candidates.retain(|w| w.bundle_id.as_deref().unwrap_or("").to_lowercase() == want);
    }
    if let Some(owner) = r#match.owner.as_deref().filter(|v| !v.is_empty()) {
        let want = owner.to_lowercase();
        candidates.retain(|w| w.owner.to_lowercase().contains(&want));
    }
    if let Some(pid) = r#match.pid {
        candidates.retain(|w| w.pid as f64 == pid);
    }
    if let Some(source) = r#match.title_regex.as_deref().filter(|v| !v.is_empty()) {
        let re = RegexBuilder::new(source)
            .case_insensitive(true)
            .build()
            .map_err(|error| anyhow!("Invalid regular expression: /{source}/i: {error}"))?;
        candidates.retain(|w| re.is_match(&w.title));
    }

    if candidates.is_empty() {
        let sample = windows
            .iter()
            .take(8)
            .map(|w| {
                format!(
                    "  id={} pid={} layer={} bundle={} owner={} title={} {}x{} onScreen={}",
                    w.id,
                    w.pid,
                    w.layer.map_or_else(|| "?".to_string(), |l| l.to_string()),
                    w.bundle_id.as_deref().unwrap_or("?"),
                    json_string(&w.owner),
                    json_string(&w.title),
                    w.width,
                    w.height,
                    w.on_screen
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "No desktop window matched {}.\nRun: astroshot movie list-windows\nSample windows:\n{}",
            serde_json::to_string(r#match).expect("match serializes"),
            if sample.is_empty() {
                "  (none)"
            } else {
                &sample
            }
        );
    }

    let prefer_on_screen = r#match.prefer_on_screen != Some(false);
    if prefer_on_screen && r#match.window_id.is_none() {
        let on_screen: Vec<&DesktopWindowInfo> =
            candidates.iter().copied().filter(|w| w.on_screen).collect();
        if !on_screen.is_empty() {
            candidates = on_screen;
        }
    }

    if r#match.pick == Some(DesktopWindowPick::First) {
        return Ok(candidates[0].clone());
    }
    // Default: largest area, then lower layer (normal windows over floaters).
    candidates.sort_by(|a, b| {
        let area_a = a.width * a.height;
        let area_b = b.width * b.height;
        area_b
            .cmp(&area_a)
            .then(a.layer.unwrap_or(0).cmp(&b.layer.unwrap_or(0)))
    });
    Ok(candidates[0].clone())
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("string serializes")
}

/// Human-readable manifest description for a captured window.
pub fn describe_desktop_window(target: &DesktopWindowInfo) -> String {
    let bundle_name = target
        .bundle_id
        .as_deref()
        .and_then(|id| id.split('.').next_back())
        .filter(|name| !name.is_empty());
    let name = bundle_name
        .or(Some(target.owner.as_str()).filter(|owner| !owner.is_empty()))
        .unwrap_or("window");
    let title = js_trim(&target.title);
    let size = format!("{}×{}", target.width, target.height);
    let location = if target.on_screen {
        "on-screen"
    } else {
        "off-screen"
    };
    let layer = match target.layer {
        Some(layer) if layer > 0 => format!(", layer {layer}"),
        _ => String::new(),
    };
    if !title.is_empty() {
        return format!("{name}: “{title}” ({size}, {location}{layer})");
    }
    format!("{name} window ({size}, {location}{layer})")
}

/// Thresholds of [`is_nearly_blank_png`]; `None` means the TS default
/// (`maxMeanLuma` 12, `minDarkFraction` 0.97).
#[derive(Debug, Clone, Copy, Default)]
pub struct BlankPngOptions {
    pub max_mean_luma: Option<f64>,
    pub min_dark_fraction: Option<f64>,
}

/// True when a PNG is almost entirely very dark (typical failed/off-screen capture).
/// Samples up to ~4k pixels across a simple grid; supports 8-bit RGB/RGBA.
pub fn is_nearly_blank_png(file_path: &str, options: Option<BlankPngOptions>) -> bool {
    let options = options.unwrap_or_default();
    let max_mean_luma = options.max_mean_luma.unwrap_or(12.0);
    let min_dark_fraction = options.min_dark_fraction.unwrap_or(0.97);
    let Ok(data) = fs::read(file_path) else {
        return true;
    };
    if data.len() < 33 || &data[1..4] != b"PNG" {
        return true;
    }

    // Minimal PNG scan: find IDAT chunks, inflate, average luma on a grid.
    let be32 = |at: usize| u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    let width = be32(16) as usize;
    let height = be32(20) as usize;
    let bit_depth = data[24];
    let color_type = data[25];
    if width == 0 || height == 0 || bit_depth != 8 {
        return false;
    }
    // 2 = RGB, 6 = RGBA
    if color_type != 2 && color_type != 6 {
        return false;
    }
    let channels = if color_type == 6 { 4 } else { 3 };

    let mut idat: Vec<u8> = Vec::new();
    let mut found_idat = false;
    let mut offset = 8usize;
    while offset + 8 <= data.len() {
        let len = be32(offset) as usize;
        let kind = &data[offset + 4..offset + 8];
        let start = offset + 8;
        let end = start + len;
        if end + 4 > data.len() {
            break;
        }
        if kind == b"IDAT" {
            idat.extend_from_slice(&data[start..end]);
            found_idat = true;
        }
        if kind == b"IEND" {
            break;
        }
        offset = end + 4;
    }
    if !found_idat {
        return true;
    }

    let mut inflated = Vec::new();
    if ZlibDecoder::new(idat.as_slice())
        .read_to_end(&mut inflated)
        .is_err()
    {
        return false; // can't decode — don't claim blank
    }

    let stride = 1 + width * channels; // filter byte + row
    let expected = stride * height;
    if inflated.len() < expected {
        return false;
    }

    // Only sample rows that use filter type 0 (None) for correct RGB bytes.
    // For filtered rows, still sample raw bytes as a coarse darkness heuristic.
    let mut samples = 0u64;
    let mut dark = 0u64;
    let mut luma_sum = 0.0f64;
    let step_y = (height / 32).max(1);
    let step_x = (width / 32).max(1);
    let byte = |at: usize| f64::from(inflated.get(at).copied().unwrap_or(0));
    let mut y = 0;
    while y < height {
        let row_start = y * stride;
        let mut x = 0;
        while x < width {
            let i = row_start + 1 + x * channels;
            let (r, g, b) = (byte(i), byte(i + 1), byte(i + 2));
            // filter≠0 means bytes aren't raw RGB; still treat very low triples as dark.
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            luma_sum += luma;
            if luma <= max_mean_luma {
                dark += 1;
            }
            samples += 1;
            x += step_x;
        }
        y += step_y;
    }
    if samples == 0 {
        return true;
    }
    let mean = luma_sum / samples as f64;
    let dark_fraction = dark as f64 / samples as f64;
    mean <= max_mean_luma && dark_fraction >= min_dark_fraction
}

/// `screencapture` argv for one window frame.
fn screencapture_args(window_id: i64, out_path: &str, cursor: bool) -> Vec<String> {
    let id = window_id.to_string();
    let mut args: Vec<&str> = vec!["-x"];
    if cursor {
        args.push("-C");
    }
    args.extend(["-o", "-t", "png", "-l", &id, out_path]);
    args.into_iter().map(str::to_string).collect()
}

/// `checkScreenRecordingAccess({request: false})` swallowing errors.
fn probe_access_quietly() -> Option<ScreenAccessReport> {
    check_screen_recording_access(CheckScreenAccessOptions { request: false }).ok()
}

fn capture_window_png(window_id: i64, out_path: &str, cursor: bool) -> Result<()> {
    let final_args = screencapture_args(window_id, out_path, cursor);
    let (status, stdout, stderr) = match Command::new(SCREENCAPTURE).args(&final_args).output() {
        Ok(output) => (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(_) => (None, String::new(), String::new()),
    };
    if status != Some(0) {
        let access = probe_access_quietly();
        if let Some(report) = access.as_ref().filter(|r| !r.granted) {
            return Err(screen_recording_denied(Some(report)));
        }
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "unknown error".to_string()
        };
        bail!(
            "screencapture failed for window {window_id} ({}): {detail}.\n{}",
            status_text(status),
            format_screen_recording_denied_help(access.as_ref())
        );
    }
    if fs::metadata(out_path).map_or(true, |meta| meta.len() < 32) {
        let access = probe_access_quietly();
        if let Some(report) = access.as_ref().filter(|r| !r.granted) {
            return Err(screen_recording_denied(Some(report)));
        }
        bail!(
            "screencapture produced an empty image for window {window_id}. \
             The window may have closed, or Screen Recording is denied.\n{}",
            format_screen_recording_denied_help(access.as_ref())
        );
    }
    Ok(())
}

/// Width/height from a PNG's IHDR, or `None` when it is not a PNG.
fn read_png_size(file_path: &str) -> Result<Option<Size>> {
    let mut file = fs::File::open(file_path)?;
    let mut buf = [0u8; 24];
    let mut filled = 0;
    while filled < buf.len() {
        let read = file.read(&mut buf[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    if &buf[1..4] != b"PNG" {
        return Ok(None);
    }
    Ok(Some(Size {
        width: u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]),
        height: u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]),
    }))
}

/// `fs.mkdtempSync(path.join(os.tmpdir(), prefix))`.
fn mkdtemp(prefix: &str) -> Result<PathBuf> {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let base = std::env::temp_dir();
    for _ in 0..100 {
        let suffix: String = random_bytes(6)
            .iter()
            .map(|byte| CHARS[*byte as usize % CHARS.len()] as char)
            .collect();
        let dir = base.join(format!("{prefix}{suffix}"));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!(
        "could not create a temporary directory under {}",
        base.display()
    )
}

/// Removes the temp directory on every exit path (`rmSync({recursive, force})`).
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Sample a macOS window at `fps` for `durationMs`, encode to movie + poster.
/// Uses OS `screencapture` (already on every Mac) — no separate download.
pub async fn record_desktop_window_movie(
    options: DesktopWindowMovieOptions,
) -> Result<MovieArtifact> {
    assert_macos()?;
    assert_screencapture()?;

    let duration_ms = options.duration_ms.unwrap_or(3_000.0);
    let fps = options.fps.unwrap_or(10.0);
    if duration_ms.is_nan() || duration_ms <= 0.0 {
        bail!("--duration-ms must be positive");
    }

    // Detect TCC up front (may prompt once; opens Settings if still denied).
    ensure_screen_recording_access(EnsureScreenAccessOptions {
        request: Some(true),
        open_settings: Some(true),
    })?;

    let windows = list_desktop_windows()?;
    let target = match_desktop_window(&windows, &options.r#match)?;

    if !target.on_screen && !options.allow_blank {
        bail!(
            "Matched window id={} ({}) is off-screen ({}×{} at {},{}). \
             Off-screen windows usually produce black movies. Bring the window on-screen \
             (open the tray/popover) or pass --allow-blank to record anyway.\n\
             Hint: {}",
            target.id,
            target.bundle_id.as_deref().unwrap_or(target.owner.as_str()),
            target.width,
            target.height,
            target.x,
            target.y,
            describe_desktop_window(&target)
        );
    }

    let tmp = mkdtemp("astroshot-desktop-")?;
    let _cleanup = TempDirGuard(tmp.clone());
    let probe = path_str(&tmp.join("probe.png"));
    capture_window_png(target.id, &probe, options.cursor)?;

    if !options.allow_blank && is_nearly_blank_png(&probe, None) {
        bail!(
            "Probe frame for window id={} is nearly blank/black. \
             Screen Recording may be denied for this host app, the window may be empty, \
             or the popover may not be visible. Fix visibility/TCC, or pass --allow-blank.\n\
             Window: {}\n\
             Run: astroshot movie check-screen-access",
            target.id,
            describe_desktop_window(&target)
        );
    }

    let probed = read_png_size(&probe)?;
    let size = options.size.or(probed).unwrap_or(Size {
        width: target.width.max(0) as u32,
        height: target.height.max(0) as u32,
    });

    let mut session = MovieSession::create(MovieSessionOptions {
        feature: options.feature.clone(),
        slug: options.slug.clone(),
        root: options.root.clone(),
        run_id: options.run_id.clone(),
        title: options.title.clone(),
        description: Some(
            options
                .description
                .clone()
                .unwrap_or_else(|| describe_desktop_window(&target)),
        ),
        size: Some(size),
        fps: Some(fps),
        format: options.format,
        status: options.status,
        source: MovieSourceKind::DesktopWindow,
    })?;

    // Seed with probe frame so we never end empty if duration is tiny.
    session.push_frame(&fs::read(&probe)?, FrameExtension::Png)?;

    let interval_ms = (1000.0 / fps).round().max(50.0) as u64;
    let deadline = now_ms() as f64 + duration_ms;
    let mut index = 1u32;

    while (now_ms() as f64) < deadline {
        let frame_path = path_str(&tmp.join(format!("f-{index:05}.png")));
        let started = now_ms();
        let target_id = target.id;
        let cursor = options.cursor;
        let capture = {
            let frame_path = frame_path.clone();
            tokio::task::spawn_blocking(move || capture_window_png(target_id, &frame_path, cursor))
                .await?
        };
        let pushed = capture.and_then(|()| session.push_frame_file(&frame_path).map(|_| ()));
        if let Err(error) = pushed {
            // Window may close mid-recording; stop cleanly if we have frames.
            if !session.list_frames()?.is_empty() {
                break;
            }
            return Err(error);
        }
        index += 1;
        let elapsed = now_ms().saturating_sub(started);
        let sleep = interval_ms.saturating_sub(elapsed);
        if sleep > 0 && ((now_ms() + sleep) as f64) < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(sleep)).await;
        }
    }

    session
        .stop(StopOptions {
            status: Some(options.status.unwrap_or(ManifestStatus::Running)),
            ..StopOptions::default()
        })
        .await
}

/// CLI-style string flags of `desktopMatchFromFlags` (`--window-id` etc.).
/// A flag counts as set only when it is a non-empty string.
#[derive(Debug, Clone, Default)]
pub struct DesktopMatchFlags {
    pub window_id: Option<String>,
    pub bundle_id: Option<String>,
    pub title_regex: Option<String>,
    pub owner: Option<String>,
    pub pid: Option<String>,
    pub pick: Option<String>,
}

static JS_DECIMAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[+-]?(?:[0-9]+\.?[0-9]*(?:[eE][+-]?[0-9]+)?|\.[0-9]+(?:[eE][+-]?[0-9]+)?)$")
        .expect("static regex")
});

/// JS `Number(text)` for strings: NaN when not a numeric literal.
fn js_number_from_str(text: &str) -> f64 {
    let text = js_trim(text);
    if text.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if text.len() > 2 && text[..2].eq_ignore_ascii_case(prefix) {
            return u64::from_str_radix(&text[2..], radix).map_or(f64::NAN, |n| n as f64);
        }
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if JS_DECIMAL.is_match(text) {
        text.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.is_empty())
}

/// Resolve match flags from CLI-style strings.
pub fn desktop_match_from_flags(flags: &DesktopMatchFlags) -> Result<DesktopWindowMatch> {
    let mut found = DesktopWindowMatch::default();
    if let Some(value) = non_empty(&flags.window_id) {
        found.window_id = Some(js_number_from_str(value));
    }
    if let Some(value) = non_empty(&flags.bundle_id) {
        found.bundle_id = Some(value.to_string());
    }
    if let Some(value) = non_empty(&flags.title_regex) {
        found.title_regex = Some(value.to_string());
    }
    if let Some(value) = non_empty(&flags.owner) {
        found.owner = Some(value.to_string());
    }
    if let Some(value) = non_empty(&flags.pid) {
        found.pid = Some(js_number_from_str(value));
    }
    match flags.pick.as_deref() {
        Some("first") => found.pick = Some(DesktopWindowPick::First),
        Some("largest") => found.pick = Some(DesktopWindowPick::Largest),
        _ => {}
    }
    if found.window_id.is_none()
        && found.bundle_id.is_none()
        && found.title_regex.is_none()
        && found.owner.is_none()
        && found.pid.is_none()
    {
        bail!(
            "desktop.window requires one of --window-id, --bundle-id, --title-regex, --owner, or --pid. \
             Run: astroshot movie list-windows"
        );
    }
    if found.window_id.is_some_and(f64::is_nan) {
        bail!("--window-id must be a number");
    }
    if found.pid.is_some_and(f64::is_nan) {
        bail!("--pid must be a number");
    }
    Ok(found)
}

/// Ensure Swift is runnable (for clearer errors at CLI start).
pub fn assert_desktop_toolchain() -> Result<()> {
    assert_macos()?;
    let ok = Command::new("swift")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !ok {
        bail!(
            "desktop.window requires the Swift toolchain (`swift` on PATH). \
             Install Xcode Command Line Tools: xcode-select --install"
        );
    }
    assert_screencapture()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movie_harness::png::encode_solid_png;

    fn window(id: i64, on_screen: bool, width: i64, height: i64, layer: i64) -> DesktopWindowInfo {
        DesktopWindowInfo {
            id,
            pid: 1,
            owner: "Astroshots".to_string(),
            title: String::new(),
            bundle_id: Some("ai.archastro.Astroshots".to_string()),
            width,
            height,
            x: 0,
            y: 0,
            on_screen,
            layer: Some(layer),
        }
    }

    // ---- desktop-quality.test.ts ("desktop capture quality helpers") ----

    #[test]
    fn describes_windows_for_humans_not_raw_flag_dumps() {
        let text = describe_desktop_window(&DesktopWindowInfo {
            id: 12,
            pid: 1,
            owner: "Astroshots".to_string(),
            title: String::new(),
            bundle_id: Some("ai.archastro.Astroshots".to_string()),
            width: 400,
            height: 640,
            x: 10,
            y: 20,
            on_screen: true,
            layer: Some(0),
        });
        assert!(text.contains("Astroshots"));
        assert!(text.contains("400×640"));
        assert!(text.contains("on-screen"));
        assert!(!text.contains("desktop.window id="));
        assert_eq!(text, "Astroshots window (400×640, on-screen)");
    }

    #[test]
    fn detects_nearly_blank_black_pngs() {
        let dir = std::env::temp_dir().join(format!(
            "blank-png-{}",
            random_bytes(4)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ));
        fs::create_dir_all(&dir).unwrap();
        let black = dir.join("black.png");
        let color = dir.join("color.png");
        fs::write(&black, encode_solid_png(64, 48, [0, 0, 0])).unwrap();
        fs::write(&color, encode_solid_png(64, 48, [120, 50, 160])).unwrap();
        let black_blank = is_nearly_blank_png(black.to_str().unwrap(), None);
        let color_blank = is_nearly_blank_png(color.to_str().unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
        assert!(black_blank);
        assert!(!color_blank);
    }

    #[test]
    fn prefers_on_screen_windows_when_matching_by_bundle_id() {
        let mut off = window(1, false, 1000, 1000, 0);
        off.title = String::new();
        let mut tray = window(2, true, 400, 640, 5);
        tray.title = "Tray".to_string();
        tray.x = 10;
        tray.y = 40;
        let matched = match_desktop_window(
            &[off, tray],
            &DesktopWindowMatch {
                bundle_id: Some("ai.archastro.Astroshots".to_string()),
                ..DesktopWindowMatch::default()
            },
        )
        .unwrap();
        assert_eq!(matched.id, 2);
    }

    // ---- desktop-macos.test.ts (cases that need no real capture) ----

    #[cfg(target_os = "macos")]
    #[test]
    fn reports_screen_recording_tcc_preflight_as_json_shaped_status() {
        let report =
            check_screen_recording_access(CheckScreenAccessOptions { request: false }).unwrap();
        assert!(!report.enable_app.is_empty());
        let help = format_screen_recording_denied_help(Some(&report));
        assert!(help.contains("Screen Recording"));
        assert!(help.contains("open-screen-settings"));
        assert!(help.contains(&report.enable_app));
        let json: Value = serde_json::from_str(&report.to_json_pretty()).unwrap();
        assert!(json["granted"].is_boolean());
    }

    // Listing windows needs a logged-in GUI session (CGWindowList).
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a macOS GUI session with visible windows and the swift toolchain"]
    fn lists_windows_with_ids() {
        let windows = list_desktop_windows().unwrap();
        assert!(!windows.is_empty());
        assert!(windows[0].width > 0);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "needs Screen Recording permission for the host app and a visible window"]
    async fn matches_by_window_id_and_records_a_short_movie() {
        // Prefer a normal on-screen app window (layer 0), not floating system chrome.
        let windows: Vec<DesktopWindowInfo> = list_desktop_windows()
            .unwrap()
            .into_iter()
            .filter(|w| {
                w.width >= 200
                    && w.height >= 200
                    && w.on_screen
                    && w.layer.unwrap_or(0) == 0
                    && w.owner != "Dock"
                    && w.owner != "Window Server"
            })
            .collect();
        assert!(!windows.is_empty());
        let target = &windows[0];
        let matched = match_desktop_window(
            &windows,
            &DesktopWindowMatch {
                window_id: Some(target.id as f64),
                ..DesktopWindowMatch::default()
            },
        )
        .unwrap();
        assert_eq!(matched.id, target.id);

        let root = mkdtemp("desktop-movie-").unwrap();
        let result = record_desktop_window_movie(DesktopWindowMovieOptions {
            feature: "desktop-smoke".to_string(),
            slug: "window".to_string(),
            root: Some(path_str(&root)),
            r#match: DesktopWindowMatch {
                window_id: Some(target.id as f64),
                ..DesktopWindowMatch::default()
            },
            duration_ms: Some(400.0),
            fps: Some(5.0),
            status: Some(ManifestStatus::Pass),
            ..DesktopWindowMovieOptions::default()
        })
        .await;
        let checks = result.map(|artifact| {
            assert!(Path::new(&artifact.poster_path).exists());
            assert!(Path::new(&artifact.video_path).exists());
            assert_eq!(artifact.source, MovieSourceKind::DesktopWindow);
            let manifest: Value = serde_json::from_str(
                &fs::read_to_string(root.join(".astroshot/desktop-smoke/manifest.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(manifest["shots"][0]["source"], "desktop.window");
            assert_eq!(manifest["shots"][0]["kind"], "movie");
            // Description should be human-readable, not a raw flag dump.
            let description = manifest["shots"][0]["description"].as_str().unwrap_or("");
            assert!(!description.contains("desktop.window id="));
        });
        let _ = fs::remove_dir_all(&root);
        checks.unwrap();
    }

    // ---- extra coverage for the pure logic ----

    #[test]
    fn screencapture_argv_matches_ts_with_and_without_cursor() {
        assert_eq!(
            screencapture_args(42, "/tmp/a.png", false),
            ["-x", "-o", "-t", "png", "-l", "42", "/tmp/a.png"]
        );
        assert_eq!(
            screencapture_args(42, "/tmp/a.png", true),
            ["-x", "-C", "-o", "-t", "png", "-l", "42", "/tmp/a.png"]
        );
    }

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn enable_app_name_prefers_term_program_then_ide_then_swift_host() {
        let name = |pairs: &[(&str, &str)], host: Option<&str>| {
            resolve_enable_app_name_with(&env_of(pairs), host)
        };
        assert_eq!(name(&[("TERM_PROGRAM", "ghostty")], None), "Ghostty");
        assert_eq!(name(&[("TERM_PROGRAM", "iTerm.app")], None), "iTerm");
        assert_eq!(
            name(&[("TERM_PROGRAM", "Apple_Terminal")], None),
            "Terminal"
        );
        assert_eq!(name(&[("TERM_PROGRAM", "vscode")], None), "Code");
        assert_eq!(name(&[("TERM_PROGRAM", "WarpTerminal")], None), "Warp");
        assert_eq!(name(&[("TERM_PROGRAM", "WezTerm")], None), "WezTerm");
        assert_eq!(name(&[("TERM_PROGRAM", "Alacritty")], None), "Alacritty");
        assert_eq!(name(&[("TERM_PROGRAM", "  kitty \n")], None), "kitty");
        // Blank TERM_PROGRAM falls through.
        assert_eq!(
            name(&[("TERM_PROGRAM", "  "), ("VSCODE_PID", "9")], None),
            "Code"
        );
        assert_eq!(
            name(&[("CURSOR_TRACE_ID", "x"), ("VSCODE_PID", "9")], None),
            "Cursor"
        );
        assert_eq!(name(&[], Some("Terminal")), "Terminal");
        let fallback = "your terminal or IDE (the app that launched this command)";
        assert_eq!(name(&[], Some("swift-frontend")), fallback);
        assert_eq!(name(&[], Some("SwiftRunner")), fallback);
        assert_eq!(name(&[], Some("")), fallback);
        assert_eq!(name(&[], None), fallback);
    }

    #[test]
    fn denied_help_text_is_the_ts_message() {
        let report = ScreenAccessReport {
            granted: false,
            requested: false,
            host_app: "Ghostty".to_string(),
            host_bundle_id: None,
            enable_app: "Ghostty".to_string(),
            settings_hint: String::new(),
        };
        assert_eq!(
            format_screen_recording_denied_help(Some(&report)),
            [
                "Screen Recording permission is required for --source desktop.window.",
                "",
                "Fix:",
                "  1. Open System Settings → Privacy & Security → Screen Recording",
                "     (or: astroshot movie open-screen-settings)",
                "  2. Enable \"Ghostty\"",
                "  3. Quit and reopen Ghostty completely (TCC applies on next launch)",
                "  4. Re-run: astroshot movie check-screen-access",
                "",
                "Note: macOS may not always show an automatic prompt; the Settings toggle is the reliable path.",
                "browser / pty sources do not need this permission.",
            ]
            .join("\n")
        );
    }

    #[test]
    fn parses_screen_access_json_into_the_doctor_contract() {
        let report = parse_screen_access(
            r#"{"granted":true,"requested":false,"hostApp":"Ghostty","hostBundleId":"com.mitchellh.ghostty","settingsHint":"x"}"#,
        )
        .unwrap();
        assert!(report.granted);
        assert!(!report.requested);
        assert_eq!(report.host_app, "Ghostty");
        assert_eq!(
            report.host_bundle_id.as_deref(),
            Some("com.mitchellh.ghostty")
        );
        assert!(
            report
                .settings_hint
                .starts_with("System Settings → Privacy & Security → Screen Recording → enable ")
        );
        assert!(report.settings_hint.ends_with(", then quit & reopen it"));
        let json: Value = serde_json::from_str(&report.to_json_pretty()).unwrap();
        assert_eq!(json["granted"], true);
        assert_eq!(json["enableApp"], report.enable_app);
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "granted",
                "requested",
                "hostApp",
                "hostBundleId",
                "enableApp",
                "settingsHint"
            ]
        );

        // Missing / empty fields: hostApp -> "unknown", bundle -> null, falsy -> false.
        let sparse = parse_screen_access(r#"{"granted":0,"hostApp":""}"#).unwrap();
        assert!(!sparse.granted);
        assert!(!sparse.requested);
        assert_eq!(sparse.host_app, "unknown");
        assert_eq!(sparse.host_bundle_id, None);
        assert!(parse_screen_access("nope").is_err());
    }

    #[test]
    fn non_macos_platforms_get_the_frames_hint() {
        assert!(assert_macos_on("macos").is_ok());
        let error = assert_macos_on("linux").unwrap_err().to_string();
        assert_eq!(
            error,
            "desktop.window is only implemented on macOS (CGWindowList + screencapture). \
             On other platforms use --source frames and push your own captures. \
             See: astroshot movie which-source"
        );
    }

    #[test]
    fn window_list_parses_with_optional_fields() {
        assert!(parse_window_list("  \n").unwrap().is_empty());
        let windows = parse_window_list(
            r#"[{"id":7,"pid":3,"owner":"Finder","title":"Docs","width":800,"height":600,"x":1,"y":2,"onScreen":true,"layer":0},
                {"id":8,"pid":3,"owner":"Finder","title":"","bundleId":null,"width":1,"height":1,"x":0,"y":0,"onScreen":false}]"#,
        )
        .unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].bundle_id, None);
        assert_eq!(windows[0].layer, Some(0));
        assert_eq!(windows[1].layer, None);
    }

    #[test]
    fn match_filters_by_every_field() {
        let mut a = window(1, true, 100, 100, 0);
        a.owner = "Google Chrome".to_string();
        a.title = "Inbox - Mail".to_string();
        a.bundle_id = Some("com.google.Chrome".to_string());
        a.pid = 10;
        let mut b = window(2, true, 200, 200, 0);
        b.owner = "Safari".to_string();
        b.title = "Docs".to_string();
        b.bundle_id = None;
        b.pid = 11;
        let all = [a, b];
        let one = |m: DesktopWindowMatch| match_desktop_window(&all, &m).map(|w| w.id);
        assert_eq!(
            one(DesktopWindowMatch {
                window_id: Some(1.0),
                ..Default::default()
            })
            .unwrap(),
            1
        );
        assert_eq!(
            one(DesktopWindowMatch {
                bundle_id: Some("COM.GOOGLE.CHROME".into()),
                ..Default::default()
            })
            .unwrap(),
            1
        );
        assert!(
            one(DesktopWindowMatch {
                owner: Some("zzz".into()),
                ..Default::default()
            })
            .is_err()
        );
        assert_eq!(
            one(DesktopWindowMatch {
                owner: Some("SAFARI".into()),
                ..Default::default()
            })
            .unwrap(),
            2
        );
        assert_eq!(
            one(DesktopWindowMatch {
                pid: Some(10.0),
                ..Default::default()
            })
            .unwrap(),
            1
        );
        assert_eq!(
            one(DesktopWindowMatch {
                title_regex: Some("^inbox".into()),
                ..Default::default()
            })
            .unwrap(),
            1
        );
        // Default pick is largest; "first" keeps list order.
        assert_eq!(one(DesktopWindowMatch::default()).unwrap(), 2);
        assert_eq!(
            one(DesktopWindowMatch {
                pick: Some(DesktopWindowPick::First),
                ..Default::default()
            })
            .unwrap(),
            1
        );
    }

    #[test]
    fn match_picks_largest_then_lower_layer_and_honors_prefer_on_screen() {
        let big_off = window(1, false, 1000, 1000, 0);
        let small_on = window(2, true, 100, 100, 0);
        let tie_high_layer = window(3, true, 100, 100, 3);
        let m = |windows: &[DesktopWindowInfo], prefer: Option<bool>, pick| {
            match_desktop_window(
                windows,
                &DesktopWindowMatch {
                    bundle_id: Some("ai.archastro.Astroshots".into()),
                    prefer_on_screen: prefer,
                    pick,
                    ..Default::default()
                },
            )
            .unwrap()
            .id
        };
        let windows = [big_off.clone(), tie_high_layer.clone(), small_on.clone()];
        assert_eq!(m(&windows, None, None), 2); // on-screen, tie -> lower layer
        assert_eq!(m(&windows, Some(false), None), 1); // largest overall
        assert_eq!(m(&windows, Some(true), Some(DesktopWindowPick::First)), 3);
        // Explicit window id skips the on-screen preference.
        let by_id = match_desktop_window(
            &windows,
            &DesktopWindowMatch {
                window_id: Some(1.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_id.id, 1);
        // Nothing on screen: keep the off-screen candidates.
        assert_eq!(m(&[big_off], None, None), 1);
    }

    #[test]
    fn match_failure_lists_up_to_eight_sample_windows() {
        let none = match_desktop_window(
            &[],
            &DesktopWindowMatch {
                owner: Some("x".into()),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            none,
            "No desktop window matched {\"owner\":\"x\"}.\nRun: astroshot movie list-windows\nSample windows:\n  (none)"
        );
        let mut w = window(5, true, 10, 20, 2);
        w.title = "a \"quoted\" title".to_string();
        let error = match_desktop_window(
            &[w, window(6, false, 1, 1, 0)],
            &DesktopWindowMatch {
                window_id: Some(99.0),
                pick: Some(DesktopWindowPick::First),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "No desktop window matched {\"windowId\":99,\"pick\":\"first\"}.\n\
             Run: astroshot movie list-windows\n\
             Sample windows:\n  \
             id=5 pid=1 layer=2 bundle=ai.archastro.Astroshots owner=\"Astroshots\" title=\"a \\\"quoted\\\" title\" 10x20 onScreen=true\n  \
             id=6 pid=1 layer=0 bundle=ai.archastro.Astroshots owner=\"Astroshots\" title=\"\" 1x1 onScreen=false"
        );
        let many: Vec<_> = (0..12).map(|i| window(i, true, 1, 1, 0)).collect();
        let error = match_desktop_window(
            &many,
            &DesktopWindowMatch {
                owner: Some("nobody".into()),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error.matches("\n  id=").count(), 8);
        // Layer unknown prints "?"; bundle unknown prints "?".
        let mut unknown = window(1, true, 1, 1, 0);
        unknown.layer = None;
        unknown.bundle_id = None;
        let error = match_desktop_window(
            &[unknown],
            &DesktopWindowMatch {
                owner: Some("nobody".into()),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("layer=? bundle=? owner"));
    }

    #[test]
    fn invalid_title_regex_is_reported() {
        let error = match_desktop_window(
            &[window(1, true, 1, 1, 0)],
            &DesktopWindowMatch {
                title_regex: Some("(".into()),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.starts_with("Invalid regular expression: /(/i: "));
    }

    #[test]
    fn describe_covers_title_layer_and_name_fallbacks() {
        let mut w = window(1, false, 10, 20, 3);
        w.title = "  Inbox \n".to_string();
        assert_eq!(
            describe_desktop_window(&w),
            "Astroshots: “Inbox” (10×20, off-screen, layer 3)"
        );
        w.title = String::new();
        w.bundle_id = None;
        w.owner = "Finder".to_string();
        w.layer = Some(0);
        assert_eq!(
            describe_desktop_window(&w),
            "Finder window (10×20, off-screen)"
        );
        w.owner = String::new();
        w.layer = None;
        assert_eq!(
            describe_desktop_window(&w),
            "window window (10×20, off-screen)"
        );
        w.bundle_id = Some("".to_string());
        assert_eq!(
            describe_desktop_window(&w),
            "window window (10×20, off-screen)"
        );
    }

    // ---- isNearlyBlankPng edge cases ----

    fn temp_file(name: &str, bytes: &[u8]) -> (PathBuf, String) {
        let dir = mkdtemp("blank-edge-").unwrap();
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        let text = path_str(&path);
        (dir, text)
    }

    #[test]
    fn blank_png_edge_cases_match_ts() {
        let missing = std::env::temp_dir().join("astroshot-no-such-file.png");
        assert!(is_nearly_blank_png(missing.to_str().unwrap(), None));
        let (dir, short) = temp_file("short.png", b"\x89PNG");
        assert!(is_nearly_blank_png(&short, None));
        let (dir2, not_png) = temp_file("x.png", &[0u8; 64]);
        assert!(is_nearly_blank_png(&not_png, None));

        // Valid header but corrupt IDAT: cannot decode, so not claimed blank.
        let mut png = encode_solid_png(4, 4, [0, 0, 0]);
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        for byte in &mut png[idat + 4..idat + 8] {
            *byte = 0xff;
        }
        let (dir3, corrupt) = temp_file("c.png", &png);
        assert!(!is_nearly_blank_png(&corrupt, None));

        // Unsupported color type (palette = 3) is never claimed blank.
        let mut png = encode_solid_png(4, 4, [0, 0, 0]);
        png[25] = 3;
        let (dir4, palette) = temp_file("p.png", &png);
        assert!(!is_nearly_blank_png(&palette, None));

        // 16-bit depth likewise.
        let mut png = encode_solid_png(4, 4, [0, 0, 0]);
        png[24] = 16;
        let (dir5, deep) = temp_file("d.png", &png);
        assert!(!is_nearly_blank_png(&deep, None));

        // No IDAT chunk at all: blank.
        let mut png = encode_solid_png(4, 4, [0, 0, 0]);
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        png[idat..idat + 4].copy_from_slice(b"tEXt");
        let (dir6, no_idat) = temp_file("n.png", &png);
        assert!(is_nearly_blank_png(&no_idat, None));

        // Thresholds are overridable: a mid-gray image is blank at maxMeanLuma 200.
        let (dir7, gray) = temp_file("g.png", &encode_solid_png(64, 64, [100, 100, 100]));
        assert!(!is_nearly_blank_png(&gray, None));
        assert!(is_nearly_blank_png(
            &gray,
            Some(BlankPngOptions {
                max_mean_luma: Some(200.0),
                min_dark_fraction: Some(0.5)
            })
        ));
        for dir in [dir, dir2, dir3, dir4, dir5, dir6, dir7] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn png_size_reads_ihdr_and_rejects_non_png() {
        let (dir, png) = temp_file("a.png", &encode_solid_png(64, 48, [1, 2, 3]));
        assert_eq!(
            read_png_size(&png).unwrap(),
            Some(Size {
                width: 64,
                height: 48
            })
        );
        let (dir2, junk) = temp_file("b.png", b"hello");
        assert_eq!(read_png_size(&junk).unwrap(), None);
        assert!(read_png_size("/nonexistent/astroshot.png").is_err());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir2);
    }

    // ---- flags ----

    fn flags(pairs: &[(&str, &str)]) -> DesktopMatchFlags {
        let mut out = DesktopMatchFlags::default();
        for (key, value) in pairs {
            let slot = match *key {
                "window-id" => &mut out.window_id,
                "bundle-id" => &mut out.bundle_id,
                "title-regex" => &mut out.title_regex,
                "owner" => &mut out.owner,
                "pid" => &mut out.pid,
                _ => &mut out.pick,
            };
            *slot = Some(value.to_string());
        }
        out
    }

    #[test]
    fn match_from_flags_converts_numbers_and_pick() {
        let m = desktop_match_from_flags(&flags(&[
            ("window-id", "42"),
            ("pid", " 7 "),
            ("bundle-id", "a.b"),
            ("title-regex", "t"),
            ("owner", "o"),
            ("pick", "first"),
        ]))
        .unwrap();
        assert_eq!(m.window_id, Some(42.0));
        assert_eq!(m.pid, Some(7.0));
        assert_eq!(m.bundle_id.as_deref(), Some("a.b"));
        assert_eq!(m.title_regex.as_deref(), Some("t"));
        assert_eq!(m.owner.as_deref(), Some("o"));
        assert_eq!(m.pick, Some(DesktopWindowPick::First));
        // JSON.stringify key order is assignment order.
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"windowId":42,"bundleId":"a.b","titleRegex":"t","owner":"o","pid":7,"pick":"first"}"#
        );
        let largest =
            desktop_match_from_flags(&flags(&[("owner", "o"), ("pick", "largest")])).unwrap();
        assert_eq!(largest.pick, Some(DesktopWindowPick::Largest));
        let ignored =
            desktop_match_from_flags(&flags(&[("owner", "o"), ("pick", "weird")])).unwrap();
        assert_eq!(ignored.pick, None);
        // Hex follows JS Number().
        let hex = desktop_match_from_flags(&flags(&[("window-id", "0x10")])).unwrap();
        assert_eq!(hex.window_id, Some(16.0));
        // pid 0 and window id 0 count as set.
        assert!(desktop_match_from_flags(&flags(&[("pid", "0")])).is_ok());
    }

    #[test]
    fn match_from_flags_requires_a_selector_and_numeric_ids() {
        let none = desktop_match_from_flags(&flags(&[("pick", "first")])).unwrap_err();
        assert_eq!(
            none.to_string(),
            "desktop.window requires one of --window-id, --bundle-id, --title-regex, --owner, or --pid. \
             Run: astroshot movie list-windows"
        );
        // Empty strings are falsy in JS.
        assert!(desktop_match_from_flags(&flags(&[("owner", "")])).is_err());
        assert_eq!(
            desktop_match_from_flags(&flags(&[("window-id", "abc")]))
                .unwrap_err()
                .to_string(),
            "--window-id must be a number"
        );
        assert_eq!(
            desktop_match_from_flags(&flags(&[("pid", "1x")]))
                .unwrap_err()
                .to_string(),
            "--pid must be a number"
        );
    }

    #[test]
    fn js_number_parsing_follows_number_semantics() {
        assert_eq!(js_number_from_str("12"), 12.0);
        assert_eq!(js_number_from_str(" -3.5e1 "), -35.0);
        assert_eq!(js_number_from_str(".5"), 0.5);
        assert_eq!(js_number_from_str("5."), 5.0);
        assert_eq!(js_number_from_str("0b101"), 5.0);
        assert_eq!(js_number_from_str("0o17"), 15.0);
        assert_eq!(js_number_from_str("0XfF"), 255.0);
        assert_eq!(js_number_from_str("Infinity"), f64::INFINITY);
        assert_eq!(js_number_from_str("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(js_number_from_str("   "), 0.0);
        for bad in ["inf", "NaN", "1_0", "1,0", "0x", "e5", "--1", "12px"] {
            assert!(js_number_from_str(bad).is_nan(), "{bad}");
        }
    }

    #[test]
    fn embedded_window_tools_script_is_materialized_once() {
        let first = window_tools_script().unwrap();
        let text = fs::read_to_string(&first).unwrap();
        assert!(text.contains("screen-access"));
        assert_eq!(window_tools_script().unwrap(), first);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn desktop_entry_points_refuse_to_run_off_macos() {
        let message = "desktop.window is only implemented on macOS";
        assert!(
            open_screen_recording_settings()
                .unwrap_err()
                .to_string()
                .starts_with(message)
        );
        assert!(
            check_screen_recording_access(CheckScreenAccessOptions::default())
                .unwrap_err()
                .to_string()
                .starts_with(message)
        );
        assert!(
            list_desktop_windows()
                .unwrap_err()
                .to_string()
                .starts_with(message)
        );
        assert!(
            assert_desktop_toolchain()
                .unwrap_err()
                .to_string()
                .starts_with(message)
        );
    }

    #[tokio::test]
    async fn record_validates_platform_before_anything_else() {
        let result = record_desktop_window_movie(DesktopWindowMovieOptions {
            feature: "f".into(),
            slug: "s".into(),
            duration_ms: Some(0.0),
            ..Default::default()
        })
        .await;
        let error = result.unwrap_err().to_string();
        if cfg!(target_os = "macos") {
            assert_eq!(error, "--duration-ms must be positive");
        } else {
            assert!(error.starts_with("desktop.window is only implemented on macOS"));
        }
    }
}
