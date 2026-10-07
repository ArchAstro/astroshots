//! Port of `packages/astroshot-review/src/cli.ts`: `astroshot review`.
//!
//! Ink's `render()` becomes an explicit loop (rust/PORTING.md decision 3):
//! crossterm raw mode + alternate screen + bracketed paste, then one
//! `tokio::select!` over terminal events, the hooks' wake signal, and process
//! signals. ratatui draws each frame into a buffer that is handed to the
//! [`GraphicsStdout`] as one chunk, so kitty placements are spliced into the
//! same synchronized update as the text, as they were with Ink.
//!
//! Teardown never waits without a bound: the terminal is restored first, the
//! store gets a fixed time to flush its index, and `main.rs` shuts the runtime
//! down with a timeout so a deep scan still walking the disk cannot hold the
//! process open after `q`.

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

use super::data::store::{ReviewStore, StoreOptions};
use super::images::service::{ImageService, ImageServiceImpl, ImageServiceOptions};
use super::terminal::graphics_stdout::{GraphicsStdout, create_graphics_stdout};
use super::terminal::image_layer::{ImageLayer, ImageLayerOptions};
use super::terminal::probe::{
    Env, ProbeOptions, StdioStreams, TerminalCapabilities, probe_terminal,
};
use super::ui::app::{App, AppProps, Outcome};
use super::ui::context::{AppServices, RootsSource};
use super::ui::hooks::{TerminalSize, Wake};
use super::video::ffmpeg::detect_ffmpeg;

const HELP: &str = "astroshot review — the Astroshots tray in your terminal

Usage:
  astroshot review [<dir>...] [options]

Options:
  --root <dir>       Folder to watch for .astroshot/ trees (repeatable)
  --no-graphics      Skip terminal pictures (text only)
  --no-watch         Do not follow filesystem changes
  --no-index         Ignore the on-disk index (full scan every start)
  -h, --help         Show this help

Without roots, `astroshot review` uses the folders the Astroshots app
watches (macOS) and otherwise the current directory. Images render pixel-
perfect in Kitty-protocol terminals (Ghostty, kitty, WezTerm) and inside
herdr (enable [experimental] kitty_graphics and reattach the client once);
elsewhere — mosh, tmux, plain terminals — they render as truecolor
half-block text. Movie playback needs ffmpeg on PATH.

Keys: ↑↓ move · ⏎ open · f full screen · s seen · c feedback · u history ·
      m movies · 1/2 tabs · , settings · ? help · q quit";

const NEEDS_TTY: &str =
    "astroshot review needs an interactive terminal (stdin and stdout must be a TTY).";

/// How long teardown waits for the store to stop and flush its index.
const DISPOSE_TIMEOUT: Duration = Duration::from_secs(2);

pub fn review_help() -> &'static str {
    HELP
}

/// `interface ParsedArgs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArgs {
    pub roots: Vec<String>,
    pub graphics: bool,
    pub watch: bool,
    pub index: bool,
    pub help: bool,
    pub version: bool,
    pub roots_source: RootsSource,
}

/// `parseArgs(argv)`; the error is the message the TS throws.
pub fn parse_args(argv: &[String]) -> Result<ParsedArgs, String> {
    let cwd = std::env::current_dir()
        .map(|cwd| cwd.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());
    parse_args_in(argv, &cwd)
}

fn parse_args_in(argv: &[String], cwd: &str) -> Result<ParsedArgs, String> {
    let mut parsed = ParsedArgs {
        roots: Vec::new(),
        graphics: true,
        watch: true,
        index: true,
        help: false,
        version: false,
        roots_source: RootsSource::Cli,
    };
    let mut index = 0;
    while index < argv.len() {
        let argument = argv[index].as_str();
        match argument {
            "-h" | "--help" => parsed.help = true,
            "-v" | "--version" => parsed.version = true,
            "--no-graphics" => parsed.graphics = false,
            "--no-watch" => parsed.watch = false,
            "--no-index" => parsed.index = false,
            "--root" => {
                let value = argv.get(index + 1).filter(|value| !value.is_empty());
                let Some(value) = value else {
                    return Err("--root requires a directory".to_string());
                };
                parsed.roots.push(value.clone());
                index += 1;
            }
            "--roots-source" => {
                parsed.roots_source = match argv.get(index + 1).map(String::as_str) {
                    Some("app") => RootsSource::App,
                    Some("cli") => RootsSource::Cli,
                    Some("cwd") => RootsSource::Cwd,
                    _ => return Err("--roots-source must be app, cli, or cwd".to_string()),
                };
                index += 1;
            }
            _ if argument.starts_with('-') => {
                return Err(format!("Unknown option: {argument}"));
            }
            _ => parsed.roots.push(argument.to_string()),
        }
        index += 1;
    }
    if parsed.roots.is_empty() {
        parsed.roots = vec![cwd.to_string()];
        parsed.roots_source = RootsSource::Cwd;
    }
    Ok(parsed)
}

/// `readVersion()`: the TS read its package.json; the crate carries the version.
fn read_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
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

/// Leave the alternate screen, show the cursor and leave raw mode. Runs once
/// per `enter`, from whichever exit path gets there first.
fn restore_terminal() {
    if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
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
        // A panic on the UI thread prints its message after the screen is back.
        let ui_thread = std::thread::current().id();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().id() == ui_thread {
                restore_terminal();
            }
            previous(info);
        }));
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

/// `main(argv)`: run `astroshot review` with the arguments after the
/// subcommand; returns the exit code.
pub async fn run(argv: &[String]) -> i32 {
    let args = match parse_args(argv) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            eprintln!();
            eprintln!("{}", review_help());
            return 1;
        }
    };
    if args.help {
        println!("{}", review_help());
        return 0;
    }
    if args.version {
        println!("{}", read_version());
        return 0;
    }
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        eprintln!("{NEEDS_TTY}");
        return 1;
    }
    let cwd = std::env::current_dir()
        .map(|cwd| cwd.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());
    let roots: Vec<String> = args
        .roots
        .iter()
        .map(|root| resolve(&cwd, root))
        .filter(|root| match std::fs::metadata(root) {
            Ok(metadata) => metadata.is_dir(),
            Err(_) => {
                eprintln!("Ignoring missing folder: {root}");
                false
            }
        })
        .collect();

    let debug = debug_log();
    let env: Option<Env> = (!args.graphics).then(|| {
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
    let herdr = if args.graphics {
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
        roots,
        watch: Some(args.watch),
        use_index: Some(args.index),
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
        roots_source: args.roots_source,
        version: read_version(),
    };
    let pictures = Arc::new(Pictures {
        layer: layer.clone(),
        cleared: AtomicBool::new(false),
    });

    let code = match run_app(&services, pictures.clone()).await {
        Ok(code) => code,
        Err(error) => {
            restore_terminal();
            eprintln!("astroshot review: {error}");
            1
        }
    };
    // `finally`: pictures first, then the store and the image workers. The
    // terminal is already restored; neither wait is unbounded.
    pictures.clear();
    let _ = tokio::time::timeout(DISPOSE_TIMEOUT, store.dispose()).await;
    service.dispose();
    code
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

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn parse(list: &[&str]) -> Result<ParsedArgs, String> {
        parse_args_in(&args(list), "/work")
    }

    #[test]
    fn help_matches_the_ts_text() {
        // The template literal `reviewHelp()` returns in cli.ts.
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/astroshot-review/src/cli.ts"
        ));
        let start = source.find("return `astroshot review").unwrap() + "return `".len();
        let end = start + source[start..].find("`;\n}").unwrap();
        let expected = source[start..end].replace("\\`", "`");
        assert_eq!(review_help(), expected);
        assert!(
            review_help().starts_with("astroshot review — the Astroshots tray in your terminal\n")
        );
        assert!(review_help().ends_with("· , settings · ? help · q quit"));
    }

    #[test]
    fn no_arguments_watch_the_current_directory() {
        assert_eq!(
            parse(&[]).unwrap(),
            ParsedArgs {
                roots: vec!["/work".into()],
                graphics: true,
                watch: true,
                index: true,
                help: false,
                version: false,
                roots_source: RootsSource::Cwd,
            }
        );
    }

    #[test]
    fn positionals_and_root_flags_collect_in_order() {
        let parsed = parse(&["a", "--root", "b", "c", "--root", "d"]).unwrap();
        assert_eq!(parsed.roots, ["a", "b", "c", "d"]);
        assert_eq!(parsed.roots_source, RootsSource::Cli);
    }

    #[test]
    fn switches_turn_features_off() {
        let parsed = parse(&["--no-graphics", "--no-watch", "--no-index", "x"]).unwrap();
        assert_eq!(
            (parsed.graphics, parsed.watch, parsed.index),
            (false, false, false)
        );
        assert!(parse(&["-h"]).unwrap().help && parse(&["--help"]).unwrap().help);
        assert!(parse(&["-v"]).unwrap().version && parse(&["--version"]).unwrap().version);
    }

    #[test]
    fn roots_source_accepts_only_the_three_literals() {
        for (value, source) in [
            ("app", RootsSource::App),
            ("cli", RootsSource::Cli),
            ("cwd", RootsSource::Cwd),
        ] {
            let parsed = parse(&["x", "--roots-source", value]).unwrap();
            assert_eq!(parsed.roots_source, source);
        }
        let message = "--roots-source must be app, cli, or cwd";
        assert_eq!(parse(&["--roots-source", "nope"]).unwrap_err(), message);
        assert_eq!(parse(&["--roots-source"]).unwrap_err(), message);
        // Without a root the source is the working directory, whatever was passed.
        assert_eq!(
            parse(&["--roots-source", "app"]).unwrap().roots_source,
            RootsSource::Cwd
        );
    }

    #[test]
    fn usage_errors_carry_the_ts_messages() {
        assert_eq!(
            parse(&["--root"]).unwrap_err(),
            "--root requires a directory"
        );
        assert_eq!(
            parse(&["--root", ""]).unwrap_err(),
            "--root requires a directory"
        );
        assert_eq!(parse(&["--nope"]).unwrap_err(), "Unknown option: --nope");
        assert_eq!(parse(&["-"]).unwrap_err(), "Unknown option: -");
        // A flag is taken as the value of --root, as in the TS.
        assert_eq!(
            parse(&["--root", "--no-watch"]).unwrap().roots,
            ["--no-watch"]
        );
    }

    #[test]
    fn resolve_follows_path_resolve() {
        assert_eq!(resolve("/work/dir", "a/b"), "/work/dir/a/b");
        assert_eq!(resolve("/work/dir", "../x/./y/"), "/work/x/y");
        assert_eq!(resolve("/work", "/abs//path/"), "/abs/path");
        assert_eq!(resolve("/work", "."), "/work");
        assert_eq!(resolve("/work", "../../.."), "/");
    }

    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(read_version(), env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn frames_reach_the_terminal_as_one_synchronized_chunk() {
        use crate::astroshot_review::terminal::probe::GraphicsProtocol;
        use crate::astroshot_review::ui::context::test_support::{FakeService, services};

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
}
