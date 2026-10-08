//! Port of `packages/tui-shot/src/cli.ts`: the `tui-shot` CLI (Ink shots, PTY
//! shots, batch manifests, `install-browser`).
//!
//! Divergences from the TS:
//! - [`run`] takes the argv after the program name and returns the exit code
//!   instead of reading `process.argv` / setting `process.exitCode`.
//! - `install-browser` reports the Chrome that CDP would drive instead of
//!   running Playwright's installer (same behavior as `react_shot::cli`).
//!   Terminal shots are rasterized natively and need no browser.
//! - `--headed` is accepted and passed through; nothing shows a browser.
//! - A manifest that is not valid JSON/YAML reports the Rust parser's wording.

use std::collections::VecDeque;
use std::io::Write;

use anyhow::{Result, bail};

use super::react_shot::{js_number, relative_to_cwd};
use astroshot_engine::react_shot::batch::BatchProgress;
use astroshot_engine::tui_shot::batch::{TuiBatchOptions, assert_png_path, load_tui_batch};
use astroshot_engine::tui_shot::pty_shot::take_pty_shot;
use astroshot_engine::tui_shot::shot::{close_shared_browser, take_tui_shot};
use astroshot_engine::tui_shot::types::{PtyShotRequest, TuiShotRequest};

const HELP: &str = r#"tui-shot — deterministic PNG screenshots of terminal interfaces

Usage:
  tui-shot install-browser [--with-deps]
  tui-shot shot <fixture.tsx> -o <out.png> [options]
  tui-shot pty <fixture.yaml|json> -o <out.png> [options]
  tui-shot batch <manifest.yaml|json> [options]

Options:
  -o, --out <path>       Output PNG path
  --cols <count>         Override terminal columns
  --rows <count>         Override terminal rows
  --scale <factor>       Override PNG device scale factor
  --out-dir <path>       Override batch output directory
  --headed               Show Chromium while capturing
  --with-deps            Install Chromium system dependencies (Linux)
  -h, --help             Show this help
"#;

/// The `Flags` record. String flags stay `None` when absent.
#[derive(Debug, Default)]
struct Flags {
    help: bool,
    headed: bool,
    with_deps: bool,
    out: Option<String>,
    cols: Option<String>,
    rows: Option<String>,
    scale: Option<String>,
    out_dir: Option<String>,
}

fn help() {
    // `console.log` of a template literal that already ends in a newline.
    println!("{HELP}");
}

fn take_value(args: &mut VecDeque<String>, flag: &str) -> Result<String> {
    match args.pop_front() {
        Some(value) if !value.is_empty() && !value.starts_with('-') => Ok(value),
        _ => bail!("{flag} requires a value"),
    }
}

fn parse_args(argv: &[String]) -> Result<(Flags, Vec<String>)> {
    let mut args: VecDeque<String> = argv.iter().cloned().collect();
    let mut flags = Flags::default();
    let mut positionals = Vec::new();
    while let Some(value) = args.pop_front() {
        match value.as_str() {
            "-h" | "--help" => flags.help = true,
            "--headed" => flags.headed = true,
            "--with-deps" => flags.with_deps = true,
            "-o" | "--out" => flags.out = Some(take_value(&mut args, &value)?),
            "--cols" => flags.cols = Some(take_value(&mut args, &value)?),
            "--rows" => flags.rows = Some(take_value(&mut args, &value)?),
            "--scale" => flags.scale = Some(take_value(&mut args, &value)?),
            "--out-dir" => flags.out_dir = Some(take_value(&mut args, &value)?),
            other if other.starts_with('-') => bail!("Unknown flag: {other}"),
            _ => positionals.push(value),
        }
    }
    Ok((flags, positionals))
}

fn number_flag(raw: Option<&str>, key: &str, integer: bool, maximum: u32) -> Result<Option<f64>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = js_number(raw);
    if !value.is_finite()
        || value <= 0.0
        || value > f64::from(maximum)
        || (integer && value.fract() != 0.0)
    {
        let kind = if integer {
            "positive integer"
        } else {
            "positive number"
        };
        bail!("--{key} must be a {kind} no greater than {maximum}");
    }
    Ok(Some(value))
}

/// `captureOverrides`: validated `--cols`, `--rows`, `--scale`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CaptureOverrides {
    cols: Option<f64>,
    rows: Option<f64>,
    scale: Option<f64>,
}

fn capture_overrides(flags: &Flags) -> Result<CaptureOverrides> {
    Ok(CaptureOverrides {
        cols: number_flag(flags.cols.as_deref(), "cols", true, 1_000)?,
        rows: number_flag(flags.rows.as_deref(), "rows", true, 1_000)?,
        scale: number_flag(flags.scale.as_deref(), "scale", false, 4)?,
    })
}

fn install_browser() -> Result<()> {
    match astroshot_engine::browser::find_chrome() {
        Ok(path) => {
            println!("Chrome found: {}", path.display());
            Ok(())
        }
        Err(error) => {
            eprintln!("{error}");
            bail!("Install Chrome, or set ASTROSHOT_CHROME to a browser executable.")
        }
    }
}

async fn shot(fixture_path: &str, flags: &Flags) -> Result<()> {
    let Some(out_path) = &flags.out else {
        bail!("shot requires -o <out.png>");
    };
    assert_png_path(out_path)?;
    let overrides = capture_overrides(flags)?;
    let written = take_tui_shot(&TuiShotRequest {
        fixture_path: fixture_path.to_string(),
        out_path: out_path.clone(),
        headed: Some(flags.headed),
        cols: overrides.cols,
        rows: overrides.rows,
        scale: overrides.scale,
    })
    .await?;
    println!("wrote {written}");
    Ok(())
}

async fn pty(fixture_path: &str, flags: &Flags) -> Result<()> {
    let Some(out_path) = &flags.out else {
        bail!("pty requires -o <out.png>");
    };
    assert_png_path(out_path)?;
    let overrides = capture_overrides(flags)?;
    let written = take_pty_shot(&PtyShotRequest {
        fixture_path: fixture_path.to_string(),
        out_path: out_path.clone(),
        headed: Some(flags.headed),
        cols: overrides.cols,
        rows: overrides.rows,
        scale: overrides.scale,
    })
    .await?;
    println!("wrote {written}");
    Ok(())
}

async fn batch(manifest_path: &str, flags: &Flags) -> Result<()> {
    let batch = load_tui_batch(manifest_path, flags.out_dir.as_deref())?;
    let overrides = capture_overrides(flags)?;
    let report = batch
        .run(
            &TuiBatchOptions {
                cols: overrides.cols,
                rows: overrides.rows,
                scale: overrides.scale,
                headed: flags.headed,
            },
            &mut |progress| match progress {
                BatchProgress::Started { fixture_path } => {
                    print!("shot {} … ", relative_to_cwd(&fixture_path));
                    let _ = std::io::stdout().flush();
                }
                BatchProgress::Wrote { out_path } => println!("→ {}", relative_to_cwd(&out_path)),
            },
        )
        .await?;
    println!("done: {}/{} shots", report.completed, report.total);
    Ok(())
}

fn first_positional<'a>(positionals: &'a [String], message: &str) -> Result<&'a str> {
    match positionals.first() {
        Some(value) if !value.is_empty() => Ok(value),
        _ => bail!("{message}"),
    }
}

async fn main_inner(argv: &[String]) -> Result<i32> {
    let command = argv.first().map(String::as_str).unwrap_or("");
    if matches!(command, "" | "help" | "-h" | "--help") {
        help();
        return Ok(if command.is_empty() { 1 } else { 0 });
    }
    let (flags, positionals) = parse_args(&argv[1..])?;
    if flags.help {
        help();
        return Ok(0);
    }
    match command {
        "install-browser" => {
            if !positionals.is_empty() {
                bail!("install-browser does not accept arguments");
            }
            let _ = flags.with_deps;
            install_browser()?;
        }
        "shot" => {
            let fixture = first_positional(&positionals, "shot requires a fixture path")?;
            shot(fixture, &flags).await?;
        }
        "pty" => {
            let fixture = first_positional(&positionals, "pty requires a fixture path")?;
            pty(fixture, &flags).await?;
        }
        "batch" => {
            let manifest = first_positional(&positionals, "batch requires a manifest path")?;
            batch(manifest, &flags).await?;
        }
        other => bail!("Unknown command: {other}"),
    }
    Ok(0)
}

/// Run the CLI with the arguments after the program name; returns the exit
/// code.
pub async fn run(argv: &[String]) -> i32 {
    let code = match main_inner(argv).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    };
    // `.finally(closeSharedBrowser)`: wait for every queued shot.
    let _ = close_shared_browser().await;
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn parses_flags_and_positionals() {
        let (flags, positionals) = parse_args(&args(&[
            "a.tsx",
            "-o",
            "out.png",
            "--cols",
            "80",
            "--rows",
            "24",
            "--scale",
            "1.5",
            "--out-dir",
            "shots",
            "--headed",
            "--with-deps",
            "extra",
        ]))
        .unwrap();
        assert_eq!(positionals, ["a.tsx", "extra"]);
        assert_eq!(flags.out.as_deref(), Some("out.png"));
        assert_eq!(flags.cols.as_deref(), Some("80"));
        assert_eq!(flags.rows.as_deref(), Some("24"));
        assert_eq!(flags.scale.as_deref(), Some("1.5"));
        assert_eq!(flags.out_dir.as_deref(), Some("shots"));
        assert!(flags.headed && flags.with_deps && !flags.help);
        assert!(parse_args(&args(&["-h"])).unwrap().0.help);
        assert_eq!(
            parse_args(&args(&["--out", "b.png"]))
                .unwrap()
                .0
                .out
                .as_deref(),
            Some("b.png")
        );
    }

    #[test]
    fn rejects_unknown_flags_and_missing_values() {
        assert_eq!(
            parse_args(&args(&["--bogus"])).unwrap_err().to_string(),
            "Unknown flag: --bogus"
        );
        assert_eq!(
            parse_args(&args(&["-o"])).unwrap_err().to_string(),
            "-o requires a value"
        );
        assert_eq!(
            parse_args(&args(&["--cols", "--headed"]))
                .unwrap_err()
                .to_string(),
            "--cols requires a value"
        );
        assert_eq!(
            parse_args(&args(&["--out-dir", ""]))
                .unwrap_err()
                .to_string(),
            "--out-dir requires a value"
        );
    }

    #[test]
    fn number_flags_follow_js_number_and_report_the_limit() {
        let flags = |cols: &str, scale: &str| Flags {
            cols: Some(cols.to_string()),
            scale: Some(scale.to_string()),
            ..Flags::default()
        };
        assert_eq!(
            capture_overrides(&flags("1e2", "0.5")).unwrap(),
            CaptureOverrides {
                cols: Some(100.0),
                rows: None,
                scale: Some(0.5),
            }
        );
        assert_eq!(
            capture_overrides(&flags("1000", "4")).unwrap().cols,
            Some(1000.0)
        );
        for bad in ["0", "1001", "1.5", "abc", "inf", "NaN"] {
            assert_eq!(
                capture_overrides(&flags(bad, "1")).unwrap_err().to_string(),
                "--cols must be a positive integer no greater than 1000",
                "{bad}"
            );
        }
        for bad in ["0", "4.01", "x"] {
            assert_eq!(
                capture_overrides(&flags("10", bad))
                    .unwrap_err()
                    .to_string(),
                "--scale must be a positive number no greater than 4",
                "{bad}"
            );
        }
        let rows = Flags {
            rows: Some("2.5".to_string()),
            ..Flags::default()
        };
        assert_eq!(
            capture_overrides(&rows).unwrap_err().to_string(),
            "--rows must be a positive integer no greater than 1000"
        );
    }

    #[tokio::test]
    async fn usage_errors_exit_1_and_help_exits_0() {
        assert_eq!(run(&args(&[])).await, 1);
        assert_eq!(run(&args(&["help"])).await, 0);
        assert_eq!(run(&args(&["shot", "--help"])).await, 0);
        assert_eq!(run(&args(&["shot"])).await, 1);
        assert_eq!(run(&args(&["shot", "a.tsx"])).await, 1);
        assert_eq!(run(&args(&["pty", "a.yaml", "-o", "a.jpg"])).await, 1);
        assert_eq!(run(&args(&["install-browser", "extra"])).await, 1);
        assert_eq!(run(&args(&["nope"])).await, 1);
    }
}
