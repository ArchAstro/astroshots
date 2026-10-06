//! Port of `packages/astroshot/bin/astroshot.mjs`: the top-level `astroshot`
//! dispatcher. The TS bin spawned `node` on each engine's own bin script;
//! here every subcommand is an in-process function. Engines that are not
//! ported yet dispatch to [`pending`] and exit 2.

use std::path::PathBuf;

use crate::bin::demo::{demo_help, run_demo};
use crate::bin::doctor::{doctor_help, run_doctor};
use crate::bin::mac_preferences::{ReadWatchConfigurationOptions, read_watch_configuration};
use crate::bin::templates::{WriteFixtureTemplateOptions, write_fixture_template};

const HELP: &str = r#"astroshot — one CLI for React, Ink, PTY stills, and movies

Usage:
  astroshot review [<dir>...] [--root <dir>] [--no-graphics]
  astroshot demo [--feature <name>] [--root <dir>] [--dry-run] [--clean]
  astroshot doctor [--root <dir>] [--json]
  astroshot init react [fixture.tsx] [--force]
  astroshot init ink [fixture.tsx] [--force]
  astroshot init pty [fixture.yaml] [--force]
  astroshot react <fixture.tsx> -o <out.png> [options]
  astroshot react shot <fixture.tsx> -o <out.png> [options]
  astroshot react batch <manifest.yaml|json> [options]
  astroshot ink <fixture.tsx> -o <out.png> [options]
  astroshot ink batch <manifest.yaml|json> [options]
  astroshot pty <fixture.yaml|json> -o <out.png> [options]
  astroshot movie <command> [options]
  astroshot install-browser [--with-deps]

Start here:
  review             Review the .astroshot/ stream in your terminal (Kitty graphics)
  demo               Write a complete .astroshot/ example set (no prerequisites)
  doctor             Check Node, watched folders, app, Chromium, permissions

Commands:
  react              Capture an isolated React component (still PNG)
  ink                Capture an Ink component fixture (still PNG)
  pty                Capture any executable in a pseudoterminal (still PNG)
  movie              Record a journey movie into .astroshot/ (poster + video)
  init               Generate a React, Ink, or PTY fixture template
  install-browser    Install the shared Chromium runtime

Movie sources (see "astroshot movie which-source"):
  browser            Web / agent-browser / Playwright viewport
  pty                Truecolor TUI/CLI (never screenshot Terminal.app)
  desktop.window     Native macOS app window (uses OS screencapture)
  frames             Push your own PNG/JPEG sequence

Compatibility: "astroshot tui" remains an alias for "astroshot ink".
Run "astroshot <mode> --help" for mode options."#;

const REACT_HELP: &str = r#"astroshot react — deterministic React component screenshots

Usage:
  astroshot init react [fixture.tsx] [--force]
  astroshot react <fixture.tsx> -o <out.png> [options]
  astroshot react batch <manifest.yaml|json> [options]

Options:
  -o, --out <path>       Output PNG path
  --root <dir>           Package root for imports and aliases
  --config <path>        Fixture configuration path
  --width <px>           Override viewport width
  --height <px>          Override viewport height
  --headed               Show Chromium for debugging
  -h, --help             Show this help"#;

const INK_HELP: &str = r#"astroshot ink — deterministic Ink component screenshots

Usage:
  astroshot init ink [fixture.tsx] [--force]
  astroshot ink <fixture.tsx> -o <out.png> [options]
  astroshot ink batch <manifest.yaml|json> [options]

Options:
  -o, --out <path>       Output PNG path
  --cols <count>         Override terminal columns
  --rows <count>         Override terminal rows
  --scale <factor>       Override PNG device scale factor
  --out-dir <path>       Override batch output directory
  --headed               Show Chromium for debugging
  -h, --help             Show this help"#;

const PTY_HELP: &str = r#"astroshot pty — screenshots of arbitrary terminal programs

Usage:
  astroshot init pty [fixture.yaml] [--force]
  astroshot pty <fixture.yaml|json> -o <out.png> [options]

Options:
  -o, --out <path>       Output PNG path
  --cols <count>         Override terminal columns
  --rows <count>         Override terminal rows
  --scale <factor>       Override PNG device scale factor
  --headed               Show Chromium for debugging
  -h, --help             Show this help"#;

const INIT_HELP: &str = r#"astroshot init — generate a fixture template

Usage:
  astroshot init react [fixture.tsx] [--force]
  astroshot init ink [fixture.tsx] [--force]
  astroshot init pty [fixture.yaml] [--force]

Defaults:
  react.shot.tsx
  ink.shot.tsx
  pty.shot.yaml

Existing files are never replaced unless --force is passed."#;

pub fn help() -> &'static str {
    HELP
}

pub fn mode_help(mode: &str) -> &'static str {
    match mode {
        "react" => REACT_HELP,
        "ink" => INK_HELP,
        _ => PTY_HELP,
    }
}

pub fn init_help() -> &'static str {
    INIT_HELP
}

/// Stand-ins for engines that later waves port. Each takes the arguments the
/// TS engine bin would have received and returns its exit code; replace one
/// function (and keep the signature) when its CLI lands.
pub mod pending {
    fn not_available(command: &str) -> i32 {
        eprintln!("astroshot {command}: not yet available in the Rust build");
        2
    }

    /// `astroshot-review` (`astroshot_review::cli`).
    pub async fn review(_args: &[String]) -> i32 {
        not_available("review")
    }

    /// `astroshot-movie` (`movie_harness::cli`).
    pub async fn movie(args: &[String]) -> i32 {
        crate::movie_harness::cli::run_cli(args).await
    }

    /// `react-shot`: `args` already carry the `shot` prefix when the first
    /// argument was a fixture path.
    pub async fn react_shot(args: &[String]) -> i32 {
        crate::react_shot::cli::run(args).await
    }

    /// `tui-shot`: `mode` is `ink` or `pty`; for `pty`, `args` start with
    /// `pty`, as the TS dispatcher forwards them.
    pub async fn tui_shot(_mode: &str, args: &[String]) -> i32 {
        // One CLI serves both modes, as the TS `tui-shot` bin did: `pty` is
        // its own subcommand there.
        crate::tui_shot::cli::run(args).await
    }
}

fn is_help_word(value: &str) -> bool {
    matches!(value, "help" | "-h" | "--help")
}

fn is_ts_fixture(value: &str) -> bool {
    // /\.[cm]?tsx?$/
    let Some((_, ext)) = value.rsplit_once('.') else {
        return false;
    };
    let ext = ext.strip_prefix(['c', 'm']).unwrap_or(ext);
    matches!(ext, "ts" | "tsx")
}

/// `runEngine` for react/ink/pty: normalize arguments, then hand them to the
/// engine.
async fn run_engine(mode: &str, args: &[String]) -> i32 {
    if mode == "pty" {
        let mut forwarded = vec!["pty".to_string()];
        forwarded.extend_from_slice(args);
        return pending::tui_shot("pty", &forwarded).await;
    }
    let normalized: Vec<String> = if args.first().is_some_and(|first| is_ts_fixture(first)) {
        std::iter::once("shot".to_string())
            .chain(args.iter().cloned())
            .collect()
    } else {
        args.to_vec()
    };
    if mode == "react" {
        pending::react_shot(&normalized).await
    } else {
        pending::tui_shot("ink", &normalized).await
    }
}

/// `astroshot install-browser`: the Rust build drives a system Chrome over
/// CDP, so there is nothing to download; report what was found.
fn install_browser() -> i32 {
    match crate::browser::find_chrome() {
        Ok(path) => {
            println!("Chrome found: {}", path.display());
            0
        }
        Err(error) => {
            eprintln!("{error}");
            eprintln!("Install Chrome, or set ASTROSHOT_CHROME to a browser executable.");
            1
        }
    }
}

fn run_init(args: &[String]) -> anyhow::Result<i32> {
    if args.is_empty() || is_help_word(&args[0]) {
        println!("{INIT_HELP}");
        return Ok(if args.is_empty() { 1 } else { 0 });
    }
    let mode = args[0].clone();
    let rest = &args[1..];
    let force = rest.iter().any(|v| v == "--force" || v == "-f");
    if let Some(flag) = rest
        .iter()
        .find(|v| v.starts_with('-') && *v != "--force" && *v != "-f")
    {
        anyhow::bail!("Unknown init flag: {flag}");
    }
    let positionals: Vec<&String> = rest.iter().filter(|v| !v.starts_with('-')).collect();
    if positionals.len() > 1 {
        anyhow::bail!("init accepts at most one output path");
    }
    let result = write_fixture_template(WriteFixtureTemplateOptions {
        mode,
        output_path: positionals.first().map(|v| (*v).clone()),
        force,
        cwd: None,
    })?;
    println!(
        "Created {} fixture: {}",
        result.label,
        result.absolute_path.display()
    );
    Ok(0)
}

/// `astroshot review`: without explicit roots, watch the same folders as the
/// Astroshots app so both surfaces show one stream.
async fn run_review(args: &[String]) -> i32 {
    let wants_help = args.first().is_some_and(|v| v == "help")
        || args.iter().any(|v| v == "-h" || v == "--help");
    let mut forwarded: Vec<String> = if wants_help {
        vec!["--help".into()]
    } else {
        args.to_vec()
    };
    let has_roots =
        forwarded.iter().any(|v| v == "--root") || forwarded.iter().any(|v| !v.starts_with('-'));
    if !has_roots && !wants_help {
        let configuration = read_watch_configuration(&ReadWatchConfigurationOptions::default());
        if configuration.available && !configuration.roots.is_empty() {
            for root in &configuration.roots {
                forwarded.push("--root".into());
                forwarded.push(root.clone());
            }
            forwarded.push("--roots-source".into());
            forwarded.push("app".into());
        }
    }
    pending::review(&forwarded).await
}

/// Run the dispatcher. `args` is argv after the program name; returns the
/// process exit code.
pub async fn run(args: &[String]) -> i32 {
    let command = args.first().map(String::as_str);
    let rest = args.get(1..).unwrap_or(&[]);

    if matches!(command, Some("-v" | "--version")) {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    let Some(command) = command else {
        println!("{HELP}");
        return 1;
    };
    if is_help_word(command) {
        println!("{HELP}");
        return 0;
    }

    match command {
        "demo" => with_help_on_error(run_demo(rest, &mut |line| println!("{line}")), demo_help),
        "doctor" => with_help_on_error(
            run_doctor(rest, &mut |line| println!("{line}")),
            doctor_help,
        ),
        "install-browser" => install_browser(),
        "init" => match run_init(rest) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("{error}");
                1
            }
        },
        "react" | "ink" | "tui" | "pty" => {
            let mode = if command == "tui" { "ink" } else { command };
            if rest.is_empty() || is_help_word(&rest[0]) {
                println!("{}", mode_help(mode));
                return if rest.is_empty() { 1 } else { 0 };
            }
            run_engine(mode, rest).await
        }
        "review" | "tray" => run_review(rest).await,
        "movie" => pending::movie(rest).await,
        _ => {
            eprintln!("Unknown command: {command}");
            println!("{HELP}");
            1
        }
    }
}

fn with_help_on_error(result: anyhow::Result<i32>, help: fn() -> String) -> i32 {
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            eprintln!();
            eprintln!("{}", help());
            1
        }
    }
}

/// Subcommand implied by the executable name (the npm bins `astroshot-review`,
/// `astroshot-movie`, `react-shot`, `tui-shot`), or `None` for `astroshot`.
pub fn subcommand_for_program(program: &str) -> Option<&'static str> {
    let name = PathBuf::from(program)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())?;
    match name.as_str() {
        "astroshot-review" => Some("review"),
        "astroshot-movie" => Some("movie"),
        "react-shot" => Some("react"),
        "tui-shot" => Some("ink"),
        _ => None,
    }
}

/// Arguments for [`run`] from a full argv, applying argv[0] multi-call.
pub fn args_from_argv(argv: &[String]) -> Vec<String> {
    let program = argv.first().map(String::as_str).unwrap_or("astroshot");
    let rest = argv.get(1..).unwrap_or(&[]);
    // `tui-shot pty ...` is the PTY mode of the engine bin.
    if subcommand_for_program(program) == Some("ink") && rest.first().is_some_and(|v| v == "pty") {
        return std::iter::once("pty".to_string())
            .chain(rest[1..].iter().cloned())
            .collect();
    }
    match subcommand_for_program(program) {
        Some(sub) => std::iter::once(sub.to_string())
            .chain(rest.iter().cloned())
            .collect(),
        None => rest.to_vec(),
    }
}
