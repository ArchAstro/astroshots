//! Port of `packages/astroshot-review/src/cli.ts`: `astroshot review`.
//!
//! Argument parsing, help and the messages around the tray. The tray itself
//! is [`super::tray`]; nothing in this module draws.

use std::io::{self, IsTerminal};

use astroshot_review::{RootsSource, TrayError, TrayOptions, resolve_roots, run_tray};

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
    let resolved = resolve_roots(&cwd, &args.roots);
    for root in &resolved.missing {
        eprintln!("Ignoring missing folder: {root}");
    }
    let mut options = TrayOptions::new(resolved.existing);
    options.roots_source = args.roots_source;
    options.graphics = args.graphics;
    options.watch = args.watch;
    options.use_index = args.index;
    options.version = read_version();
    match run_tray(options).await {
        Ok(code) => code,
        Err(TrayError::NotATerminal) => {
            eprintln!("{NEEDS_TTY}");
            1
        }
        Err(TrayError::Terminal(error)) => {
            eprintln!("astroshot review: {error}");
            1
        }
    }
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
    fn version_is_the_crate_version() {
        assert_eq!(read_version(), env!("CARGO_PKG_VERSION"));
    }
}
