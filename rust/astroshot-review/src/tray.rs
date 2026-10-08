//! The review tray: `astroshot review` as a library.
//!
//! Ink's `render()` becomes an explicit loop (rust/PORTING.md decision 3):
//! crossterm raw mode + alternate screen + bracketed paste, then one
//! `tokio::select!` over terminal events, the hooks' wake signal, and process
//! signals. ratatui draws each frame into a buffer that is handed to the
//! [`GraphicsStdout`] as one chunk, so kitty placements are spliced into the
//! same synchronized update as the text, as they were with Ink.
//!
//! Teardown never waits without a bound: the terminal is restored first and
//! the store gets a fixed time to flush its index. A host should run the tray
//! on a multi-thread tokio runtime and shut that runtime down with a timeout
//! afterwards (a deep scan may still be walking the disk);
//! [`run_tray_blocking`] does both.
//!
//! This module prints nothing: usage errors, help and the "needs a TTY"
//! message belong to the program that parses arguments.

use std::io::{self, IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::EventStream;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use tokio::sync::Notify;

use crate::data::store::{ReviewStore, StoreOptions};
use crate::images::service::{ImageService, ImageServiceImpl, ImageServiceOptions};
use crate::terminal::graphics_stdout::{GraphicsStdout, create_graphics_stdout};
use crate::terminal::image_layer::{ImageLayer, ImageLayerOptions};
#[cfg(unix)]
use crate::terminal::probe::TerminalCapabilities;
use crate::terminal::probe::{Env, ProbeOptions, StdioStreams, probe_terminal};
use crate::ui::app::{App, AppProps, Outcome};
use crate::ui::context::{AppServices, RootsSource};
use crate::ui::hooks::{TerminalSize, Wake};
use crate::video::ffmpeg::detect_ffmpeg;

/// How long teardown waits for the store to stop and flush its index.
const DISPOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long [`run_tray_blocking`] lets the runtime wind down after the tray
/// has exited.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);

/// What to run the tray with.
#[derive(Debug, Clone)]
pub struct TrayOptions {
    /// Absolute, existing folders to watch (see [`resolve_roots`]).
    pub roots: Vec<String>,
    /// Where the roots came from; shown in the settings pane.
    pub roots_source: RootsSource,
    /// Render pictures with terminal graphics (otherwise half-block text).
    pub graphics: bool,
    /// Follow filesystem changes.
    pub watch: bool,
    /// Use the on-disk index so the tray opens instantly.
    pub use_index: bool,
    /// Version shown in the settings pane.
    pub version: String,
    /// The command that starts the tray, shown in hints and the settings
    /// header: `astroshot review` standalone, or whatever the host calls it.
    pub command: String,
}

impl TrayOptions {
    /// Watch `roots` with graphics, watching and the index on, labelled as
    /// `astroshot review`.
    pub fn new(roots: Vec<String>) -> Self {
        Self {
            roots,
            roots_source: RootsSource::Cli,
            graphics: true,
            watch: true,
            use_index: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            command: "astroshot review".to_string(),
        }
    }
}

/// Why the tray could not run.
#[derive(Debug)]
pub enum TrayError {
    /// Stdin or stdout is not a terminal.
    NotATerminal,
    /// The terminal could not be driven (raw mode, alternate screen, draw).
    Terminal(io::Error),
}

impl std::fmt::Display for TrayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotATerminal => f.write_str(
                "the review tray needs an interactive terminal (stdin and stdout must be a TTY)",
            ),
            Self::Terminal(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TrayError {}

/// Roots split into those that exist as directories and those that do not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedRoots {
    pub existing: Vec<String>,
    pub missing: Vec<String>,
}

/// Resolve `roots` against `cwd` (lexically, like `path.resolve`) and split
/// them by whether they are directories. Order is preserved.
pub fn resolve_roots(cwd: &str, roots: &[String]) -> ResolvedRoots {
    let mut resolved = ResolvedRoots::default();
    for root in roots {
        let root = resolve(cwd, root);
        match std::fs::metadata(&root) {
            Ok(metadata) if metadata.is_dir() => resolved.existing.push(root),
            // Something that is not a folder is skipped without a word.
            Ok(_) => {}
            Err(_) => resolved.missing.push(root),
        }
    }
    resolved
}

/// `path.resolve(root)` against `cwd` (lexical, POSIX).
fn resolve(cwd: &str, root: &str) -> String {
    let joined = if root.starts_with('/') {
        root.to_string()
    } else {
        format!("{cwd}/{root}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

type DebugLog = Option<Arc<dyn Fn(&str) + Send + Sync>>;

/// `ASTROSHOT_REVIEW_DEBUG` appends timestamped lines to `astroshot-review.log`.
fn debug_log() -> DebugLog {
    std::env::var("ASTROSHOT_REVIEW_DEBUG")
        .ok()
        .filter(|value| !value.is_empty())?;
    Some(Arc::new(|message: &str| {
        let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("astroshot-review.log")
        {
            let _ = writeln!(file, "{stamp} {message}");
        }
    }))
}

fn log(debug: &DebugLog, message: impl FnOnce() -> String) {
    if let Some(debug) = debug {
        debug(&message());
    }
}

/// Write straight to the terminal, bypassing the UI (`process.stdout.write`).
fn write_stdout(data: &str) {
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(data.as_bytes());
    let _ = stdout.flush();
}

/// Collects everything ratatui writes for one frame and hands it to the
/// graphics stdout as a single chunk on `flush`, inside one synchronized
/// update. Ink wrote whole frames the same way; the graphics stdout decides
/// per chunk whether to splice placements in.
struct FrameWriter<W: Write> {
    pending: Vec<u8>,
    out: GraphicsStdout<W>,
}

impl<W: Write> FrameWriter<W> {
    fn new(out: GraphicsStdout<W>) -> Self {
        Self {
            pending: Vec::new(),
            out,
        }
    }
}

const BEGIN_SYNC: &str = "\x1b[?2026h";
const END_SYNC: &str = "\x1b[?2026l";

impl<W: Write> Write for FrameWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let frame = String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned();
        self.out
            .write_str(&format!("{BEGIN_SYNC}{frame}{END_SYNC}"))?;
        self.out.flush()
    }
}

const ENTER_SCREEN: &str = "\x1b[?1049h\x1b[?25l\x1b[?2004h";
const LEAVE_SCREEN: &str = "\x1b[?2004l\x1b[?1049l\x1b[?25h";

static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);
static UI_THREAD: std::sync::Mutex<Option<std::thread::ThreadId>> = std::sync::Mutex::new(None);
static HOOK_INSTALLED: std::sync::Once = std::sync::Once::new();

/// Leave the alternate screen, show the cursor and leave raw mode. Runs once
/// per `enter`, from whichever exit path gets there first.
fn restore_terminal() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    *UI_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // The terminal may already be gone; nothing to do about a failed write.
    write_stdout(LEAVE_SCREEN);
    let _ = crossterm::terminal::disable_raw_mode();
}

/// Restores the terminal when dropped, so an early return or an unwinding
/// panic cannot leave the alternate screen behind.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        // A panic on the UI thread prints its message after the screen is
        // back. The hook is chained once per process and stays a pass-through
        // after the tray exits, so a host that runs `review` again (or does
        // anything else afterwards) does not accumulate hooks.
        *UI_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current().id());
        HOOK_INSTALLED.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let ui_thread = *UI_THREAD.lock().unwrap_or_else(|e| e.into_inner());
                if ui_thread == Some(std::thread::current().id()) {
                    restore_terminal();
                }
                previous(info);
            }));
        });
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// `clearPictures`: delete every placement once and close the herdr streams.
struct Pictures {
    layer: ImageLayer,
    cleared: AtomicBool,
}

impl Pictures {
    fn clear(&self) {
        if self.cleared.swap(true, Ordering::SeqCst) {
            return;
        }
        let output = self.layer.clear();
        if !output.is_empty() {
            write_stdout(&output);
        }
        self.layer.dispose_herdr();
    }
}

/// Resolves with the exit code of the first termination signal: `kill <pid>`
/// or a closing terminal must not leave pictures or the alternate screen behind.
///
/// The handlers are installed by the call, not by the first poll of the
/// returned future: `select!` does not poll its other branches while one is
/// ready, so a signal arriving during a busy stretch of the loop would still
/// have had its default action and killed the process mid-screen.
#[cfg(unix)]
fn termination_signal() -> impl Future<Output = i32> {
    use tokio::signal::unix::{SignalKind, signal};
    let signals = (
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
        signal(SignalKind::quit()),
        signal(SignalKind::interrupt()),
    );
    async move {
        let (Ok(mut term), Ok(mut hup), Ok(mut quit), Ok(mut int)) = signals else {
            return std::future::pending().await;
        };
        tokio::select! {
            _ = hup.recv() => 129,
            _ = term.recv() => 143,
            _ = quit.recv() => 143,
            _ = int.recv() => 143,
        }
    }
}

#[cfg(not(unix))]
fn termination_signal() -> impl Future<Output = i32> {
    async {
        let _ = tokio::signal::ctrl_c().await;
        143
    }
}

/// Inside herdr, raw Kitty escapes are dropped; render through its socket
/// graphics API instead (pixel-perfect), falling back to half-blocks with an
/// actionable reason when herdr can't yet report the host cell size.
#[cfg(unix)]
async fn herdr_sink(
    capabilities: &mut TerminalCapabilities,
    debug: &DebugLog,
) -> Option<super::terminal::herdr::HerdrSink> {
    use super::terminal::herdr::{
        DiscoverOptions, HerdrSink, discover_herdr, herdr_address_from_env, probe_herdr_set,
    };
    use super::terminal::probe::{CellSource, GraphicsProtocol};

    let herdr = herdr_address_from_env()?;
    let waiting = debug.clone();
    let discovery = discover_herdr(
        &herdr,
        DiscoverOptions {
            on_waiting: Some(Box::new(move || {
                log(&waiting, || "herdr: waiting for host cell size".to_string());
            })),
            ..DiscoverOptions::default()
        },
    )
    .await;
    let cell = |value: Option<u32>| value.filter(|value| *value != 0);
    let (true, Some(cell_width), Some(cell_height)) = (
        discovery.ok,
        cell(discovery.cell_width),
        cell(discovery.cell_height),
    ) else {
        if let Some(reason) = discovery.reason.filter(|reason| !reason.is_empty()) {
            capabilities.reason = Some(reason);
        }
        return None;
    };
    let set_support = probe_herdr_set(&herdr).await;
    if !set_support.ok {
        // herdr can size images but not place per-image layers; half-blocks it is.
        capabilities.reason = Some(
            set_support
                .reason
                .unwrap_or_else(|| "herdr pane.graphics.set unavailable".to_string()),
        );
        return None;
    }
    let errors = debug.clone();
    let sink = HerdrSink::new(herdr, move |error| {
        log(&errors, || format!("herdr: {error}"));
    });
    capabilities.graphics = GraphicsProtocol::Herdr;
    capabilities.cell_width = cell_width;
    capabilities.cell_height = cell_height;
    capabilities.cell_source = CellSource::Query;
    capabilities.reason = None;
    Some(sink)
}

/// Run the tray until the user quits or a termination signal arrives.
/// Returns the exit code (0 for quit, 129/143/... for signals) after the
/// terminal is restored. Needs a tokio runtime.
pub async fn run_tray(options: TrayOptions) -> Result<i32, TrayError> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        return Err(TrayError::NotATerminal);
    }
    let debug = debug_log();
    let env: Option<Env> = (!options.graphics).then(|| {
        let mut env: Env = std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        env.insert("ASTROSHOT_REVIEW_GRAPHICS".into(), "none".into());
        env
    });
    #[allow(unused_mut)]
    let mut capabilities = probe_terminal(
        &mut StdioStreams::new(),
        ProbeOptions {
            env,
            ..ProbeOptions::default()
        },
    )
    .await;

    #[cfg(unix)]
    let herdr = if options.graphics {
        herdr_sink(&mut capabilities, &debug).await
    } else {
        None
    };
    log(&debug, || {
        format!("capabilities {}", capabilities.to_json())
    });
    let ffmpeg = detect_ffmpeg();
    let service = ImageServiceImpl::new(ImageServiceOptions::default());
    let shared_service: Arc<dyn ImageService> = Arc::new(service.clone());
    let mut layer_options =
        ImageLayerOptions::new(capabilities.clone(), shared_service.clone(), write_stdout);
    #[cfg(unix)]
    {
        layer_options.herdr = herdr;
    }
    let image_errors = debug.clone();
    layer_options.on_error = Some(Box::new(move |src, error| {
        log(&image_errors, || format!("image {src}: {error}"));
    }));
    if let Some(debug) = debug.clone() {
        layer_options.on_debug = Some(Box::new(move |message| debug(&format!("layer {message}"))));
    }
    let layer = ImageLayer::new(layer_options);
    let store_log = debug.clone();
    let store = ReviewStore::with_options(StoreOptions {
        roots: options.roots,
        watch: Some(options.watch),
        use_index: Some(options.use_index),
        on_log: Some(Arc::new(move |message| {
            log(&store_log, || format!("store {message}"));
        })),
        ..StoreOptions::default()
    });
    // `void store.start()`.
    let starting = store.clone();
    tokio::spawn(async move { starting.start().await });

    let services = AppServices {
        store: store.clone(),
        layer: layer.clone(),
        service: shared_service,
        capabilities,
        ffmpeg,
        roots_source: options.roots_source,
        version: options.version.clone(),
        command: options.command.clone(),
    };
    let pictures = Arc::new(Pictures {
        layer: layer.clone(),
        cleared: AtomicBool::new(false),
    });

    let outcome = match run_app(&services, pictures.clone()).await {
        Ok(code) => Ok(code),
        Err(error) => {
            restore_terminal();
            Err(TrayError::Terminal(error))
        }
    };
    // `finally`: pictures first, then the store and the image workers. The
    // terminal is already restored; neither wait is unbounded.
    pictures.clear();
    let _ = tokio::time::timeout(DISPOSE_TIMEOUT, store.dispose()).await;
    service.dispose();
    outcome
}

/// [`run_tray`] on a fresh multi-thread runtime that is shut down with a
/// short timeout afterwards, so a scan still walking the disk cannot hold the
/// process open after quit. Call from a non-async context.
pub fn run_tray_blocking(options: TrayOptions) -> Result<i32, TrayError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(TrayError::Terminal)?;
    let outcome = runtime.block_on(run_tray(options));
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
    outcome
}

type Tui = Terminal<CrosstermBackend<FrameWriter<io::Stdout>>>;

fn draw(terminal: &mut Tui, app: &mut App, layer: &ImageLayer) -> io::Result<()> {
    terminal.draw(|frame| app.render(frame))?;
    // A frame with no changed text is not spliced; place pictures anyway.
    layer.flush_now();
    Ok(())
}

/// Ink's `render(<App/>)` + `waitUntilExit()`.
async fn run_app(services: &AppServices, pictures: Arc<Pictures>) -> io::Result<i32> {
    let notify = Arc::new(Notify::new());
    let wake_target = notify.clone();
    let wake: Wake = Arc::new(move || wake_target.notify_one());

    let guard = TerminalGuard::enter()?;
    let mut writer = FrameWriter::new(create_graphics_stdout(io::stdout(), services.layer.clone()));
    // Through the graphics stdout: entering the alternate screen homes the
    // cursor and makes the layer re-send its placements.
    writer.out.write_str(ENTER_SCREEN)?;
    writer.out.flush()?;
    let backend = CrosstermBackend::new(writer);
    let mut terminal = match crossterm::terminal::size() {
        Ok((columns, rows)) if columns != 0 && rows != 0 => Terminal::new(backend)?,
        // A terminal that reports no size: Ink draws at 80x24, and ratatui
        // would otherwise draw into an empty area.
        _ => {
            let size = TerminalSize::from_reported(None, None);
            let viewport = Viewport::Fixed(Rect::new(0, 0, size.columns, size.rows));
            Terminal::with_options(backend, TerminalOptions { viewport })?
        }
    };

    let on_quit = pictures.clone();
    let mut app = App::new(
        services,
        AppProps {
            on_quit: Some(Box::new(move || on_quit.clear())),
        },
        TerminalSize::current(),
        wake,
    );

    let mut events = EventStream::new();
    let signal = termination_signal();
    tokio::pin!(signal);
    draw(&mut terminal, &mut app, &services.layer)?;
    let code = loop {
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => {
                    if app.handle(event) == Outcome::Quit {
                        break 0;
                    }
                }
                // Input ended (the terminal went away): nothing more can happen.
                Some(Err(_)) | None => break 0,
            },
            () = notify.notified() => {}
            code = &mut signal => break code,
        }
        draw(&mut terminal, &mut app, &services.layer)?;
    };

    // Unmount: drop the widgets' pictures, delete placements, then restore.
    drop(app);
    pictures.clear();
    drop(terminal);
    drop(guard);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_follows_path_resolve() {
        assert_eq!(resolve("/work/dir", "a/b"), "/work/dir/a/b");
        assert_eq!(resolve("/work/dir", "../x/./y/"), "/work/x/y");
        assert_eq!(resolve("/work", "/abs//path/"), "/abs/path");
        assert_eq!(resolve("/work", "."), "/work");
        assert_eq!(resolve("/work", "../../.."), "/");
    }

    #[tokio::test]
    async fn frames_reach_the_terminal_as_one_synchronized_chunk() {
        use crate::terminal::probe::GraphicsProtocol;
        use crate::ui::context::test_support::{FakeService, services};

        use std::sync::Mutex;

        struct Chunks(Arc<Mutex<Vec<String>>>);
        impl Write for Chunks {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(buf).into_owned());
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let services = services(GraphicsProtocol::None, FakeService::new(false));
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let mut writer = FrameWriter::new(create_graphics_stdout(
            Chunks(chunks.clone()),
            services.layer.clone(),
        ));
        writer.out.write_str(ENTER_SCREEN).unwrap();
        writer.write_all(b"\x1b[1;1H").unwrap();
        writer.write_all(b"hello").unwrap();
        assert_eq!(chunks.lock().unwrap().len(), 1);
        writer.flush().unwrap();
        // Nothing buffered: a second flush writes nothing.
        writer.flush().unwrap();
        assert_eq!(
            *chunks.lock().unwrap(),
            [
                // The graphics stdout homes and clears on entering the alternate screen.
                "\x1b[?1049h\x1b[H\x1b[2J\x1b[?25l\x1b[?2004h",
                "\x1b[?2026h\x1b[1;1Hhello\x1b[?2026l",
            ]
        );
    }

    #[test]
    fn resolve_roots_splits_directories_from_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("file"), "x").unwrap();
        let cwd = dir.path().to_string_lossy().into_owned();
        let resolved = resolve_roots(&cwd, &["a".into(), "file".into(), "nope".into()]);
        assert_eq!(resolved.existing, [format!("{cwd}/a")]);
        assert_eq!(resolved.missing, [format!("{cwd}/nope")]);
    }
}
