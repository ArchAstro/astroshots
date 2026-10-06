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
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use super::batch_paths::resolve_batch_output_paths;
use super::pty_shot::take_pty_shot;
use super::shot::{close_shared_browser, take_tui_shot};
use super::types::{BatchEntry, BatchManifest, PtyShotRequest, TuiShotRequest};
use crate::react_shot::batch_paths::resolve;
use crate::react_shot::cli::{js_number, relative_to_cwd};

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

fn assert_png_path(out_path: &str) -> Result<()> {
    // `path.extname(outPath).toLowerCase() !== ".png"`
    let is_png = Path::new(out_path)
        .extension()
        .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == "png");
    if !is_png {
        bail!("Output must use a .png extension: {out_path}");
    }
    Ok(())
}

fn install_browser() -> Result<()> {
    match crate::browser::find_chrome() {
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

fn truthy_string<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn parse_manifest(absolute: &str) -> Result<BatchManifest> {
    let raw = std::fs::read_to_string(absolute).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!("ENOENT: no such file or directory, open '{absolute}'")
        } else {
            anyhow!("{error}")
        }
    })?;
    let value: Value = if absolute.ends_with(".json") {
        serde_json::from_str(&raw)?
    } else {
        serde_yaml_ng::from_str(&raw)?
    };
    let Some(shots) = value.get("shots").and_then(Value::as_array) else {
        bail!("No shots listed in {absolute}");
    };
    let entries: Option<Vec<BatchEntry>> = shots
        .iter()
        .map(|entry| {
            Some(BatchEntry {
                fixture: truthy_string(entry, "fixture")?.to_string(),
                out: truthy_string(entry, "out")?.to_string(),
            })
        })
        .collect();
    match entries {
        Some(shots) if !shots.is_empty() => Ok(BatchManifest { shots }),
        _ => bail!("Every shot in {absolute} needs fixture and out paths"),
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

async fn batch(manifest_path: &str, flags: &Flags) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let absolute = resolve(&cwd, manifest_path);
    let manifest = parse_manifest(&display(&absolute))?;
    let base = absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    let out_dir = flags
        .out_dir
        .as_deref()
        .map(|out_dir| display(&resolve(&cwd, out_dir)));
    let out_paths =
        resolve_batch_output_paths(&manifest.shots, &display(&base), out_dir.as_deref())?;
    for out_path in &out_paths {
        assert_png_path(out_path)?;
    }
    let overrides = capture_overrides(flags)?;
    let mut completed = 0;
    for (index, entry) in manifest.shots.iter().enumerate() {
        let fixture_path = display(&resolve(&base, &entry.fixture));
        let out_path = &out_paths[index];
        print!("shot {} … ", relative_to_cwd(&fixture_path));
        let _ = std::io::stdout().flush();
        take_tui_shot(&TuiShotRequest {
            fixture_path,
            out_path: out_path.clone(),
            headed: Some(flags.headed),
            cols: overrides.cols,
            rows: overrides.rows,
            scale: overrides.scale,
        })
        .await?;
        println!("→ {}", relative_to_cwd(out_path));
        completed += 1;
    }
    println!("done: {completed}/{} shots", manifest.shots.len());
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

    #[test]
    fn output_paths_need_a_png_extension_in_any_case() {
        for ok in ["a.png", "dir/a.PNG", "a.b.Png"] {
            assert!(assert_png_path(ok).is_ok(), "{ok}");
        }
        for bad in ["a.jpg", "png", ".png", "a.png.txt", "a"] {
            assert_eq!(
                assert_png_path(bad).unwrap_err().to_string(),
                format!("Output must use a .png extension: {bad}")
            );
        }
    }

    #[test]
    fn manifests_need_a_non_empty_shots_list_with_fixture_and_out() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            path.to_string_lossy().into_owned()
        };

        let json = write(
            "ok.json",
            r#"{"shots":[{"fixture":"a.tsx","out":"a.png"}]}"#,
        );
        assert_eq!(
            parse_manifest(&json).unwrap().shots,
            [BatchEntry {
                fixture: "a.tsx".to_string(),
                out: "a.png".to_string(),
            }]
        );
        let yaml = write("ok.yaml", "shots:\n  - fixture: b.tsx\n    out: b.png\n");
        assert_eq!(parse_manifest(&yaml).unwrap().shots[0].fixture, "b.tsx");

        for (name, body) in [
            ("null.json", "null"),
            ("list.json", "[]"),
            ("none.json", "{}"),
            ("object.json", r#"{"shots":{}}"#),
        ] {
            let path = write(name, body);
            assert_eq!(
                parse_manifest(&path).unwrap_err().to_string(),
                format!("No shots listed in {path}")
            );
        }
        for (name, body) in [
            ("empty.json", r#"{"shots":[]}"#),
            ("nullentry.json", r#"{"shots":[null]}"#),
            ("noout.json", r#"{"shots":[{"fixture":"a.tsx"}]}"#),
            ("blank.json", r#"{"shots":[{"fixture":"","out":"a.png"}]}"#),
            ("typed.json", r#"{"shots":[{"fixture":1,"out":"a.png"}]}"#),
        ] {
            let path = write(name, body);
            assert_eq!(
                parse_manifest(&path).unwrap_err().to_string(),
                format!("Every shot in {path} needs fixture and out paths")
            );
        }

        let missing = dir.path().join("missing.json");
        assert_eq!(
            parse_manifest(&missing.to_string_lossy())
                .unwrap_err()
                .to_string(),
            format!(
                "ENOENT: no such file or directory, open '{}'",
                missing.display()
            )
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
