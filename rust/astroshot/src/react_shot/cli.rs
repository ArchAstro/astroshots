//! Port of `packages/react-shot/src/cli.ts`: the `react-shot` CLI (single
//! shot, batch manifests, `install-browser`).
//!
//! Divergences from the TS:
//! - [`run`] takes the argv after the program name and returns the exit code
//!   instead of reading `process.argv` / setting `process.exitCode`.
//! - `--version` prints the crate version (TS read the package's `package.json`).
//! - `install-browser` reports the Chrome that CDP will drive instead of
//!   running Playwright's installer (same behavior as `astroshot install-browser`).

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::Value;

use super::batch_paths::{resolve, resolve_batch_output_paths};
use super::shot::{close_shared_browser, take_shot};
use super::types::{BatchEntry, ShotRequest};

const HELP: &str = r#"react-shot - deterministic React component screenshots

Usage:
  react-shot shot <fixture.tsx> -o <out.png> [options]
  react-shot batch <manifest.yaml|json> [options]
  react-shot <fixture.tsx> -o <out.png> [options]
  react-shot install-browser [--with-deps]

Options:
  -o, --out <path>       Output PNG path
  --root <dir>           Package root for imports and aliases
  --config <path>        Config path; otherwise discovered from the fixture
  --width <px>           Override viewport width
  --height <px>          Override viewport height
  --headed               Show Chromium for debugging
  --with-deps            Install Chromium OS dependencies too
  -v, --version          Print the installed version
  -h, --help             Show this help

Run "react-shot install-browser" once before the first capture.
"#;

/// The `Flags` record. String flags stay `None` when absent.
#[derive(Debug, Default)]
struct Flags {
    help: bool,
    version: bool,
    headed: bool,
    with_deps: bool,
    out: Option<String>,
    root: Option<String>,
    config: Option<String>,
    width: Option<String>,
    height: Option<String>,
}

fn print_help() {
    // `console.log` of a template literal that already ends in a newline.
    println!("{HELP}");
}

fn required_value(args: &mut std::collections::VecDeque<String>, flag: &str) -> Result<String> {
    match args.pop_front() {
        Some(value) if !value.is_empty() && !value.starts_with('-') => Ok(value),
        _ => bail!("{flag} requires a value"),
    }
}

fn parse_args(argv: &[String]) -> Result<(Flags, Vec<String>)> {
    let mut args: std::collections::VecDeque<String> = argv.iter().cloned().collect();
    let mut flags = Flags::default();
    let mut positionals = Vec::new();
    while let Some(argument) = args.pop_front() {
        match argument.as_str() {
            "-h" | "--help" => flags.help = true,
            "-v" | "--version" => flags.version = true,
            "--headed" => flags.headed = true,
            "--with-deps" => flags.with_deps = true,
            "-o" | "--out" => flags.out = Some(required_value(&mut args, &argument)?),
            "--root" => flags.root = Some(required_value(&mut args, &argument)?),
            "--config" => flags.config = Some(required_value(&mut args, &argument)?),
            "--width" => flags.width = Some(required_value(&mut args, &argument)?),
            "--height" => flags.height = Some(required_value(&mut args, &argument)?),
            other if other.starts_with('-') => bail!("Unknown option: {other}"),
            _ => positionals.push(argument),
        }
    }
    Ok((flags, positionals))
}

/// JS `Number(text)` for the spellings a CLI flag can plausibly carry.
fn js_number(text: &str) -> f64 {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if trimmed.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).map_or(f64::NAN, |v| v as f64);
        }
    }
    match trimmed {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // f64::from_str also accepts "inf"/"nan"; JS does not.
    if trimmed
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
    {
        trimmed.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

fn valid_dimension(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && (1.0..=10_000.0).contains(&value)
}

fn dimension(raw: Option<&str>, name: &str) -> Result<Option<u32>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = js_number(raw);
    if !valid_dimension(value) {
        bail!("--{name} must be an integer between 1 and 10000");
    }
    Ok(Some(value as u32))
}

async fn capture_fixture(fixture: &str, flags: &Flags) -> Result<()> {
    let Some(output) = &flags.out else {
        bail!("A screenshot requires -o <out.png>");
    };
    let out_path = take_shot(&ShotRequest {
        fixture_path: fixture.to_string(),
        out_path: output.clone(),
        root: flags.root.clone(),
        config_path: flags.config.clone(),
        headed: Some(flags.headed),
        width: dimension(flags.width.as_deref(), "width")?,
        height: dimension(flags.height.as_deref(), "height")?,
    })
    .await?;
    println!("wrote {out_path}");
    Ok(())
}

fn manifest_relative(base: &Path, value: Option<&str>) -> Option<String> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| resolve(base, v).to_string_lossy().into_owned())
}

/// `path.relative(process.cwd(), target)` for absolute lexical paths.
fn relative_to_cwd(target: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let from: Vec<Component> = cwd.components().collect();
    let to_path = Path::new(target);
    let to: Vec<Component> = to_path.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn truthy_string<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// An entry's `width`/`height`: kept as a raw number so `takeShot`'s own
/// validation message applies when it is out of range.
fn entry_dimension(entry: &Value, key: &str) -> Option<f64> {
    entry.get(key).and_then(Value::as_f64)
}

async fn capture_batch(manifest_path: &str, flags: &Flags) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let absolute_manifest = resolve(&cwd, manifest_path);
    let display = absolute_manifest.to_string_lossy().into_owned();
    let raw = std::fs::read_to_string(&absolute_manifest).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("ENOENT: no such file or directory, open '{display}'")
        } else {
            anyhow::anyhow!("{error}")
        }
    })?;
    let manifest: Value = if display.ends_with(".json") {
        serde_json::from_str(&raw)?
    } else {
        serde_yaml_ng::from_str(&raw)?
    };
    let shots = match manifest.get("shots").and_then(Value::as_array) {
        Some(shots) if !shots.is_empty() => shots,
        _ => bail!("No shots listed in {display}"),
    };

    let base_directory = absolute_manifest
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    for entry in shots {
        if truthy_string(entry, "fixture").is_none() || truthy_string(entry, "out").is_none() {
            bail!("Each batch entry requires fixture and out");
        }
    }
    let entries: Vec<BatchEntry> = shots
        .iter()
        .map(|entry| BatchEntry {
            fixture: truthy_string(entry, "fixture")
                .unwrap_or_default()
                .to_string(),
            out: truthy_string(entry, "out").unwrap_or_default().to_string(),
            root: None,
            config: None,
            width: None,
            height: None,
        })
        .collect();
    let out_paths = resolve_batch_output_paths(&entries, &base_directory.to_string_lossy())?;
    let cli_width = dimension(flags.width.as_deref(), "width")?;
    let cli_height = dimension(flags.height.as_deref(), "height")?;
    let manifest_root = manifest.get("root").and_then(Value::as_str);
    let manifest_config = manifest.get("config").and_then(Value::as_str);
    let mut completed = 0;

    for (index, entry) in shots.iter().enumerate() {
        let fixture_path = resolve(&base_directory, &entries[index].fixture);
        let fixture_path = fixture_path.to_string_lossy().into_owned();
        let out_path = out_paths[index].clone();
        let root = match &flags.root {
            Some(root) => Some(resolve(&cwd, root).to_string_lossy().into_owned()),
            None => manifest_relative(
                &base_directory,
                truthy_string(entry, "root").or(manifest_root),
            ),
        };
        let config_path = match &flags.config {
            Some(config) => Some(resolve(&cwd, config).to_string_lossy().into_owned()),
            None => manifest_relative(
                &base_directory,
                truthy_string(entry, "config").or(manifest_config),
            ),
        };

        print!("shot {} ... ", relative_to_cwd(&fixture_path));
        let _ = std::io::stdout().flush();
        let entry_width = entry_dimension(entry, "width");
        let entry_height = entry_dimension(entry, "height");
        let width = pick_dimension(cli_width, entry_width, "width")?;
        let height = pick_dimension(cli_height, entry_height, "height")?;
        take_shot(&ShotRequest {
            fixture_path,
            out_path: out_path.clone(),
            root,
            config_path,
            headed: Some(flags.headed),
            width,
            height,
        })
        .await?;
        completed += 1;
        println!("wrote {}", relative_to_cwd(&out_path));
    }
    println!("done: {completed}/{} shots", shots.len());
    Ok(())
}

/// `cliValue ?? entryValue`, with an out-of-range manifest value reported the
/// way `takeShot` reports it.
fn pick_dimension(cli: Option<u32>, entry: Option<f64>, name: &str) -> Result<Option<u32>> {
    if cli.is_some() {
        return Ok(cli);
    }
    match entry {
        None => Ok(None),
        Some(value) if valid_dimension(value) => Ok(Some(value as u32)),
        Some(_) => bail!("{name} must be an integer between 1 and 10000"),
    }
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

fn is_ts_fixture(value: &str) -> bool {
    // /\.[cm]?tsx?$/
    let Some((_, ext)) = value.rsplit_once('.') else {
        return false;
    };
    let ext = ext.strip_prefix(['c', 'm']).unwrap_or(ext);
    matches!(ext, "ts" | "tsx")
}

async fn dispatch(flags: &Flags, positionals: &[String]) -> Result<()> {
    let first = positionals.first().map(String::as_str);
    let second = positionals.get(1).map(String::as_str);
    let result = match first {
        Some("install-browser") => install_browser(),
        Some("shot") => match second {
            None | Some("") => Err(anyhow::anyhow!("shot requires a fixture path")),
            Some(fixture) => capture_fixture(fixture, flags).await,
        },
        Some("batch") => match second {
            None | Some("") => Err(anyhow::anyhow!("batch requires a manifest path")),
            Some(manifest) => capture_batch(manifest, flags).await,
        },
        Some(fixture) if is_ts_fixture(fixture) => capture_fixture(fixture, flags).await,
        other => Err(anyhow::anyhow!(
            "Unknown command: {}",
            other.unwrap_or("undefined")
        )),
    };
    // `finally { await closeSharedBrowser() }`
    let closed = close_shared_browser().await;
    result?;
    closed
}

async fn main_inner(argv: &[String]) -> Result<i32> {
    if argv.is_empty() {
        print_help();
        return Ok(1);
    }
    let (flags, positionals) = parse_args(argv)?;
    if flags.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if flags.help || positionals.first().is_some_and(|p| p == "help") {
        print_help();
        return Ok(0);
    }
    let _ = flags.with_deps;
    dispatch(&flags, &positionals).await?;
    Ok(0)
}

/// Run the CLI with the arguments after the program name; returns the exit
/// code.
pub async fn run(argv: &[String]) -> i32 {
    match main_inner(argv).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
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
            "shot", "a.tsx", "-o", "out.png", "--width", "10", "--headed",
        ]))
        .unwrap();
        assert_eq!(positionals, ["shot", "a.tsx"]);
        assert_eq!(flags.out.as_deref(), Some("out.png"));
        assert_eq!(flags.width.as_deref(), Some("10"));
        assert!(flags.headed);
    }

    #[test]
    fn rejects_unknown_options_and_missing_values() {
        assert_eq!(
            parse_args(&args(&["--bogus"])).unwrap_err().to_string(),
            "Unknown option: --bogus"
        );
        assert_eq!(
            parse_args(&args(&["-o"])).unwrap_err().to_string(),
            "-o requires a value"
        );
        assert_eq!(
            parse_args(&args(&["--root", "--headed"]))
                .unwrap_err()
                .to_string(),
            "--root requires a value"
        );
    }

    #[test]
    fn validates_dimensions_like_number_is_integer() {
        assert_eq!(dimension(Some("1e3"), "width").unwrap(), Some(1000));
        assert_eq!(dimension(None, "width").unwrap(), None);
        for bad in ["0", "10001", "1.5", "abc", "inf"] {
            assert_eq!(
                dimension(Some(bad), "height").unwrap_err().to_string(),
                "--height must be an integer between 1 and 10000"
            );
        }
    }

    #[test]
    fn recognizes_ts_fixtures() {
        for yes in ["a.ts", "a.tsx", "a.mts", "a.cts", "a.mtsx"] {
            assert!(is_ts_fixture(yes), "{yes}");
        }
        for no in ["a.js", "a.png", "tsx", "a.xtsx"] {
            assert!(!is_ts_fixture(no), "{no}");
        }
    }

    #[test]
    fn relative_paths_walk_up_from_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let inside = cwd.join("a/b.png");
        assert_eq!(relative_to_cwd(&inside.to_string_lossy()), "a/b.png");
        let outside = cwd.parent().unwrap().join("x.png");
        assert_eq!(relative_to_cwd(&outside.to_string_lossy()), "../x.png");
    }
}
