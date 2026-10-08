//! Binary-level tests for the movie CLI (`astroshot movie …`, and the
//! `astroshot-movie` executable name). `packages/movie-harness/src/cli.ts` has
//! no TS test file; these run the built `astroshot` binary against the
//! contract in that file: stdout JSON, stderr text, exit codes, and the
//! artifacts published under `.astroshot/`.
//!
//! Encoding needs `ffmpeg` (or Chrome, the encoder's fallback). Cases that
//! encode are skipped with a printed reason when neither is installed.
//! Nothing here needs Screen Recording permission or a visible window.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use astroshot_engine::movie_harness::encode_solid_png;
use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_astroshot");

const HINT: &str = "hint: See `astroshot movie which-source` or `astroshot movie --help` for the decision table.\n";

fn run_program(program: &Path, args: &[&str], cwd: &Path) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn astroshot")
}

/// `astroshot movie <args>` in `cwd`.
fn movie_in(cwd: &Path, args: &[&str]) -> Output {
    let mut full = vec!["movie"];
    full.extend_from_slice(args);
    run_program(Path::new(BIN), &full, cwd)
}

fn movie(args: &[&str]) -> Output {
    movie_in(&std::env::temp_dir(), args)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn combined(result: &Output) -> String {
    format!(
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        result.status,
        text(&result.stdout),
        text(&result.stderr)
    )
}

/// Stdout of a command that must exit 0, parsed as JSON.
fn json_ok(result: &Output) -> Value {
    assert_eq!(result.status.code(), Some(0), "{}", combined(result));
    serde_json::from_slice(&result.stdout).unwrap_or_else(|error| {
        panic!("stdout is not JSON ({error}):\n{}", combined(result));
    })
}

fn object_keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn can_encode() -> bool {
    let ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success());
    if ffmpeg || astroshot_engine::browser::find_chrome().is_ok() {
        return true;
    }
    common::skip("neither ffmpeg nor Chrome is installed");
    false
}

fn read_manifest(root: &Path, feature: &str) -> Value {
    let path = root.join(".astroshot").join(feature).join("manifest.json");
    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
}

/// The artifact's video and poster exist, are non-empty, sit in the feature
/// directory under `root`, and the poster is a PNG.
fn assert_published(artifact: &Value, root: &Path, feature: &str) -> (PathBuf, PathBuf) {
    let feature_dir = root.join(".astroshot").join(feature);
    let video = PathBuf::from(artifact["videoPath"].as_str().unwrap());
    let poster = PathBuf::from(artifact["posterPath"].as_str().unwrap());
    assert_eq!(video.parent(), Some(feature_dir.as_path()));
    assert_eq!(poster.parent(), Some(feature_dir.as_path()));
    assert!(std::fs::metadata(&video).unwrap().len() > 0);
    let poster_bytes = std::fs::read(&poster).unwrap();
    assert_eq!(&poster_bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    (video, poster)
}

const ARTIFACT_KEYS: [&str; 9] = [
    "videoPath",
    "posterPath",
    "durationMs",
    "chapters",
    "source",
    "feature",
    "slug",
    "sequence",
    "runId",
];

#[test]
fn frames_session_start_push_mark_stop_publishes_video_poster_and_manifest() {
    if !can_encode() {
        return;
    }
    // Setup: a worktree root and three solid frames a harness would produce.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("worktree");
    std::fs::create_dir_all(&root).unwrap();
    let root_arg = root.to_string_lossy().into_owned();
    let frames_dir = dir.path().join("captures");
    std::fs::create_dir_all(&frames_dir).unwrap();
    for (index, rgb) in [[124, 92, 255], [90, 200, 250], [80, 220, 160]]
        .into_iter()
        .enumerate()
    {
        std::fs::write(
            frames_dir.join(format!("f{index}.png")),
            encode_solid_png(64, 48, rgb),
        )
        .unwrap();
    }

    // start: each command is its own process; state lives under the root.
    let started = json_ok(&movie(&[
        "start",
        "--root",
        &root_arg,
        "--feature",
        "walkthrough",
        "--slug",
        "tour",
        "--run-id",
        "walk-1",
        "--title",
        "Guided tour",
        "--size",
        "64x48",
        "--fps",
        "5",
    ]));
    assert_eq!(
        object_keys(&started),
        ["id", "feature", "slug", "runId", "frameDir"]
    );
    let id = started["id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 12);
    assert_eq!(started["feature"], "walkthrough");
    assert_eq!(started["slug"], "tour");
    assert_eq!(started["runId"], "walk-1");
    let frame_dir = root
        .join(".astroshot/walkthrough/.movie")
        .join(&id)
        .join("frames");
    assert_eq!(started["frameDir"], frame_dir.to_string_lossy().as_ref());

    // push-frame: `--file` is resolved against the caller's directory, and
    // the session is found without `--session` (latest).
    for (index, name) in ["f0.png", "f1.png"].into_iter().enumerate() {
        let pushed = movie_in(
            &frames_dir,
            &[
                "push-frame",
                "--root",
                &root_arg,
                "--feature",
                "walkthrough",
                "--file",
                name,
            ],
        );
        assert_eq!(
            text(&pushed.stdout),
            format!(
                "{{\n  \"id\": \"{id}\",\n  \"frameCount\": {}\n}}\n",
                index + 1
            ),
            "{}",
            combined(&pushed)
        );
    }

    // mark: a chapter with a note, addressed by session id.
    let marked = json_ok(&movie(&[
        "mark",
        "--root",
        &root_arg,
        "--feature",
        "walkthrough",
        "--session",
        &id,
        "--slug",
        "second-step",
        "--note",
        "after the first click",
    ]));
    assert_eq!(object_keys(&marked), ["id", "chapters"]);
    let chapter = &marked["chapters"][0];
    assert_eq!(object_keys(chapter), ["slug", "tMs", "note"]);
    assert_eq!(chapter["slug"], "second-step");
    assert_eq!(chapter["note"], "after the first click");
    assert!(chapter["tMs"].is_u64(), "tMs prints as a JS integer");

    let last = frames_dir.join("f2.png");
    json_ok(&movie(&[
        "push-frame",
        "--root",
        &root_arg,
        "--feature",
        "walkthrough",
        "--file",
        &last.to_string_lossy(),
    ]));
    assert!(frame_dir.join("000002.png").exists());

    // stop: encode, publish next to the manifest, drop the session state.
    let artifact = json_ok(&movie(&[
        "stop",
        "--root",
        &root_arg,
        "--feature",
        "walkthrough",
        "--status",
        "pass",
    ]));
    assert_eq!(object_keys(&artifact), ARTIFACT_KEYS);
    let (video, poster) = assert_published(&artifact, &root, "walkthrough");
    assert_eq!(video.file_name().unwrap(), "0001-tour.webm");
    assert_eq!(poster.file_name().unwrap(), "0001-tour.png");
    assert_eq!(artifact["durationMs"], 600);
    assert_eq!(artifact["source"], "frames");
    assert_eq!(artifact["sequence"], "0001");
    assert_eq!(artifact["runId"], "walk-1");
    assert_eq!(artifact["chapters"], marked["chapters"]);
    assert!(!frame_dir.exists());

    let manifest = read_manifest(&root, "walkthrough");
    assert_eq!(manifest["run_id"], "walk-1");
    assert_eq!(manifest["status"], "pass");
    let shot = &manifest["shots"][0];
    assert_eq!(shot["file"], "0001-tour.png");
    assert_eq!(shot["video"], "0001-tour.webm");
    assert_eq!(shot["title"], "Guided tour");
    assert_eq!(shot["kind"], "movie");
    assert_eq!(shot["source"], "frames");
    assert_eq!(shot["viewport"], "64x48");
    assert_eq!(shot["duration_ms"], 600);
    assert_eq!(shot["chapters"][0]["slug"], "second-step");

    // The session is gone: a second stop has nothing to load.
    let again = movie(&["stop", "--root", &root_arg, "--feature", "walkthrough"]);
    assert_eq!(again.status.code(), Some(1));
    assert_eq!(
        text(&again.stderr),
        format!("error: no movie session for feature walkthrough\n{HINT}")
    );

    // finalize: flips the manifest status and echoes what it set.
    let finalized = movie(&[
        "finalize",
        "--root",
        &root_arg,
        "--feature",
        "walkthrough",
        "--run-id",
        "walk-1",
        "--status",
        "fail",
    ]);
    assert_eq!(finalized.status.code(), Some(0), "{}", combined(&finalized));
    assert_eq!(
        text(&finalized.stdout),
        "{\n  \"feature\": \"walkthrough\",\n  \"runId\": \"walk-1\",\n  \"status\": \"fail\"\n}\n"
    );
    assert_eq!(read_manifest(&root, "walkthrough")["status"], "fail");
}

#[test]
fn run_source_frames_records_its_own_demo_frames_with_a_midpoint_chapter() {
    if !can_encode() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    let artifact = json_ok(&movie(&[
        "run",
        "--source",
        "frames",
        "--root",
        &root,
        "--feature",
        "demo",
        "--slug",
        "colors",
        "--demo-frames",
        "4",
        "--format",
        "mp4",
    ]));
    assert_eq!(object_keys(&artifact), ARTIFACT_KEYS);
    let (video, _) = assert_published(&artifact, dir.path(), "demo");
    assert_eq!(video.file_name().unwrap(), "0001-colors.mp4");
    // 4 frames at the default 15 fps.
    assert_eq!(artifact["durationMs"], 267);
    assert_eq!(artifact["chapters"].as_array().unwrap().len(), 1);
    assert_eq!(artifact["chapters"][0]["slug"], "midpoint");
    let manifest = read_manifest(dir.path(), "demo");
    assert_eq!(manifest["status"], "running");
    // Default demo size.
    assert_eq!(manifest["shots"][0]["viewport"], "320x180");
    assert!(
        !dir.path()
            .join(".astroshot/demo/.movie")
            .join("demo-src")
            .exists()
    );
}

#[test]
fn run_source_pty_demo_records_a_truecolor_movie() {
    if !can_encode() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    let artifact = json_ok(&movie(&[
        "run",
        "--source",
        "pty-demo",
        "--root",
        &root,
        "--feature",
        "tui",
        "--slug",
        "truecolor",
        "--color",
        "ff8800",
        "--status",
        "idle",
    ]));
    assert_eq!(object_keys(&artifact), ARTIFACT_KEYS);
    assert_eq!(artifact["source"], "pty");
    assert_eq!(artifact["chapters"], Value::Array(Vec::new()));
    assert!(artifact["durationMs"].as_u64().unwrap() > 0);
    let (video, poster) = assert_published(&artifact, dir.path(), "tui");
    assert_eq!(video.file_name().unwrap(), "0001-truecolor.webm");

    // The poster carries the requested 24-bit color, not a palette color.
    let png = image::load_from_memory(&std::fs::read(poster).unwrap())
        .unwrap()
        .to_rgba8();
    assert!(
        png.pixels()
            .any(|pixel| pixel.0 == [0xff, 0x88, 0x00, 0xff]),
        "poster has no #ff8800 pixel"
    );

    let manifest = read_manifest(dir.path(), "tui");
    assert_eq!(manifest["status"], "idle");
    assert_eq!(manifest["shots"][0]["source"], "pty");
    assert_eq!(manifest["shots"][0]["kind"], "movie");
}

#[test]
fn which_source_prints_the_recommendation_as_json() {
    let tui = movie(&["which-source", "ratatui", "truecolor", "dashboard"]);
    assert_eq!(tui.status.code(), Some(0));
    assert_eq!(text(&tui.stderr), "");
    assert_eq!(
        text(&tui.stdout),
        "{\n  \"intent\": \"ratatui truecolor dashboard\",\n  \"recommended\": \"pty\",\n  \"reason\": \"Terminal/TUI intent detected — use pty for truecolor SGR fidelity (not desktop of a terminal app).\"\n}\n"
    );

    // `--intent` and `--for` carry the text when there are no positionals.
    let native = json_ok(&movie(&[
        "which-source",
        "--intent",
        "SwiftUI native app window bundle id",
    ]));
    assert_eq!(object_keys(&native), ["intent", "recommended", "reason"]);
    assert_eq!(native["recommended"], "desktop.window");
    let frames = json_ok(&movie(&["which-source", "--for", "png sequence"]));
    assert_eq!(frames["recommended"], "frames");
    // The parser accepts any `--flag`; unknown ones are ignored.
    let ignored = json_ok(&movie(&["which-source", "web page", "--bogus"]));
    assert_eq!(ignored["recommended"], "browser");

    // No intent: the decision table and how to ask.
    let bare = movie(&["which-source"]);
    assert_eq!(bare.status.code(), Some(0));
    let out = text(&bare.stdout);
    assert!(out.starts_with("Which --source should I use?\n"));
    assert!(out.ends_with(
        "\n\nPass free text, e.g.:\n  astroshot movie which-source \"native SwiftUI onboarding window\"\n  astroshot movie which-source \"ratatui truecolor dashboard\"\n"
    ));

    let catalog = movie(&["help-sources"]);
    assert_eq!(catalog.status.code(), Some(0));
    let out = text(&catalog.stdout);
    assert!(out.starts_with("Which --source should I use?\n"));
    assert!(out.contains("\n\nSource catalog\n--------------\n\n--source browser\n"));
    assert!(out.contains("--source desktop.window\n"));
}

#[test]
fn help_goes_to_stdout_and_unknown_commands_repeat_it_on_stderr() {
    let help = movie(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert_eq!(text(&help.stderr), "");
    let usage = text(&help.stdout);
    assert!(usage.starts_with(
        "astroshot movie — universal movie harness → .astroshot/ poster + video\n\nWhich --source should I use?\n"
    ));
    assert!(usage.ends_with("  astroshot movie list-windows\n\n"));
    for args in [&[][..], &["help"][..], &["-h"][..], &["run", "--help"][..]] {
        let result = movie(args);
        assert_eq!(result.status.code(), Some(0), "{args:?}");
        assert_eq!(text(&result.stdout), usage, "{args:?}");
    }

    // The usage text names which-source, so no extra hint line follows.
    let unknown = movie(&["nope"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(text(&unknown.stdout), "");
    assert_eq!(
        text(&unknown.stderr),
        format!("error: unknown command: nope\n\n{usage}")
    );
}

#[test]
fn usage_errors_exit_1_with_the_exact_message_and_a_hint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    let cases: Vec<(Vec<&str>, String)> = vec![
        (
            vec!["start"],
            format!("error: --feature is required\n{HINT}"),
        ),
        (
            vec!["start", "--feature", "x"],
            format!("error: --slug is required\n{HINT}"),
        ),
        // A flag followed by another flag has no value.
        (
            vec!["start", "--feature", "--slug", "y"],
            format!("error: --feature is required\n{HINT}"),
        ),
        (
            vec!["start", "--feature", "x", "--slug", "y", "--size", "big"],
            format!("error: --size must be WxH, got big\n{HINT}"),
        ),
        (
            vec!["start", "--feature", "x", "--slug", "y", "--fps", "61"],
            format!("error: --fps must be in (0, 60]\n{HINT}"),
        ),
        (
            vec!["start", "--feature", "x", "--slug", "y", "--format", "gif"],
            format!("error: --format must be webm|mp4\n{HINT}"),
        ),
        (
            vec![
                "start",
                "--root",
                &root,
                "--feature",
                "Bad Name",
                "--slug",
                "y",
            ],
            format!("error: feature must be kebab-case [a-z0-9-]+, got \"Bad Name\"\n{HINT}"),
        ),
        (
            vec!["mark", "--root", &root, "--feature", "none", "--slug", "a"],
            format!("error: no movie session for feature none\n{HINT}"),
        ),
        (
            vec![
                "push-frame",
                "--root",
                &root,
                "--feature",
                "none",
                "--file",
                "a.png",
            ],
            format!("error: no movie session for feature none\n{HINT}"),
        ),
        (vec!["run"], format!("error: --source is required\n{HINT}")),
        (
            vec!["run", "--source", "frames"],
            format!("error: --feature is required\n{HINT}"),
        ),
        (
            vec!["run", "--source", "pty", "--feature", "x", "--slug", "y"],
            format!("error: --fixture is required\n{HINT}"),
        ),
        (
            vec![
                "run",
                "--source",
                "frames",
                "--root",
                &root,
                "--feature",
                "x",
                "--slug",
                "y",
                "--demo-frames",
                "none",
            ],
            format!("error: cannot stop movie session with zero frames\n{HINT}"),
        ),
        (
            vec!["finalize", "--root", &root],
            format!("error: --feature is required\n{HINT}"),
        ),
        (
            vec![
                "finalize",
                "--root",
                &root,
                "--feature",
                "x",
                "--run-id",
                "r",
                "--status",
                "done",
            ],
            format!("error: --status must be pass|fail|idle|running\n{HINT}"),
        ),
        // Messages that already name the decision table or which-source get
        // no hint line.
        (
            vec!["run", "--source", "no\"pe", "--feature", "x", "--slug", "y"],
            format!(
                "error: unknown --source \"no\\\"pe\"\n{}\n",
                astroshot::cli::source_help::SOURCE_DECISION_TABLE
            ),
        ),
        (
            vec![
                "run",
                "--source",
                "desktop.region",
                "--feature",
                "x",
                "--slug",
                "y",
            ],
            format!(
                "error: desktop.region is not implemented yet.\nUse --source desktop.window for a single app, or --source frames.\n{}\n{HINT}",
                astroshot::cli::source_help::source_hint_for_error(Some("desktop.window"))
            ),
        ),
    ];
    for (args, expected) in cases {
        let result = movie(&args);
        assert_eq!(
            result.status.code(),
            Some(1),
            "{args:?}\n{}",
            combined(&result)
        );
        assert_eq!(text(&result.stdout), "", "{args:?}");
        assert_eq!(text(&result.stderr), expected, "{args:?}");
    }
}

#[cfg(unix)]
#[test]
fn astroshot_movie_argv0_alias_runs_the_movie_cli() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("astroshot-movie");
    std::os::unix::fs::symlink(BIN, &alias).unwrap();
    let call = |args: &[&str]| run_program(&alias, args, dir.path());

    // The alias takes the movie command as its first argument.
    let advice = json_ok(&call(&["which-source", "ink terminal app"]));
    assert_eq!(advice["recommended"], "pty");
    assert_eq!(
        text(&call(&["--help"]).stdout),
        text(&movie(&["--help"]).stdout)
    );
    // No arguments is help with exit 0, as the npm bin behaved.
    let bare = call(&[]);
    assert_eq!(bare.status.code(), Some(0));
    assert!(text(&bare.stdout).starts_with("astroshot movie — universal movie harness"));
    let missing = call(&["start"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        text(&missing.stderr),
        format!("error: --feature is required\n{HINT}")
    );
}

/// `check-screen-access` only preflights the permission (no `--request`
/// prompt), so it runs unattended; granted and denied are both valid results.
#[cfg(target_os = "macos")]
#[test]
fn check_screen_access_prints_the_report_json_and_exits_0_or_2() {
    let result = movie(&["check-screen-access"]);
    let stderr = text(&result.stderr);
    if result.status.code() == Some(1)
        && stderr.starts_with("error: desktop.window requires the Swift toolchain")
    {
        eprintln!("SKIP: {stderr}");
        return;
    }
    let stdout = text(&result.stdout);
    let report: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout is not JSON ({error}):\n{}", combined(&result)));
    assert_eq!(
        object_keys(&report),
        [
            "granted",
            "requested",
            "hostApp",
            "hostBundleId",
            "enableApp",
            "settingsHint"
        ]
    );
    // `JSON.stringify(report, null, 2)` plus one newline.
    assert!(stdout.starts_with("{\n  \"granted\": "));
    assert!(stdout.ends_with("\"\n}\n"));
    assert_eq!(report["requested"], false);
    let app = report["enableApp"].as_str().unwrap();
    let hint = report["settingsHint"].as_str().unwrap();
    assert!(!app.is_empty());

    if report["granted"] == true {
        assert_eq!(result.status.code(), Some(0));
        assert_eq!(
            stderr,
            format!(
                "# Screen Recording preflight: OK — still enable \"{app}\" in Settings if captures fail\n"
            )
        );
    } else {
        assert_eq!(report["granted"], false);
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(
            stderr,
            format!(
                "\nScreen Recording: DENIED (enable \"{app}\")\n  → {hint}\n  → or: astroshot movie open-screen-settings\n  → then quit & reopen {app}, re-run check-screen-access\n"
            )
        );
    }
}

/// Off macOS every desktop command fails at the toolchain check. The message
/// names which-source, so there is no hint line.
#[cfg(not(target_os = "macos"))]
#[test]
fn desktop_commands_are_refused_off_macos() {
    let expected = "error: desktop.window is only implemented on macOS (CGWindowList + screencapture). On other platforms use --source frames and push your own captures. See: astroshot movie which-source\n";
    for args in [
        &["check-screen-access"][..],
        &["list-windows"][..],
        &["open-screen-settings"][..],
        &[
            "run",
            "--source",
            "desktop.window",
            "--feature",
            "x",
            "--slug",
            "y",
        ][..],
    ] {
        let result = movie(args);
        assert_eq!(result.status.code(), Some(1), "{args:?}");
        assert_eq!(text(&result.stdout), "", "{args:?}");
        assert_eq!(text(&result.stderr), expected, "{args:?}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn desktop_window_needs_a_window_selector_before_any_capture() {
    let probe = movie(&[
        "run",
        "--source",
        "desktop.window",
        "--feature",
        "x",
        "--slug",
        "y",
    ]);
    let stderr = text(&probe.stderr);
    if stderr.starts_with("error: desktop.window requires the Swift toolchain") {
        eprintln!("SKIP: {stderr}");
        return;
    }
    assert_eq!(probe.status.code(), Some(1));
    // The message names list-windows, so there is no hint line.
    assert_eq!(
        stderr,
        "error: desktop.window requires one of --window-id, --bundle-id, --title-regex, --owner, or --pid. Run: astroshot movie list-windows\n"
    );
    let bad_pid = movie(&[
        "run",
        "--source",
        "desktop.window",
        "--feature",
        "x",
        "--slug",
        "y",
        "--pid",
        "abc",
    ]);
    assert_eq!(bad_pid.status.code(), Some(1));
    assert_eq!(
        text(&bad_pid.stderr),
        format!("error: --pid must be a number\n{HINT}")
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs a logged-in macOS GUI session with visible windows (CGWindowList)"]
fn list_windows_prints_the_window_rows_and_a_count_on_stderr() {
    let result = movie(&["list-windows"]);
    let windows = json_ok(&result);
    let rows = windows.as_array().unwrap();
    assert!(!rows.is_empty());
    assert!(rows[0]["id"].is_u64());
    assert!(rows.iter().all(|row| row.get("bundleId").is_some()));
    assert_eq!(
        text(&result.stderr),
        format!(
            "# {} windows — match with --window-id / --bundle-id / --title-regex / --owner / --pid\n",
            rows.len()
        )
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs Screen Recording permission for the host app and a visible Finder window"]
fn run_source_desktop_window_records_a_native_window() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    let artifact = json_ok(&movie(&[
        "run",
        "--source",
        "desktop.window",
        "--root",
        &root,
        "--feature",
        "native",
        "--slug",
        "finder",
        "--bundle-id",
        "com.apple.finder",
        "--duration-ms",
        "600",
    ]));
    assert_eq!(artifact["source"], "desktop.window");
    assert_published(&artifact, dir.path(), "native");
}

/// Help text `astroshot-movie --help` printed, captured from the TS bin.
const MOVIE_HELP: &str = include_str!("fixtures/help/astroshot-movie.txt");

/// `packages/movie-harness/bin/astroshot-movie.mjs` ran `runCli` with the raw
/// arguments; the binary does the same when it is named `astroshot-movie`.
#[cfg(unix)]
#[test]
fn astroshot_movie_bin_help_and_unknown_commands_match_the_ts_bin() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("astroshot-movie");
    std::os::unix::fs::symlink(BIN, &alias).unwrap();
    let call = |args: &[&str]| run_program(&alias, args, dir.path());
    #[track_caller]
    fn assert_exact(output: &Output, stdout: &str, stderr: &str, code: i32) {
        assert_eq!(text(&output.stdout), stdout, "stdout");
        assert_eq!(text(&output.stderr), stderr, "stderr");
        assert_eq!(output.status.code(), Some(code), "exit code");
    }

    // No arguments, the help words, and --help after any command: usage, exit 0.
    for args in [
        &[][..],
        &["--help"],
        &["-h"],
        &["help"],
        &["which-source", "--help"],
        &["start", "--help"],
        &["run", "--help"],
    ] {
        assert_exact(&call(args), MOVIE_HELP, "", 0);
    }
    // The movie CLI has no version flag; anything else is an unknown command
    // that repeats the usage on stderr.
    for command in ["--version", "-v", "bogus", "--bogus"] {
        assert_exact(
            &call(&[command]),
            "",
            &format!("error: unknown command: {command}\n\n{MOVIE_HELP}"),
            1,
        );
    }
    // `astroshot movie` forwards everything to the same CLI.
    assert_exact(&movie_in(dir.path(), &[]), MOVIE_HELP, "", 0);
    assert_exact(&movie_in(dir.path(), &["--help"]), MOVIE_HELP, "", 0);
}
