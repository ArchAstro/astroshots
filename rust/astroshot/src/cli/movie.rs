//! Port of `packages/movie-harness/src/cli.ts`: the `astroshot movie` CLI
//! (`astroshot-movie` npm bin).
//!
//! Divergences from the TS:
//! - `--status` on `stop` / `run` is validated (`running|pass|fail|idle`).
//!   The TS cast any string through to the manifest; here an unknown value
//!   fails with the message `finalize` already uses.
//! - `--size` components must fit in `u32`; a larger one fails as a malformed
//!   `--size`.
//! - `list-windows` prints the Swift tool's JSON as parsed (as the TS did),
//!   through [`list_desktop_windows_json`], not the typed window structs.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::json;

use super::react_shot::js_number;
use super::source_help::{
    SOURCE_DECISION_TABLE, format_source_catalog, recommend_source, source_hint_for_error,
};
use astroshot_engine::movie_harness::paths::{resolve_lexically, resolve_root};
use astroshot_engine::movie_harness::sink::finalize_manifest;
use astroshot_engine::movie_harness::sources::browser::{
    BrowserMovieSessionOptions, record_browser_movie,
};
use astroshot_engine::movie_harness::sources::desktop_macos::{
    CheckScreenAccessOptions, DesktopError, DesktopMatchFlags, DesktopWindowMovieOptions,
    ScreenAccessReport, assert_desktop_toolchain, check_screen_recording_access,
    desktop_match_from_flags, list_desktop_windows_json, open_screen_recording_settings,
    record_desktop_window_movie, resolve_enable_app_name,
};
use astroshot_engine::movie_harness::sources::frames_store::{
    StartFrameSessionOptions, load_frame_session, mark_frame_session, push_frame_to_session,
    record_frames_demo, start_frame_session, stop_frame_session,
};
use astroshot_engine::movie_harness::sources::pty::{
    PtyMovieSessionOptions, record_pty_movie, record_truecolor_demo_movie,
};
use astroshot_engine::movie_harness::types::{
    BrowserMovieOptions, ManifestStatus, MovieFormat, MovieSessionOptions, MovieSourceKind, Size,
};

const USAGE_HEAD: &str = "astroshot movie — universal movie harness → .astroshot/ poster + video";

const USAGE_BODY: &str = r#"Commands
--------
  which-source [intent…]     Recommend a --source for an agent/human intent
  list-windows               List macOS windows (id, bundle, title) for desktop.window
  check-screen-access        Detect Screen Recording TCC (optional --request prompt)
  open-screen-settings       Open System Settings → Screen Recording
  start / push-frame / mark / stop
                             Multi-process frames session
  run                        One-shot recorder (browser | pty | desktop.window | frames | …)
  finalize                   Set manifest status pass|fail|idle

run --source …
--------------
  browser         --url URL | --script path.mjs [--headed] [--settle-ms N]
  pty             --fixture path.yaml|json   # truecolor TUI — preferred for terminals
  pty-demo        [--color 7c5cff]           # truecolor smoke, no program
  desktop.window  --bundle-id ID | --window-id N | --title-regex RE | --owner NAME | --pid N
                  [--duration-ms 3000] [--fps 10] [--cursor] [--allow-blank]
  frames          [--demo-frames N]          # or use start/push-frame/stop

Common options
--------------
  --feature NAME   kebab-case feature dir under .astroshot/ (required for capture)
  --slug SLUG      kebab-case movie slug (required for capture)
  --root DIR       worktree root (default: git root or cwd)
  --run-id ID      stable run identity (share across movies in one journey)
  --title TEXT     human title
  --description T  what the movie proves
  --size WxH       encode size (desktop defaults to window pixels)
  --fps N          default 15 (desktop default 10)
  --format webm|mp4
  --status running|pass|fail|idle
  --session ID     frames session id (default: latest)
  -h, --help

Agent notes
-----------
  • Prefer the decision table over improvising OS screen recording for web/TUI.
  • Always land poster+video under .astroshot/ so Astroshots streams posters.
  • desktop.window uses macOS screencapture (already on the system) + Swift window list
    shipped in this package — no extra binary download. Needs Screen Recording TCC.
  • For full catalog: astroshot movie help-sources

Examples
--------
  astroshot movie which-source "record a ratatui TUI with truecolor"
  astroshot movie run --source browser --feature web --slug home --url https://example.com
  astroshot movie run --source pty --feature tui --slug flow --fixture ./flow.pty.yaml
  astroshot movie run --source desktop.window --feature app --slug onboard \
    --bundle-id com.example.App --duration-ms 4000
  astroshot movie list-windows
"#;

fn usage() -> String {
    format!("{USAGE_HEAD}\n\n{SOURCE_DECISION_TABLE}\n\n{USAGE_BODY}")
}

/// One `Args` value: `true` for a bare flag, or the string that followed it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ArgValue {
    Flag,
    Text(String),
}

/// The TS `Args` record (`Record<string, string | boolean>`).
#[derive(Debug, Default, PartialEq, Eq)]
struct Args(HashMap<String, ArgValue>);

impl Args {
    /// `req(args, key)`: a non-empty string value.
    fn req(&self, key: &str) -> Result<&str> {
        match self.0.get(key) {
            Some(ArgValue::Text(value)) if !value.is_empty() => Ok(value),
            _ => bail!("--{key} is required"),
        }
    }

    /// `opt(args, key)`: the string value, empty included.
    fn opt(&self, key: &str) -> Option<&str> {
        match self.0.get(key) {
            Some(ArgValue::Text(value)) => Some(value),
            _ => None,
        }
    }

    /// `opt(args, key)` used where the TS tests truthiness (`!value`,
    /// `a || b`, `x ? … : …`): an empty string counts as missing.
    fn opt_truthy(&self, key: &str) -> Option<&str> {
        self.opt(key).filter(|value| !value.is_empty())
    }

    fn opt_string(&self, key: &str) -> Option<String> {
        self.opt(key).map(str::to_string)
    }

    /// `Boolean(args[key])`.
    fn truthy(&self, key: &str) -> bool {
        match self.0.get(key) {
            Some(ArgValue::Flag) => true,
            Some(ArgValue::Text(value)) => !value.is_empty(),
            None => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    command: String,
    args: Args,
    rest: Vec<String>,
}

fn parse_args(argv: &[String]) -> Parsed {
    let first = argv.first().map(String::as_str);
    let Some(command) = first.filter(|first| !matches!(*first, "-h" | "--help")) else {
        return Parsed {
            command: "help".to_string(),
            args: Args::default(),
            rest: Vec::new(),
        };
    };
    let mut args = Args::default();
    let mut rest = Vec::new();
    let mut i = 1;
    while i < argv.len() {
        let token = &argv[i];
        i += 1;
        if token == "-h" || token == "--help" {
            args.0.insert("help".to_string(), ArgValue::Flag);
            continue;
        }
        let Some(key) = token.strip_prefix("--") else {
            rest.push(token.clone());
            continue;
        };
        match argv.get(i) {
            Some(next) if !next.starts_with("--") => {
                args.0.insert(key.to_string(), ArgValue::Text(next.clone()));
                i += 1;
            }
            _ => {
                args.0.insert(key.to_string(), ArgValue::Flag);
            }
        }
    }
    Parsed {
        command: command.to_string(),
        args,
        rest,
    }
}

/// `/^(\d+)x(\d+)$/`.
fn parse_size(value: Option<&str>) -> Result<Option<Size>> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let digits = |text: &str| -> Option<u32> {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.parse().ok()
    };
    let size = value
        .split_once('x')
        .and_then(|(width, height)| Some((digits(width)?, digits(height)?)));
    match size {
        Some((width, height)) => Ok(Some(Size { width, height })),
        None => bail!("--size must be WxH, got {value}"),
    }
}

fn parse_format(value: Option<&str>) -> Result<Option<MovieFormat>> {
    match value {
        None | Some("") => Ok(None),
        Some("webm") => Ok(Some(MovieFormat::Webm)),
        Some("mp4") => Ok(Some(MovieFormat::Mp4)),
        Some(_) => bail!("--format must be webm|mp4"),
    }
}

fn parse_fps(value: Option<&str>) -> Result<Option<f64>> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let n = js_number(value);
    if !(n > 0.0 && n <= 60.0) {
        bail!("--fps must be in (0, 60]");
    }
    Ok(Some(n))
}

fn status_from(value: &str) -> Result<ManifestStatus> {
    match value {
        "pass" => Ok(ManifestStatus::Pass),
        "fail" => Ok(ManifestStatus::Fail),
        "idle" => Ok(ManifestStatus::Idle),
        "running" => Ok(ManifestStatus::Running),
        _ => bail!("--status must be pass|fail|idle|running"),
    }
}

/// `opt(args, "status") ?? "running"`.
fn parse_status(args: &Args) -> Result<ManifestStatus> {
    status_from(args.opt("status").unwrap_or("running"))
}

/// `JSON.stringify(value, null, 2)` plus the newline every command appends.
fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// `opt(args, "root") ?? resolveRoot()`.
fn root_or_default(args: &Args) -> String {
    match args.opt("root") {
        Some(root) => root.to_string(),
        None => resolve_root(None),
    }
}

/// The multi-line "how to grant Screen Recording" message. It names the
/// commands of this CLI, which the engine's errors do not.
fn format_screen_recording_denied_help(report: Option<&ScreenAccessReport>) -> String {
    let app = report.map_or_else(|| resolve_enable_app_name(None), |r| r.enable_app.clone());
    let lines = [
        "Screen Recording permission is required for --source desktop.window.".to_string(),
        String::new(),
        "Fix:".to_string(),
        "  1. Open System Settings → Privacy & Security → Screen Recording".to_string(),
        "     (or: astroshot movie open-screen-settings)".to_string(),
        format!("  2. Enable \"{app}\""),
        format!("  3. Quit and reopen {app} completely (TCC applies on next launch)"),
        "  4. Re-run: astroshot movie check-screen-access".to_string(),
        String::new(),
        "Note: macOS may not always show an automatic prompt; the Settings toggle is the reliable path."
            .to_string(),
        "browser / pty sources do not need this permission.".to_string(),
    ];
    lines.join("\n")
}

/// The message to print for an error from the engine: typed desktop errors
/// get the command hints that used to be part of their text.
fn describe_error(error: &anyhow::Error) -> String {
    match error.downcast_ref::<DesktopError>() {
        Some(DesktopError::ScreenAccessDenied { report }) => {
            format_screen_recording_denied_help(report.as_ref())
        }
        Some(
            typed @ (DesktopError::CaptureFailed { access, .. }
            | DesktopError::EmptyCapture { access, .. }),
        ) => format!(
            "{typed}\n{}",
            format_screen_recording_denied_help(access.as_ref())
        ),
        Some(typed @ DesktopError::NoWindowMatched { sample, .. }) => format!(
            "{typed}\nRun: astroshot movie list-windows\nSample windows:\n{}",
            if sample.is_empty() {
                "  (none)"
            } else {
                sample
            }
        ),
        None => error.to_string(),
    }
}

/// Run the movie CLI with the arguments after the program name; returns the
/// exit code.
pub async fn run_cli(argv: &[String]) -> i32 {
    match dispatch(argv).await {
        Ok(code) => code,
        Err(error) => {
            let message = describe_error(&error);
            eprintln!("error: {message}");
            // /which-source|decision table|list-windows/i
            let lower = message.to_ascii_lowercase();
            if !["which-source", "decision table", "list-windows"]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                eprintln!("hint: {}", source_hint_for_error(None));
            }
            1
        }
    }
}

async fn dispatch(argv: &[String]) -> Result<i32> {
    let Parsed {
        command,
        args,
        rest,
    } = parse_args(argv);
    if command == "help" || args.truthy("help") {
        println!("{}", usage());
        return Ok(0);
    }

    match command.as_str() {
        "which-source" => cmd_which_source(&rest, &args),
        "help-sources" => {
            println!("{}", format_source_catalog());
            Ok(0)
        }
        "list-windows" => cmd_list_windows(),
        "check-screen-access" => cmd_check_screen_access(&args),
        "open-screen-settings" => cmd_open_screen_settings(),
        "start" => cmd_start(&args),
        "push-frame" => cmd_push_frame(&args),
        "mark" => cmd_mark(&args),
        "stop" => cmd_stop(&args).await,
        "run" => cmd_run(&args).await,
        "finalize" => cmd_finalize(&args),
        _ => Err(anyhow!("unknown command: {command}\n\n{}", usage())),
    }
}

fn cmd_which_source(rest: &[String], args: &Args) -> Result<i32> {
    let joined = rest.join(" ");
    let intent = Some(joined.as_str())
        .filter(|joined| !joined.is_empty())
        .or_else(|| args.opt_truthy("intent"))
        .or_else(|| args.opt_truthy("for"))
        .unwrap_or("");
    if intent.is_empty() {
        print!(
            "{SOURCE_DECISION_TABLE}\n\n\
             Pass free text, e.g.:\n\
             \x20 astroshot movie which-source \"native SwiftUI onboarding window\"\n\
             \x20 astroshot movie which-source \"ratatui truecolor dashboard\"\n"
        );
        return Ok(0);
    }
    let advice = recommend_source(intent);
    print_json(&json!({
        "intent": intent,
        "recommended": advice.source.as_str(),
        "reason": advice.reason,
    }))?;
    Ok(0)
}

fn cmd_list_windows() -> Result<i32> {
    assert_desktop_toolchain()?;
    let windows = list_desktop_windows_json()?;
    print_json(&windows)?;
    eprintln!(
        "# {} windows — match with --window-id / --bundle-id / --title-regex / --owner / --pid",
        windows.len()
    );
    Ok(0)
}

fn cmd_check_screen_access(args: &Args) -> Result<i32> {
    assert_desktop_toolchain()?;
    let report = check_screen_recording_access(CheckScreenAccessOptions {
        request: args.truthy("request"),
    })?;
    println!("{}", report.to_json_pretty());
    if !report.granted {
        eprint!(
            "\nScreen Recording: DENIED (enable \"{app}\")\n\
             \x20 → {hint}\n\
             \x20 → or: astroshot movie open-screen-settings\n\
             \x20 → then quit & reopen {app}, re-run check-screen-access\n",
            app = report.enable_app,
            hint = report.settings_hint,
        );
        if args.truthy("open-settings") || args.truthy("open") {
            open_screen_recording_settings()?;
        }
        return Ok(2);
    }
    eprintln!(
        "# Screen Recording preflight: OK — still enable \"{}\" in Settings if captures fail",
        report.enable_app
    );
    Ok(0)
}

fn cmd_open_screen_settings() -> Result<i32> {
    assert_desktop_toolchain()?;
    if !open_screen_recording_settings()? {
        eprintln!(
            "error: could not open System Settings; open Privacy & Security → Screen Recording manually"
        );
        return Ok(1);
    }
    eprint!(
        "Opened System Settings (Screen Recording). Enable the app that launched this command, quit it fully, reopen, then:\n\
         \x20 astroshot movie check-screen-access\n"
    );
    Ok(0)
}

fn cmd_start(args: &Args) -> Result<i32> {
    let state = start_frame_session(StartFrameSessionOptions {
        feature: args.req("feature")?.to_string(),
        slug: args.req("slug")?.to_string(),
        root: args.opt_string("root"),
        run_id: args.opt_string("run-id"),
        title: args.opt_string("title"),
        description: args.opt_string("description"),
        size: parse_size(args.opt("size"))?,
        fps: parse_fps(args.opt("fps"))?,
        format: parse_format(args.opt("format"))?,
    })?;
    print_json(&json!({
        "id": state.id,
        "feature": state.feature,
        "slug": state.slug,
        "runId": state.run_id,
        "frameDir": state.frame_dir,
    }))?;
    Ok(0)
}

fn cmd_push_frame(args: &Args) -> Result<i32> {
    let root = root_or_default(args);
    let state = load_frame_session(&root, args.req("feature")?, args.opt("session"))?;
    let file = args.req("file")?;
    let absolute = resolve_lexically(Path::new(file));
    let next = push_frame_to_session(&state, &absolute.to_string_lossy())?;
    print_json(&json!({ "id": next.id, "frameCount": next.frame_count }))?;
    Ok(0)
}

fn cmd_mark(args: &Args) -> Result<i32> {
    let root = root_or_default(args);
    let state = load_frame_session(&root, args.req("feature")?, args.opt("session"))?;
    let next = mark_frame_session(&state, args.req("slug")?, args.opt("note"))?;
    print_json(&json!({ "id": next.id, "chapters": next.chapters }))?;
    Ok(0)
}

async fn cmd_stop(args: &Args) -> Result<i32> {
    let root = root_or_default(args);
    let state = load_frame_session(&root, args.req("feature")?, args.opt("session"))?;
    let status = parse_status(args)?;
    let artifact = stop_frame_session(&state, Some(status)).await?;
    print_json(&artifact)?;
    Ok(0)
}

async fn cmd_run(args: &Args) -> Result<i32> {
    let source = args.req("source")?;
    let feature = args.req("feature")?.to_string();
    let slug = args.req("slug")?.to_string();
    let root = args.opt_string("root");
    let run_id = args.opt_string("run-id");
    let title = args.opt_string("title");
    let description = args.opt_string("description");
    let size = parse_size(args.opt("size"))?;
    let fps = parse_fps(args.opt("fps"))?;
    let format = parse_format(args.opt("format"))?;
    let status = parse_status(args)?;

    // `{ ...common }` as session options for the given source.
    let session = |source: MovieSourceKind| MovieSessionOptions {
        feature: feature.clone(),
        slug: slug.clone(),
        root: root.clone(),
        run_id: run_id.clone(),
        title: title.clone(),
        description: description.clone(),
        size,
        fps,
        format,
        status: Some(status),
        source,
    };

    match source {
        "frames" => {
            let count = js_number(args.opt("demo-frames").unwrap_or("8"));
            let artifact = record_frames_demo(
                StartFrameSessionOptions {
                    feature: feature.clone(),
                    slug: slug.clone(),
                    root: root.clone(),
                    run_id: run_id.clone(),
                    title: title.clone(),
                    description: description.clone(),
                    size,
                    fps,
                    format,
                },
                count,
                status,
            )
            .await?;
            print_json(&artifact)?;
            Ok(0)
        }
        "browser" => {
            // `Number(...)` of a non-numeric value is NaN, which the browser
            // source treats like no settle time.
            let settle_ms = args
                .opt_truthy("settle-ms")
                .map(js_number)
                .filter(|ms| ms.is_finite() && *ms > 0.0)
                .map(|ms| ms as u64);
            let artifact = record_browser_movie(BrowserMovieSessionOptions {
                session: session(MovieSourceKind::Browser),
                browser: BrowserMovieOptions {
                    url: args.opt_string("url"),
                    script_path: args.opt_string("script"),
                    headed: Some(args.truthy("headed")),
                    settle_ms,
                },
            })
            .await?;
            print_json(&artifact)?;
            Ok(0)
        }
        "pty" => {
            let artifact = record_pty_movie(PtyMovieSessionOptions {
                session: session(MovieSourceKind::Pty),
                fixture_path: args.req("fixture")?.to_string(),
            })
            .await?;
            print_json(&artifact)?;
            Ok(0)
        }
        "pty-demo" => {
            let artifact =
                record_truecolor_demo_movie(session(MovieSourceKind::Pty), args.opt("color"))
                    .await?;
            print_json(&artifact)?;
            Ok(0)
        }
        "desktop.window" => {
            assert_desktop_toolchain()?;
            let r#match = desktop_match_from_flags(&DesktopMatchFlags {
                window_id: args.opt_string("window-id"),
                bundle_id: args.opt_string("bundle-id"),
                title_regex: args.opt_string("title-regex"),
                owner: args.opt_string("owner"),
                pid: args.opt_string("pid"),
                pick: args.opt_string("pick"),
            })?;
            let duration_ms = args.opt_truthy("duration-ms").map_or(3_000.0, js_number);
            let artifact = record_desktop_window_movie(DesktopWindowMovieOptions {
                feature,
                slug,
                root,
                run_id,
                title,
                description,
                size,
                fps: Some(fps.unwrap_or(10.0)),
                format,
                status: Some(status),
                r#match,
                duration_ms: Some(duration_ms),
                cursor: args.truthy("cursor"),
                allow_blank: args.truthy("allow-blank"),
            })
            .await?;
            print_json(&artifact)?;
            Ok(0)
        }
        "desktop.display" | "desktop.region" => bail!(
            "{source} is not implemented yet.\n\
             Use --source desktop.window for a single app, or --source frames.\n\
             {}",
            source_hint_for_error(Some("desktop.window"))
        ),
        _ => bail!(
            "unknown --source {}\n{SOURCE_DECISION_TABLE}",
            serde_json::to_string(source)?
        ),
    }
}

fn cmd_finalize(args: &Args) -> Result<i32> {
    let root = resolve_root(args.opt("root"));
    let feature = args.req("feature")?;
    let run_id = args.req("run-id")?;
    let status = args.req("status")?;
    finalize_manifest(&root, feature, run_id, status_from(status)?)?;
    print_json(&json!({ "feature": feature, "runId": run_id, "status": status }))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use astroshot_engine::movie_harness::png::encode_solid_png;

    fn argv(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    fn text(value: &str) -> ArgValue {
        ArgValue::Text(value.to_string())
    }

    #[test]
    fn no_arguments_or_a_leading_help_flag_is_the_help_command() {
        for input in [
            &[][..],
            &["-h"][..],
            &["--help"][..],
            &["--help", "run"][..],
        ] {
            let parsed = parse_args(&argv(input));
            assert_eq!(parsed.command, "help", "{input:?}");
            assert!(parsed.args.0.is_empty() && parsed.rest.is_empty());
        }
    }

    #[test]
    fn flags_take_the_next_token_unless_it_starts_with_two_dashes() {
        let parsed = parse_args(&argv(&[
            "run",
            "free",
            "--feature",
            "web",
            "--headed",
            "--slug",
            "-h",
            "--cursor",
            "--url",
            "",
            "text",
            "-x",
            "--last",
        ]));
        assert_eq!(parsed.command, "run");
        assert_eq!(parsed.rest, ["free", "text", "-x"]);
        assert_eq!(parsed.args.0.get("feature"), Some(&text("web")));
        assert_eq!(parsed.args.0.get("headed"), Some(&ArgValue::Flag));
        // A single-dash token is a value, even `-h`.
        assert_eq!(parsed.args.0.get("slug"), Some(&text("-h")));
        assert_eq!(parsed.args.0.get("cursor"), Some(&ArgValue::Flag));
        assert_eq!(parsed.args.0.get("url"), Some(&text("")));
        assert_eq!(parsed.args.0.get("last"), Some(&ArgValue::Flag));
        assert_eq!(parsed.args.0.get("help"), None);

        let help = parse_args(&argv(&["run", "--feature", "--help", "-h"]));
        assert_eq!(help.args.0.get("feature"), Some(&ArgValue::Flag));
        assert!(help.args.truthy("help"));
        // The last occurrence of a flag wins.
        let twice = parse_args(&argv(&["run", "--slug", "a", "--slug", "b"]));
        assert_eq!(twice.args.opt("slug"), Some("b"));
    }

    #[test]
    fn req_needs_a_non_empty_string_and_opt_keeps_empty_strings() {
        let parsed = parse_args(&argv(&["run", "--a", "x", "--b", "--c", ""]));
        let args = parsed.args;
        assert_eq!(args.req("a").unwrap(), "x");
        for key in ["b", "c", "missing"] {
            assert_eq!(
                args.req(key).unwrap_err().to_string(),
                format!("--{key} is required")
            );
        }
        assert_eq!(args.opt("a"), Some("x"));
        assert_eq!(args.opt("b"), None);
        assert_eq!(args.opt("c"), Some(""));
        assert_eq!(args.opt_truthy("c"), None);
        assert!(args.truthy("a") && args.truthy("b"));
        assert!(!args.truthy("c") && !args.truthy("missing"));
    }

    #[test]
    fn size_is_two_ascii_digit_runs_around_an_x() {
        assert_eq!(parse_size(None).unwrap(), None);
        assert_eq!(parse_size(Some("")).unwrap(), None);
        assert_eq!(
            parse_size(Some("1280x720")).unwrap(),
            Some(Size {
                width: 1280,
                height: 720
            })
        );
        assert_eq!(
            parse_size(Some("0x007")).unwrap(),
            Some(Size {
                width: 0,
                height: 7
            })
        );
        for bad in [
            "1280", "x720", "1280x", "1280X720", "1x2x3", " 1x2", "1.5x2", "-1x2", "١x2",
        ] {
            assert_eq!(
                parse_size(Some(bad)).unwrap_err().to_string(),
                format!("--size must be WxH, got {bad}")
            );
        }
    }

    #[test]
    fn format_is_webm_or_mp4() {
        assert_eq!(parse_format(None).unwrap(), None);
        assert_eq!(parse_format(Some("")).unwrap(), None);
        assert_eq!(parse_format(Some("webm")).unwrap(), Some(MovieFormat::Webm));
        assert_eq!(parse_format(Some("mp4")).unwrap(), Some(MovieFormat::Mp4));
        for bad in ["mov", "WEBM", " mp4"] {
            assert_eq!(
                parse_format(Some(bad)).unwrap_err().to_string(),
                "--format must be webm|mp4"
            );
        }
    }

    #[test]
    fn fps_follows_js_number_and_must_be_in_0_60() {
        assert_eq!(parse_fps(None).unwrap(), None);
        assert_eq!(parse_fps(Some("")).unwrap(), None);
        assert_eq!(parse_fps(Some("15")).unwrap(), Some(15.0));
        assert_eq!(parse_fps(Some("0.5")).unwrap(), Some(0.5));
        assert_eq!(parse_fps(Some("60")).unwrap(), Some(60.0));
        assert_eq!(parse_fps(Some("6e1")).unwrap(), Some(60.0));
        assert_eq!(parse_fps(Some("0x10")).unwrap(), Some(16.0));
        for bad in ["0", "-1", "60.5", "abc", "NaN", "Infinity", "12fps", " "] {
            assert_eq!(
                parse_fps(Some(bad)).unwrap_err().to_string(),
                "--fps must be in (0, 60]",
                "{bad}"
            );
        }
    }

    #[test]
    fn status_defaults_to_running_and_rejects_unknown_values() {
        let status = |input: &[&str]| parse_status(&parse_args(&argv(input)).args);
        assert_eq!(status(&["stop"]).unwrap(), ManifestStatus::Running);
        assert_eq!(
            status(&["stop", "--status"]).unwrap(),
            ManifestStatus::Running
        );
        assert_eq!(
            status(&["stop", "--status", "pass"]).unwrap(),
            ManifestStatus::Pass
        );
        assert_eq!(
            status(&["stop", "--status", "fail"]).unwrap(),
            ManifestStatus::Fail
        );
        assert_eq!(
            status(&["stop", "--status", "idle"]).unwrap(),
            ManifestStatus::Idle
        );
        assert_eq!(
            status(&["stop", "--status", "done"])
                .unwrap_err()
                .to_string(),
            "--status must be pass|fail|idle|running"
        );
    }

    #[test]
    fn usage_carries_the_decision_table_and_every_command() {
        let usage = usage();
        assert!(usage.starts_with(
            "astroshot movie — universal movie harness → .astroshot/ poster + video\n\nWhich --source should I use?"
        ));
        assert!(usage.contains(&format!("{SOURCE_DECISION_TABLE}\n\nCommands\n--------\n")));
        assert!(usage.contains("--slug onboard \\\n    --bundle-id com.example.App"));
        assert!(usage.ends_with("  astroshot movie list-windows\n"));
        for command in [
            "which-source",
            "list-windows",
            "check-screen-access",
            "open-screen-settings",
            "start / push-frame / mark / stop",
            "finalize",
            "help-sources",
        ] {
            assert!(usage.contains(command), "{command}");
        }
    }

    #[tokio::test]
    async fn help_and_advice_exit_0_and_usage_errors_exit_1() {
        for ok in [
            &[][..],
            &["--help"][..],
            &["help"][..],
            &["run", "-h"][..],
            &["nope", "--help"][..],
            &["help-sources"][..],
            &["which-source"][..],
            &["which-source", "ratatui dashboard"][..],
            &["which-source", "--intent", "web page"][..],
            &["which-source", "--for", "png sequence"][..],
        ] {
            assert_eq!(run_cli(&argv(ok)).await, 0, "{ok:?}");
        }
        for bad in [
            &["nope"][..],
            &["start"][..],
            &["start", "--feature", "x"][..],
            &["start", "--feature", "x", "--slug", "y", "--size", "big"][..],
            &["start", "--feature", "x", "--slug", "y", "--fps", "0"][..],
            &["start", "--feature", "x", "--slug", "y", "--format", "gif"][..],
            &["run"][..],
            &["run", "--source", "frames"][..],
            &["run", "--source", "nope", "--feature", "x", "--slug", "y"][..],
            &[
                "run",
                "--source",
                "desktop.display",
                "--feature",
                "x",
                "--slug",
                "y",
            ][..],
            &["run", "--source", "pty", "--feature", "x", "--slug", "y"][..],
            &[
                "run",
                "--source",
                "frames",
                "--feature",
                "x",
                "--slug",
                "y",
                "--status",
                "done",
            ][..],
            &["finalize", "--root", "/tmp"][..],
            &[
                "finalize",
                "--root",
                "/tmp",
                "--feature",
                "x",
                "--run-id",
                "r",
                "--status",
                "done",
            ][..],
        ] {
            assert_eq!(run_cli(&argv(bad)).await, 1, "{bad:?}");
        }
    }

    #[tokio::test]
    async fn frames_commands_persist_a_session_under_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let frame = dir.path().join("frame.png");
        std::fs::write(&frame, encode_solid_png(16, 16, [1, 2, 3])).unwrap();
        let frame = frame.to_string_lossy().into_owned();

        // Nothing started yet.
        assert_eq!(
            run_cli(&argv(&[
                "mark",
                "--root",
                &root,
                "--feature",
                "walk",
                "--slug",
                "a"
            ]))
            .await,
            1
        );
        assert_eq!(
            run_cli(&argv(&[
                "start",
                "--root",
                &root,
                "--feature",
                "walk",
                "--slug",
                "tour",
                "--size",
                "16x16",
                "--fps",
                "5",
                "--format",
                "mp4",
                "--run-id",
                "walk-1",
            ]))
            .await,
            0
        );
        let state = load_frame_session(&root, "walk", None).unwrap();
        assert_eq!(state.slug, "tour");
        assert_eq!(state.run_id, "walk-1");
        assert_eq!(state.fps, 5.0);
        assert_eq!(state.format, MovieFormat::Mp4);
        assert_eq!(
            state.size,
            Size {
                width: 16,
                height: 16
            }
        );

        let push = [
            "push-frame",
            "--root",
            &root,
            "--feature",
            "walk",
            "--file",
            &frame,
        ];
        assert_eq!(run_cli(&argv(&push)).await, 0);
        assert_eq!(run_cli(&argv(&push)).await, 0);
        // `--file` is required, must exist, and `--session` must name a session.
        assert_eq!(run_cli(&argv(&push[..5])).await, 1);
        assert_eq!(
            run_cli(&argv(&[
                "push-frame",
                "--root",
                &root,
                "--feature",
                "walk",
                "--file",
                "/nope/f.png",
            ]))
            .await,
            1
        );
        assert_eq!(
            run_cli(&argv(&[
                "push-frame",
                "--root",
                &root,
                "--feature",
                "walk",
                "--session",
                "nope",
                "--file",
                &frame,
            ]))
            .await,
            1
        );
        assert_eq!(
            run_cli(&argv(&[
                "mark",
                "--root",
                &root,
                "--feature",
                "walk",
                "--session",
                &state.id,
                "--slug",
                "half",
                "--note",
                "midway",
            ]))
            .await,
            0
        );
        assert_eq!(
            run_cli(&argv(&[
                "mark",
                "--root",
                &root,
                "--feature",
                "walk",
                "--slug",
                "Not Kebab"
            ]))
            .await,
            1
        );

        let state = load_frame_session(&root, "walk", Some(&state.id)).unwrap();
        assert_eq!(state.frame_count, 2);
        assert_eq!(state.chapters.len(), 1);
        assert_eq!(state.chapters[0].slug, "half");
        assert_eq!(state.chapters[0].note.as_deref(), Some("midway"));
        assert!(Path::new(&state.frame_dir).join("000001.png").exists());

        // An unknown status is refused before anything is encoded.
        assert_eq!(
            run_cli(&argv(&[
                "stop",
                "--root",
                &root,
                "--feature",
                "walk",
                "--status",
                "done"
            ]))
            .await,
            1
        );
        assert_eq!(
            load_frame_session(&root, "walk", None).unwrap().frame_count,
            2
        );
    }

    #[test]
    fn denied_help_text_is_the_ts_message() {
        let report = ScreenAccessReport {
            granted: false,
            requested: false,
            host_app: "Ghostty".to_string(),
            host_bundle_id: None,
            enable_app: "Ghostty".to_string(),
            settings_hint: String::new(),
        };
        assert_eq!(
            format_screen_recording_denied_help(Some(&report)),
            [
                "Screen Recording permission is required for --source desktop.window.",
                "",
                "Fix:",
                "  1. Open System Settings → Privacy & Security → Screen Recording",
                "     (or: astroshot movie open-screen-settings)",
                "  2. Enable \"Ghostty\"",
                "  3. Quit and reopen Ghostty completely (TCC applies on next launch)",
                "  4. Re-run: astroshot movie check-screen-access",
                "",
                "Note: macOS may not always show an automatic prompt; the Settings toggle is the reliable path.",
                "browser / pty sources do not need this permission.",
            ]
            .join("\n")
        );
    }

    #[test]
    fn engine_errors_get_the_legacy_command_hints() {
        use astroshot_engine::movie_harness::sources::desktop_macos::{
            DesktopWindowMatch, match_desktop_window,
        };
        let error = match_desktop_window(
            &[],
            &DesktopWindowMatch {
                owner: Some("x".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(
            describe_error(&error),
            "No desktop window matched {\"owner\":\"x\"}.\nRun: astroshot movie list-windows\nSample windows:\n  (none)"
        );
        let report = ScreenAccessReport {
            granted: false,
            requested: false,
            host_app: "Ghostty".to_string(),
            host_bundle_id: None,
            enable_app: "Ghostty".to_string(),
            settings_hint: String::new(),
        };
        let denied = anyhow::Error::new(DesktopError::ScreenAccessDenied {
            report: Some(report.clone()),
        });
        assert_eq!(
            describe_error(&denied),
            format_screen_recording_denied_help(Some(&report))
        );
        let failed = anyhow::Error::new(DesktopError::CaptureFailed {
            window_id: 7,
            status: "1".into(),
            detail: "boom".into(),
            access: Some(report.clone()),
        });
        assert_eq!(
            describe_error(&failed),
            format!(
                "screencapture failed for window 7 (1): boom.\n{}",
                format_screen_recording_denied_help(Some(&report))
            )
        );
        let empty = anyhow::Error::new(DesktopError::EmptyCapture {
            window_id: 7,
            access: None,
        });
        assert!(describe_error(&empty).starts_with(
            "screencapture produced an empty image for window 7. The window may have closed, or Screen Recording is denied.\nScreen Recording permission is required"
        ));
        assert_eq!(describe_error(&anyhow::anyhow!("plain")), "plain");
    }
}
