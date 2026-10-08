//! Port of `packages/astroshot/bin/astroshot.mjs`: the top-level `astroshot`
//! dispatcher. The TS bin spawned `node` on each engine's own bin script;
//! here every subcommand is an in-process function.

use std::path::PathBuf;

use crate::bin::demo::{demo_help, run_demo};
use crate::bin::doctor::{doctor_help, run_doctor};
use crate::bin::templates::{WriteFixtureTemplateOptions, write_fixture_template};
use astroshot_review::mac_preferences::{ReadWatchConfigurationOptions, read_watch_configuration};

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
        // One CLI serves both terminal modes, as the TS `tui-shot` bin did:
        // `pty` is its own subcommand there.
        return crate::cli::tui_shot::run(&forwarded).await;
    }
    let normalized: Vec<String> = if args.first().is_some_and(|first| is_ts_fixture(first)) {
        std::iter::once("shot".to_string())
            .chain(args.iter().cloned())
            .collect()
    } else {
        args.to_vec()
    };
    if mode == "react" {
        crate::cli::react_shot::run(&normalized).await
    } else {
        crate::cli::tui_shot::run(&normalized).await
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
    crate::cli::review::run(&forwarded).await
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
        "install-browser" => {
            // `runEngine("react", ["install-browser", ...arguments_])`: the
            // react-shot CLI owns the flags, help and the browser check.
            let forwarded: Vec<String> = std::iter::once(command.to_string())
                .chain(rest.iter().cloned())
                .collect();
            crate::cli::react_shot::run(&forwarded).await
        }
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
        "movie" => crate::cli::movie::run_cli(rest).await,
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

/// The npm bin a program name stands for. Each one ran its own package's
/// `cli.ts` with the raw arguments; none of them went through the unified
/// `astroshot` layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageBin {
    /// `astroshot-review` (`packages/astroshot-review/bin/astroshot-review.mjs`).
    Review,
    /// `astroshot-movie` (`packages/movie-harness/bin/astroshot-movie.mjs`).
    Movie,
    /// `react-shot` (`packages/react-shot/bin/react-shot.mjs`).
    ReactShot,
    /// `tui-shot` (`packages/tui-shot/bin/tui-shot.mjs`).
    TuiShot,
}

/// Package bin implied by the executable name, or `None` for `astroshot`.
pub fn package_bin_for_program(program: &str) -> Option<PackageBin> {
    let name = PathBuf::from(program)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())?;
    match name.as_str() {
        "astroshot-review" => Some(PackageBin::Review),
        "astroshot-movie" => Some(PackageBin::Movie),
        "react-shot" => Some(PackageBin::ReactShot),
        "tui-shot" => Some(PackageBin::TuiShot),
        _ => None,
    }
}

/// Whether this argv runs the review tray (`astroshot review|tray` or the
/// `astroshot-review` bin).
pub fn is_review_invocation(argv: &[String]) -> bool {
    let program = argv.first().map(String::as_str).unwrap_or("astroshot");
    match package_bin_for_program(program) {
        Some(bin) => bin == PackageBin::Review,
        None => matches!(argv.get(1).map(String::as_str), Some("review" | "tray")),
    }
}

/// Run a full argv (program name first). A package bin's name hands the raw
/// arguments to that package's CLI; `astroshot` runs the dispatcher.
pub async fn run_argv(argv: &[String]) -> i32 {
    let program = argv.first().map(String::as_str).unwrap_or("astroshot");
    let rest = argv.get(1..).unwrap_or(&[]);
    match package_bin_for_program(program) {
        Some(PackageBin::Review) => crate::cli::review::run(rest).await,
        Some(PackageBin::Movie) => crate::cli::movie::run_cli(rest).await,
        Some(PackageBin::ReactShot) => crate::cli::react_shot::run(rest).await,
        Some(PackageBin::TuiShot) => crate::cli::tui_shot::run(rest).await,
        None => run(rest).await,
    }
}
