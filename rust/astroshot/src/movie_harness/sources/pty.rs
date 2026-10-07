//! The "pty" movie source: run a PTY fixture, sample terminal frames into a
//! [`MovieSession`], publish poster + video.
//!
//! Port of `packages/movie-harness/src/sources/pty.ts`.
//!
//! - `node-pty` becomes `portable-pty`; the master is read on a blocking
//!   thread that feeds a shared [`HeadlessTerminal`].
//! - Chromium paint (SGR -> xterm cells -> HTML -> screenshot) becomes
//!   [`crate::raster`]: terminal cells -> glyphs -> PNG. The fixture's
//!   `fontFamily` is accepted but ignored (the font is bundled).
//! - The TS `setInterval` sampler runs on a dedicated thread that owns the
//!   session while the program runs; its thread-local rasterizer keeps the
//!   glyph cache.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;

use crate::movie_harness::session::{FrameExtension, MovieSession, StopOptions};
use crate::movie_harness::types::{
    ManifestStatus, MovieArtifact, MovieSessionOptions, MovieSourceKind, PtyAction, PtyKey,
    PtyMovieFixture, Size,
};
use crate::raster::{HeadlessTerminal, RasterOptions, encode_png, render_rgba};
use crate::tui_shot::pty_shot::keystroke as tui_keystroke;
use crate::tui_shot::types::PtyKey as TuiPtyKey;

const DEFAULT_FONT_SIZE: f64 = 14.0;
const DEFAULT_LINE_HEIGHT: f64 = 1.35;
const DEFAULT_PADDING: f64 = 16.0;
const DEFAULT_BORDER_RADIUS: f64 = 12.0;
const DEFAULT_SCALE: f64 = 2.0;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn delay(ms: f64) {
    tokio::time::sleep(Duration::from_millis(ms.max(0.0) as u64)).await;
}

/// `path.resolve(base, path)`: absolute, lexically normalized.
fn resolve_path(base: &Path, path: &str) -> PathBuf {
    let joined = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        base.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

fn resolve_cwd_relative(path: &str) -> PathBuf {
    resolve_path(&std::env::current_dir().unwrap_or_default(), path)
}

/// First `count` UTF-16 code units, like `String#slice(0, count)`.
fn slice_utf16(text: &str, count: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(count).collect();
    String::from_utf16_lossy(&units)
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("string serializes")
}

/// `loadPtyMovieFixture`.
pub fn load_pty_movie_fixture(fixture_path: &str) -> Result<PtyMovieFixture> {
    let absolute = resolve_cwd_relative(fixture_path);
    let shown = absolute.to_string_lossy().into_owned();
    if !absolute.exists() {
        bail!("PTY fixture not found: {shown}");
    }
    let text = fs::read_to_string(&absolute)?;
    let value: Value = if shown.ends_with(".json") {
        serde_json::from_str(&text)?
    } else {
        serde_yaml_ng::from_str(&text)?
    };
    let Some(record) = value.as_object() else {
        bail!("Invalid PTY fixture {shown}: document must be object");
    };
    if record.get("version").and_then(Value::as_f64) != Some(1.0) {
        bail!("Invalid PTY fixture {shown}: version must be 1");
    }
    match record.get("command") {
        Some(Value::String(command)) if !command.trim().is_empty() => {}
        _ => bail!("Invalid PTY fixture {shown}: command required"),
    }
    serde_json::from_value(value).map_err(|error| anyhow!("Invalid PTY fixture {shown}: {error}"))
}

/// `Omit<MovieSessionOptions, "source"> & { fixturePath }`: `session.source`
/// is overwritten with `pty`.
#[derive(Debug, Clone)]
pub struct PtyMovieSessionOptions {
    pub session: MovieSessionOptions,
    pub fixture_path: String,
}

/// Paint settings shared by the sampler and the final frame.
struct Paint {
    raster: RasterOptions,
}

impl Paint {
    fn size(&self) -> Size {
        let (width, height) = self.raster.css_size();
        Size {
            width: width + 32,
            height: height + 32,
        }
    }

    /// Render with the calling thread's shared rasterizer, so glyphs stay
    /// cached across frames on that thread.
    fn png(&self, terminal: &HeadlessTerminal) -> Result<Vec<u8>> {
        let frame = terminal.frame(self.raster.foreground_rgb(), self.raster.background_rgb());
        let image = render_rgba(&frame, &self.raster)?;
        Ok(encode_png(&image)?)
    }
}

/// Run a PTY fixture, sample truecolor terminal frames into a MovieSession,
/// and publish poster + video. Color path: SGR -> terminal cells -> raster.
pub async fn record_pty_movie(options: PtyMovieSessionOptions) -> Result<MovieArtifact> {
    let fixture = load_pty_movie_fixture(&options.fixture_path)?;
    let fixture_dir = resolve_cwd_relative(&options.fixture_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let cols = fixture.cols.unwrap_or(80) as u16;
    let rows = fixture.rows.unwrap_or(24) as u16;
    let background = fixture.background.as_deref().unwrap_or("#090a12");
    let foreground = fixture.foreground.as_deref().unwrap_or("#e8e8f2");
    let fps = fixture.movie_fps.or(options.session.fps).unwrap_or(12.0);
    let timeout_ms = fixture.timeout_ms.unwrap_or(30_000) as f64;
    let settle_ms = fixture.settle_ms.unwrap_or(80) as f64;

    let mut raster = RasterOptions::movie(cols, rows).with_css_colors(foreground, background)?;
    raster.font_size = fixture.font_size.unwrap_or(DEFAULT_FONT_SIZE);
    raster.line_height = fixture.line_height.unwrap_or(DEFAULT_LINE_HEIGHT);
    raster.padding = fixture.padding.unwrap_or(DEFAULT_PADDING);
    raster.border_radius = fixture.border_radius.unwrap_or(DEFAULT_BORDER_RADIUS);
    raster.scale = fixture.scale.unwrap_or(DEFAULT_SCALE);
    let paint = Arc::new(Paint { raster });
    let size = paint.size();

    let mut session_options = options.session;
    session_options.size = Some(size);
    session_options.fps = Some(fps);
    session_options.source = MovieSourceKind::Pty;
    let status = session_options.status.unwrap_or(ManifestStatus::Running);
    let session = MovieSession::create(session_options)?;

    let terminal = Arc::new(Mutex::new(HeadlessTerminal::new(cols, rows)));
    let interval = Duration::from_millis((1000.0 / fps).round().max(40.0) as u64);

    // Sampler: one frame per interval while the program runs. Interval
    // errors are swallowed (`.catch(() => undefined)`).
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (terminal, paint, stop) =
            (Arc::clone(&terminal), Arc::clone(&paint), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut session = session;
            let mut next = Instant::now() + interval;
            while !stop.load(Ordering::Acquire) {
                let now = Instant::now();
                if now < next {
                    std::thread::sleep((next - now).min(Duration::from_millis(10)));
                    continue;
                }
                next += interval;
                let png = paint.png(&lock(&terminal));
                if let Ok(png) = png {
                    let _ = session.push_frame(&png, FrameExtension::Png);
                }
            }
            session
        })
    };

    let program = run_pty_program(ProgramArgs {
        fixture: &fixture,
        fixture_dir: &fixture_dir,
        terminal: &terminal,
        cols,
        rows,
        timeout_ms,
        settle_ms,
    })
    .await;

    stop.store(true, Ordering::Release);
    let mut session = sampler
        .join()
        .map_err(|_| anyhow!("PTY movie sampler thread panicked"))?;
    program?;

    // Final frame after settle: must match the still-shot color path.
    let png = paint.png(&lock(&terminal))?;
    session.push_frame(&png, FrameExtension::Png)?;
    session
        .stop(StopOptions {
            status: Some(status),
            ..StopOptions::default()
        })
        .await
}

struct ProgramArgs<'a> {
    fixture: &'a PtyMovieFixture,
    fixture_dir: &'a Path,
    terminal: &'a Arc<Mutex<HeadlessTerminal>>,
    cols: u16,
    rows: u16,
    timeout_ms: f64,
    settle_ms: f64,
}

/// Child exit state shared with the waiter thread.
#[derive(Default)]
struct Exit {
    code: Mutex<Option<u32>>,
    /// The reader thread hit EOF: every byte the child wrote is in the terminal.
    reader_done: AtomicBool,
}

async fn run_pty_program(args: ProgramArgs<'_>) -> Result<()> {
    let ProgramArgs {
        fixture,
        fixture_dir,
        terminal,
        cols,
        rows,
        timeout_ms,
        settle_ms,
    } = args;

    let cwd = match &fixture.cwd {
        Some(cwd) => resolve_path(fixture_dir, cwd),
        None => fixture_dir.to_path_buf(),
    };
    let command = resolve_command(&fixture.command, &cwd);

    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| anyhow!("could not open a PTY: {error}"))?;
    let mut builder = CommandBuilder::new(&command);
    builder.args(fixture.args.iter().flatten());
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    for (name, value) in fixture.env.iter().flatten() {
        builder.env(name, value);
    }
    builder.cwd(&cwd);
    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| anyhow!("{error:#}"))?;
    drop(pair.slave);
    let mut killer = child.clone_killer();
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| anyhow!("could not read the PTY: {error}"))?;
    let writer =
        Arc::new(Mutex::new(pair.master.take_writer().map_err(|error| {
            anyhow!("could not write to the PTY: {error}")
        })?));

    let exit = Arc::new(Exit::default());
    {
        let (terminal, exit) = (Arc::clone(terminal), Arc::clone(&exit));
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                lock(&terminal).write(&buffer[..read]);
            }
            exit.reader_done.store(true, Ordering::Release);
        });
    }
    {
        let exit = Arc::clone(&exit);
        std::thread::spawn(move || {
            let code = child.wait().map_or(1, |status| status.exit_code());
            *lock(&exit.code) = Some(code);
        });
    }

    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    let remaining = || {
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as f64
    };
    let screen = || lock(terminal).plain_text();
    let exited = || *lock(&exit.code);
    // Output the child wrote before exiting may still be in flight: treat the
    // exit as final once the reader hit EOF or a short grace has passed.
    let exit_seen = Mutex::new(None::<Instant>);
    let exit_grace = || *lock(&exit_seen).get_or_insert_with(Instant::now);
    let drained = |seen: Instant| {
        exit.reader_done.load(Ordering::Acquire) || seen.elapsed() > Duration::from_millis(500)
    };
    let write = |data: &str| {
        use std::io::Write;
        let mut writer = lock(&writer);
        let _ = writer.write_all(data.as_bytes());
        let _ = writer.flush();
    };

    let run = async {
        for action in fixture.actions.iter().flatten() {
            if remaining() == 0.0 {
                bail!(
                    "PTY movie exceeded timeoutMs {}",
                    fixture.timeout_ms.unwrap_or(30_000)
                );
            }
            match action {
                PtyAction::WaitFor {
                    wait_for,
                    timeout_ms,
                } => {
                    let action_deadline = deadline
                        .min(Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(120_000)));
                    loop {
                        if Instant::now() > action_deadline {
                            bail!(
                                "Timed out waiting for {}. Visible:\n{}",
                                json_string(wait_for),
                                slice_utf16(&screen(), 1_200)
                            );
                        }
                        if screen().contains(wait_for.as_str()) {
                            break;
                        }
                        if exited().is_some() && drained(exit_grace()) {
                            bail!(
                                "Timed out waiting for {}. Visible:\n{}",
                                json_string(wait_for),
                                slice_utf16(&screen(), 1_200)
                            );
                        }
                        delay(20.0).await;
                    }
                }
                PtyAction::WaitForExit { timeout_ms, .. } => {
                    let action_deadline = deadline
                        .min(Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(120_000)));
                    loop {
                        if Instant::now() > action_deadline {
                            bail!("Timed out waiting for PTY exit");
                        }
                        if exited().is_some() {
                            break;
                        }
                        delay(20.0).await;
                    }
                }
                PtyAction::Key { key } => write(keystroke(*key)),
                PtyAction::Text { text } => write(text),
                PtyAction::Pause { pause_ms } => delay((*pause_ms as f64).min(remaining())).await,
            }
        }
        delay(settle_ms.min(remaining())).await;
        Ok(())
    }
    .await;

    let _ = killer.kill();
    // Drain a short window so teardown is clean.
    let drain = Instant::now() + Duration::from_millis(500);
    while exited().is_none() && Instant::now() < drain {
        delay(20.0).await;
    }
    drop(pair.master);
    run?;

    let visible = screen();
    for expected in fixture.expect_text.iter().flatten() {
        if !visible.contains(expected.as_str()) {
            bail!(
                "PTY movie did not render expected text {}. Visible:\n{}",
                json_string(expected),
                slice_utf16(&visible, 1_200)
            );
        }
    }
    // Exit code is advisory: we may kill the child after sampling.
    let _ = fixture.allow_non_zero_exit;
    Ok(())
}

/// `KEYSTROKES[key]`, shared with tui-shot's table.
fn keystroke(key: PtyKey) -> &'static str {
    tui_keystroke(match key {
        PtyKey::Enter => TuiPtyKey::Enter,
        PtyKey::Up => TuiPtyKey::Up,
        PtyKey::Down => TuiPtyKey::Down,
        PtyKey::Left => TuiPtyKey::Left,
        PtyKey::Right => TuiPtyKey::Right,
        PtyKey::Tab => TuiPtyKey::Tab,
        PtyKey::Escape => TuiPtyKey::Escape,
        PtyKey::Backspace => TuiPtyKey::Backspace,
        PtyKey::Space => TuiPtyKey::Space,
        PtyKey::CtrlC => TuiPtyKey::CtrlC,
        PtyKey::CtrlD => TuiPtyKey::CtrlD,
    })
}

fn resolve_command(command: &str, cwd: &Path) -> String {
    if command.contains(std::path::MAIN_SEPARATOR) || command.starts_with('.') {
        return resolve_path(cwd, command).to_string_lossy().into_owned();
    }
    command.to_string()
}

/// `Number.parseInt(text, 16)` on the leading hex digits, `None` for NaN.
fn parse_hex_prefix(text: &str) -> Option<u8> {
    let digits: String = text.chars().take_while(char::is_ascii_hexdigit).collect();
    u8::from_str_radix(&digits, 16).ok()
}

/// Synthetic truecolor PTY-like movie without a child process (CI smoke).
/// `color` is hex without `#`, default `7c5cff`. `options.source` is
/// overwritten with `pty`.
pub async fn record_truecolor_demo_movie(
    options: MovieSessionOptions,
    color: Option<&str>,
) -> Result<MovieArtifact> {
    let color = color.unwrap_or("7c5cff");
    let slice = |from: usize| color.chars().skip(from).take(2).collect::<String>();
    let (Some(r), Some(g), Some(b)) = (
        parse_hex_prefix(&slice(0)),
        parse_hex_prefix(&slice(2)),
        parse_hex_prefix(&slice(4)),
    ) else {
        bail!("invalid color {color}");
    };

    let (cols, rows) = (40u16, 8u16);
    let mut raster = RasterOptions::movie(cols, rows);
    raster.font_size = 16.0;
    let paint = Paint { raster };
    let size = paint.size();

    let status = options.status.unwrap_or(ManifestStatus::Running);
    let mut session_options = options;
    session_options.size = Some(size);
    session_options.fps = Some(session_options.fps.unwrap_or(10.0));
    session_options.source = MovieSourceKind::Pty;
    let mut session = MovieSession::create(session_options)?;

    let mut terminal = HeadlessTerminal::new(cols, rows);
    for label in ["truecolor", "movie", "harness", color] {
        terminal.write(format!("\x1b[2J\x1b[H\x1b[38;2;{r};{g};{b}m{label}\x1b[0m\r\n").as_bytes());
        let frame = terminal.frame(paint.raster.foreground_rgb(), paint.raster.background_rgb());
        let first = frame.cell(0, 0).map(|cell| cell.foreground);
        if first != Some([r, g, b]) {
            bail!("truecolor path lost #{color}; cell(0,0).foreground={first:?}");
        }
        let png = paint.png(&terminal)?;
        session.push_frame(&png, FrameExtension::Png)?;
        delay(120.0).await;
    }
    session
        .stop(StopOptions {
            status: Some(status),
            ..StopOptions::default()
        })
        .await
}

#[cfg(test)]
mod tests;
