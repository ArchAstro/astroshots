//! Port of `packages/astroshot-review/src/terminal/probe.ts`.
//!
//! Detect what the host terminal can draw before the UI takes over
//! stdin/stdout.
//!
//! Sends the kitty graphics query, the cell-size and window-size reports, and a
//! primary device attributes request whose reply always arrives last, so a
//! terminal that ignores the graphics query still terminates the probe.
//!
//! Divergence: Node's `ReadStream`/`WriteStream` pair becomes the
//! [`ProbeStreams`] trait so tests can fake a terminal. [`StdioStreams`] is the
//! real implementation; its reader thread cannot be cancelled, so after a
//! timeout it consumes one further stdin chunk (and drops it).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use regex::Regex;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::kitty::{encode_file_query, encode_query};

/// Process environment as a map (`NodeJS.ProcessEnv`).
pub type Env = HashMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsProtocol {
    Kitty,
    Herdr,
    Halfblocks,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellSource {
    Query,
    Env,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub graphics: GraphicsProtocol,
    /// Whether the terminal accepted a file-path transmission (t=f).
    pub file_medium: bool,
    /// Pixel size of one cell. Falls back to a 1:2 guess when unreported.
    pub cell_width: u32,
    pub cell_height: u32,
    pub cell_source: CellSource,
    /// Why graphics are off, for the Settings pane.
    pub reason: Option<String>,
    pub inside_tmux: bool,
    pub inside_ssh: bool,
    pub inside_mosh: bool,
    pub inside_herdr: bool,
    /// A multiplexer/transport is intercepting output, so pixel graphics can't reach the screen.
    pub intercepted: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

pub const FALLBACK_CELL: CellSize = CellSize {
    width: 10,
    height: 20,
};

/// Whether the terminal can show 24-bit color, which half-block art needs.
pub fn supports_true_color(env: &Env) -> bool {
    let colorterm = env_get(env, "COLORTERM").to_lowercase();
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        return true;
    }
    let term = env_get(env, "TERM").to_lowercase();
    term.contains("256color") || term.contains("kitty") || term.contains("direct")
}

fn env_get<'a>(env: &'a Env, key: &str) -> &'a str {
    env.get(key).map_or("", String::as_str)
}

/// `Boolean(env.A || env.B ...)`: set and non-empty.
fn env_any(env: &Env, keys: &[&str]) -> bool {
    keys.iter().any(|key| !env_get(env, key).is_empty())
}

const QUERY_ID: u32 = 31;
const FILE_QUERY_ID: u32 = 32;

// Smallest valid PNG: 1×1 transparent pixel.
const PROBE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

const DEFAULT_TIMEOUT_MS: u64 = 600;

/// JS `\s` (U+FEFF in JS only, U+0085 in Rust only).
const JS_WS_CLASS: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

/// JS `String#trim`.
fn js_trim(text: &str) -> &str {
    let is_ws = |c: char| {
        matches!(
            c,
            '\u{9}'..='\u{d}'
                | ' '
                | '\u{a0}'
                | '\u{1680}'
                | '\u{2000}'..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
        )
    };
    text.trim_matches(is_ws)
}

static CELL_SIZE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^([0-9]+){JS_WS_CLASS}*[xX×]{JS_WS_CLASS}*([0-9]+)$"
    ))
    .expect("cell size regex")
});
static DEVICE_ATTRIBUTES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[\?[0-9;]*c").expect("DA regex"));
static GRAPHICS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b_G([^\x1b]*)\x1b\\").expect("graphics regex"));
static CELL_REPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[6;([0-9]+);([0-9]+)t").expect("cell report regex"));
static WINDOW_REPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[4;([0-9]+);([0-9]+)t").expect("window report regex"));

/// `Number(digits)` for a `\d+` match; saturates instead of going to a float.
fn number(digits: &str) -> u32 {
    digits.parse().unwrap_or(u32::MAX)
}

pub fn parse_cell_size_env(value: Option<&str>) -> Option<CellSize> {
    let value = value.filter(|v| !v.is_empty())?;
    let captures = CELL_SIZE_RE.captures(js_trim(value))?;
    let width = number(&captures[1]);
    let height = number(&captures[2]);
    if width == 0 || height == 0 {
        return None;
    }
    Some(CellSize { width, height })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeReport {
    pub kitty_ok: bool,
    pub file_ok: bool,
    pub cell_width: Option<u32>,
    pub cell_height: Option<u32>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub saw_device_attributes: bool,
}

/// Interpret the raw bytes a terminal wrote back during the probe.
pub fn parse_probe_response(text: &str) -> ProbeReport {
    let mut report = ProbeReport {
        saw_device_attributes: DEVICE_ATTRIBUTES_RE.is_match(text),
        ..ProbeReport::default()
    };
    for captures in GRAPHICS_RE.captures_iter(text) {
        let body = captures.get(1).map_or("", |m| m.as_str());
        if body.contains(&format!("i={QUERY_ID}")) && body.contains(";OK") {
            report.kitty_ok = true;
        }
        if body.contains(&format!("i={FILE_QUERY_ID}")) && body.contains(";OK") {
            report.file_ok = true;
        }
    }
    if let Some(cell) = CELL_REPORT_RE.captures(text) {
        report.cell_height = Some(number(&cell[1]));
        report.cell_width = Some(number(&cell[2]));
    }
    if let Some(window) = WINDOW_REPORT_RE.captures(text) {
        report.window_height = Some(number(&window[1]));
        report.window_width = Some(number(&window[2]));
    }
    report
}

/// The stdin/stdout pair the probe talks to.
#[allow(async_fn_in_trait)]
pub trait ProbeStreams {
    fn stdin_is_tty(&self) -> bool;
    fn stdout_is_tty(&self) -> bool;
    fn columns(&self) -> Option<u32>;
    fn rows(&self) -> Option<u32>;
    /// Enter raw mode (when stdin is a TTY that was not already raw) and start
    /// buffering input. Runs before the query is written, as the TS does.
    fn begin_read(&mut self);
    fn write_stdout(&mut self, data: &str);
    /// Next chunk of input, or `None` once `deadline` passes or input ends.
    async fn next_chunk(&mut self, deadline: Instant) -> Option<Vec<u8>>;
    /// Restore raw mode and pause input.
    fn end_read(&mut self);
}

#[derive(Debug, Clone, Default)]
pub struct ProbeOptions {
    pub timeout_ms: Option<u64>,
    pub env: Option<Env>,
    pub probe_file_medium: Option<bool>,
}

async fn read_response<S: ProbeStreams>(streams: &mut S, timeout_ms: u64) -> String {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut buffer = String::new();
    while let Some(chunk) = streams.next_chunk(deadline).await {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        // Primary DA reply terminates the probe.
        if DEVICE_ATTRIBUTES_RE.is_match(&buffer) {
            break;
        }
    }
    streams.end_read();
    buffer
}

/// Write a probe PNG into a fresh temp directory, returning its path.
fn write_probe_file() -> Option<PathBuf> {
    let png = STANDARD.decode(PROBE_PNG_BASE64).ok()?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut attempt = 0u32;
    loop {
        let directory = std::env::temp_dir().join(format!(
            "astroshot-review-probe-{:x}{attempt:x}",
            nanos ^ u128::from(std::process::id())
        ));
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                let file = directory.join("probe.png");
                return std::fs::write(&file, png).ok().map(|()| file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 8 => {
                attempt += 1;
            }
            Err(_) => return None,
        }
    }
}

pub async fn probe_terminal<S: ProbeStreams>(
    streams: &mut S,
    options: ProbeOptions,
) -> TerminalCapabilities {
    let env = options
        .env
        .unwrap_or_else(|| std::env::vars_os().filter_map(lossy_pair).collect());
    let timeout_ms = options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let inside_tmux = !env_get(&env, "TMUX").is_empty();
    let inside_ssh = env_any(&env, &["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]);
    let inside_mosh = env_any(
        &env,
        &["MOSH_SERVER_NETWORK_TMOUT", "MOSH_CONNECTION", "MOSH_KEY"],
    );
    let inside_herdr = env_any(&env, &["HERDR_PANE_ID", "HERDR_SOCKET_PATH"]);
    let env_cell = parse_cell_size_env(env.get("ASTROSHOT_REVIEW_CELL_PX").map(String::as_str));
    let forced = env.get("ASTROSHOT_REVIEW_GRAPHICS").map(String::as_str);
    let true_color = supports_true_color(&env);
    // A multiplexer or transport that emulates the terminal itself never forwards
    // another program's pixel escapes: mosh has no image support at all, tmux
    // needs explicit passthrough, and herdr renders only through its own socket.
    let intercepted: Option<String> = if inside_mosh {
        Some("mosh (no image protocol)".to_string())
    } else if inside_herdr {
        Some("herdr".to_string())
    } else if inside_tmux {
        Some("tmux".to_string())
    } else {
        None
    };

    let base = TerminalCapabilities {
        graphics: GraphicsProtocol::None,
        file_medium: false,
        cell_width: env_cell.map_or(FALLBACK_CELL.width, |c| c.width),
        cell_height: env_cell.map_or(FALLBACK_CELL.height, |c| c.height),
        cell_source: if env_cell.is_some() {
            CellSource::Env
        } else {
            CellSource::Fallback
        },
        reason: None,
        inside_tmux,
        inside_ssh,
        inside_mosh,
        inside_herdr,
        intercepted,
    };

    // Colored half-block text renders as ordinary output, so it survives mosh,
    // tmux, and herdr where pixel protocols do not.
    let halfblocks = |reason: &str| -> TerminalCapabilities {
        if true_color {
            TerminalCapabilities {
                graphics: GraphicsProtocol::Halfblocks,
                reason: Some(reason.to_string()),
                ..base.clone()
            }
        } else {
            TerminalCapabilities {
                reason: Some(format!("{reason}; and no truecolor for half-block art")),
                ..base.clone()
            }
        }
    };

    if forced == Some("none") {
        return TerminalCapabilities {
            reason: Some("ASTROSHOT_REVIEW_GRAPHICS=none".to_string()),
            ..base
        };
    }
    if forced == Some("kitty") {
        return TerminalCapabilities {
            graphics: GraphicsProtocol::Kitty,
            file_medium: env_get(&env, "ASTROSHOT_REVIEW_FILE_MEDIUM") == "1",
            ..base
        };
    }
    if matches!(forced, Some("halfblocks" | "half-blocks" | "text")) {
        return halfblocks("ASTROSHOT_REVIEW_GRAPHICS=halfblocks");
    }
    if !streams.stdout_is_tty() || !streams.stdin_is_tty() {
        return TerminalCapabilities {
            reason: Some("stdin/stdout is not a terminal".to_string()),
            ..base
        };
    }
    // Inside an interceptor, don't even probe for kitty (the emulator may answer
    // OK yet never paint); go straight to half-block text.
    if let Some(interceptor) = &base.intercepted {
        return halfblocks(&format!(
            "{interceptor} intercepts pixel graphics; using half-block text"
        ));
    }

    let want_file_probe = options.probe_file_medium.unwrap_or(!inside_ssh);
    let probe_file = if want_file_probe {
        write_probe_file()
    } else {
        None
    };

    let query = format!(
        "{}{}\x1b[16t\x1b[14t\x1b[c",
        encode_query(QUERY_ID),
        probe_file.as_ref().map_or_else(String::new, |file| {
            encode_file_query(FILE_QUERY_ID, &file.to_string_lossy())
        })
    );

    streams.begin_read();
    streams.write_stdout(&query);
    let response = read_response(streams, timeout_ms).await;
    if let Some(directory) = probe_file.as_ref().and_then(|file| file.parent()) {
        let _ = std::fs::remove_dir_all(directory);
    }
    let report = parse_probe_response(&response);

    let mut cell_width = base.cell_width;
    let mut cell_height = base.cell_height;
    let mut cell_source = base.cell_source;
    if env_cell.is_none() {
        let positive = |value: Option<u32>| value.filter(|v| *v > 0);
        if let (Some(width), Some(height)) =
            (positive(report.cell_width), positive(report.cell_height))
        {
            cell_width = width;
            cell_height = height;
            cell_source = CellSource::Query;
        } else if let (Some(window_width), Some(window_height)) = (
            positive(report.window_width),
            positive(report.window_height),
        ) {
            let columns = positive(streams.columns()).unwrap_or(80);
            let rows = positive(streams.rows()).unwrap_or(24);
            cell_width = (window_width / columns).max(1);
            cell_height = (window_height / rows).max(1);
            cell_source = CellSource::Query;
        }
    }

    if !report.kitty_ok {
        let why = if report.saw_device_attributes {
            "no kitty graphics protocol (try Ghostty, kitty, or WezTerm for pixel-perfect images)"
        } else {
            "terminal did not answer the capability probe"
        };
        return TerminalCapabilities {
            cell_width,
            cell_height,
            cell_source,
            ..halfblocks(why)
        };
    }
    TerminalCapabilities {
        graphics: GraphicsProtocol::Kitty,
        file_medium: report.file_ok,
        cell_width,
        cell_height,
        cell_source,
        reason: None,
        inside_tmux,
        inside_ssh,
        inside_mosh,
        inside_herdr,
        intercepted: base.intercepted,
    }
}

fn lossy_pair(pair: (std::ffi::OsString, std::ffi::OsString)) -> Option<(String, String)> {
    Some((
        pair.0.to_string_lossy().into_owned(),
        pair.1.to_string_lossy().into_owned(),
    ))
}

/// The process's real stdin/stdout.
pub struct StdioStreams {
    was_raw: bool,
    entered_raw: bool,
    receiver: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
}

impl StdioStreams {
    pub fn new() -> Self {
        Self {
            was_raw: false,
            entered_raw: false,
            receiver: None,
        }
    }
}

impl Default for StdioStreams {
    fn default() -> Self {
        Self::new()
    }
}

impl ProbeStreams for StdioStreams {
    fn stdin_is_tty(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdin())
    }

    fn stdout_is_tty(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdout())
    }

    fn columns(&self) -> Option<u32> {
        crossterm::terminal::size().ok().map(|(c, _)| u32::from(c))
    }

    fn rows(&self) -> Option<u32> {
        crossterm::terminal::size().ok().map(|(_, r)| u32::from(r))
    }

    fn begin_read(&mut self) {
        self.was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
        if self.stdin_is_tty() && !self.was_raw {
            self.entered_raw = crossterm::terminal::enable_raw_mode().is_ok();
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut chunk = [0u8; 1024];
            while let Ok(read) = stdin.read(&mut chunk) {
                if read == 0 || sender.send(chunk[..read].to_vec()).is_err() {
                    break;
                }
            }
        });
        self.receiver = Some(receiver);
    }

    fn write_stdout(&mut self, data: &str) {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(data.as_bytes());
        let _ = stdout.flush();
    }

    async fn next_chunk(&mut self, deadline: Instant) -> Option<Vec<u8>> {
        let receiver = self.receiver.as_mut()?;
        tokio::time::timeout_at(deadline, receiver.recv())
            .await
            .ok()
            .flatten()
    }

    fn end_read(&mut self) {
        if self.entered_raw {
            let _ = crossterm::terminal::disable_raw_mode();
            self.entered_raw = false;
        }
        self.receiver = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn recognizes_a_kitty_ok_plus_cell_and_window_reports() {
        let report =
            parse_probe_response("\x1b_Gi=31;OK\x1b\\\x1b[6;20;9t\x1b[4;800;1260t\x1b[?62;22c");
        assert!(report.kitty_ok);
        assert!(!report.file_ok);
        assert_eq!(report.cell_width, Some(9));
        assert_eq!(report.cell_height, Some(20));
        assert_eq!(report.window_width, Some(1260));
        assert!(report.saw_device_attributes);
    }

    #[test]
    fn treats_an_error_reply_as_unsupported() {
        let report = parse_probe_response("\x1b_Gi=31;ENOENT:bad\x1b\\\x1b[?1;2c");
        assert!(!report.kitty_ok);
        assert!(report.saw_device_attributes);
    }

    #[test]
    fn notices_the_file_medium_probe_separately() {
        let report = parse_probe_response("\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\\x1b[?62c");
        assert!(report.file_ok);
    }

    #[test]
    fn parses_the_cell_size_override() {
        assert_eq!(
            parse_cell_size_env(Some("10x20")),
            Some(CellSize {
                width: 10,
                height: 20
            })
        );
        assert_eq!(
            parse_cell_size_env(Some("8×16")),
            Some(CellSize {
                width: 8,
                height: 16
            })
        );
        assert_eq!(parse_cell_size_env(Some("nope")), None);
        assert_eq!(parse_cell_size_env(None), None);
    }

    /// A TTY pair that must never be read from: every graphics-mode test
    /// returns before any query is written.
    struct FakeTty;

    impl ProbeStreams for FakeTty {
        fn stdin_is_tty(&self) -> bool {
            true
        }
        fn stdout_is_tty(&self) -> bool {
            true
        }
        fn columns(&self) -> Option<u32> {
            None
        }
        fn rows(&self) -> Option<u32> {
            None
        }
        fn begin_read(&mut self) {
            unreachable!("the probe must not read in this mode");
        }
        fn write_stdout(&mut self, _data: &str) {
            unreachable!("the probe must not write in this mode");
        }
        async fn next_chunk(&mut self, _deadline: Instant) -> Option<Vec<u8>> {
            unreachable!("the probe must not read in this mode");
        }
        fn end_read(&mut self) {}
    }

    #[tokio::test]
    async fn uses_half_blocks_inside_mosh_without_probing_for_kitty() {
        let caps = probe_terminal(
            &mut FakeTty,
            ProbeOptions {
                env: Some(env(&[
                    ("COLORTERM", "truecolor"),
                    ("MOSH_SERVER_NETWORK_TMOUT", "600"),
                ])),
                probe_file_medium: Some(false),
                ..ProbeOptions::default()
            },
        )
        .await;
        assert_eq!(caps.graphics, GraphicsProtocol::Halfblocks);
        assert!(caps.inside_mosh);
        assert!(caps.intercepted.as_deref().unwrap().contains("mosh"));
    }

    #[tokio::test]
    async fn uses_half_blocks_inside_herdr() {
        let caps = probe_terminal(
            &mut FakeTty,
            ProbeOptions {
                env: Some(env(&[
                    ("COLORTERM", "truecolor"),
                    ("HERDR_PANE_ID", "w1:p3"),
                ])),
                ..ProbeOptions::default()
            },
        )
        .await;
        assert_eq!(caps.graphics, GraphicsProtocol::Halfblocks);
        assert!(caps.inside_herdr);
    }

    #[tokio::test]
    async fn honors_the_forced_half_block_mode_and_the_none_mode() {
        let forced = probe_terminal(
            &mut FakeTty,
            ProbeOptions {
                env: Some(env(&[
                    ("COLORTERM", "truecolor"),
                    ("ASTROSHOT_REVIEW_GRAPHICS", "halfblocks"),
                ])),
                ..ProbeOptions::default()
            },
        )
        .await;
        assert_eq!(forced.graphics, GraphicsProtocol::Halfblocks);
        let off = probe_terminal(
            &mut FakeTty,
            ProbeOptions {
                env: Some(env(&[
                    ("COLORTERM", "truecolor"),
                    ("ASTROSHOT_REVIEW_GRAPHICS", "none"),
                ])),
                ..ProbeOptions::default()
            },
        )
        .await;
        assert_eq!(off.graphics, GraphicsProtocol::None);
    }

    #[tokio::test]
    async fn stays_off_without_a_truecolor_terminal() {
        let caps = probe_terminal(
            &mut FakeTty,
            ProbeOptions {
                env: Some(env(&[
                    ("TERM", "vt100"),
                    ("MOSH_SERVER_NETWORK_TMOUT", "600"),
                ])),
                ..ProbeOptions::default()
            },
        )
        .await;
        assert_eq!(caps.graphics, GraphicsProtocol::None);
    }

    #[test]
    fn detects_truecolor_from_colorterm_and_256_color_term() {
        assert!(supports_true_color(&env(&[("COLORTERM", "truecolor")])));
        assert!(supports_true_color(&env(&[("TERM", "xterm-256color")])));
        assert!(!supports_true_color(&env(&[("TERM", "vt100")])));
    }
}
