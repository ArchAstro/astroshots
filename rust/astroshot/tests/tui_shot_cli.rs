//! Binary-level tests for the `tui-shot` CLI (`astroshot ink|tui|pty`, and the
//! `tui-shot` executable name). Ports `packages/tui-shot/src/cli.e2e.test.ts`
//! and `packages/tui-shot/src/pty-cli.e2e.test.ts`, which ran the published
//! bin; here the built `astroshot` binary is the process under test.
//!
//! Ink cases need `node` >=22 and the workspace `node_modules`; PTY fixtures
//! launch `node` too. Cases are skipped (with a printed reason) without Node.
//!
//! PNG assertions: the TS checked sharp metadata of a Chromium screenshot.
//! The Rust rasterizer draws the same cells natively, so sizes are compared
//! with the same lower bounds and the pixel-variation check is kept.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use astroshot::node_helper::find_node;

const BIN: &str = env!("CARGO_BIN_EXE_astroshot");

fn repo() -> PathBuf {
    // Not canonicalized: the worktree's node_modules may be a symlink.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn package_root() -> PathBuf {
    repo().join("packages/tui-shot")
}

fn fixture(name: &str) -> String {
    package_root()
        .join("fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn node_or_skip() -> bool {
    if let Err(error) = find_node() {
        common::skip(&error);
        return false;
    }
    true
}

fn run_with(program: &Path, args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

fn run(args: &[&str]) -> Output {
    run_with(Path::new(BIN), args, &package_root(), &[])
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

/// The PNG's size, after checking that it decodes and is not a flat color
/// (`stats.channels.some((channel) => channel.max > channel.min)`).
fn varied_png_size(path: &Path) -> (u32, u32) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    let png = image::load_from_memory(&bytes).unwrap().to_rgba8();
    let first = png.get_pixel(0, 0).0;
    let varied = (0..4).any(|channel| png.pixels().any(|p| p.0[channel] != first[channel]));
    assert!(varied, "{} is a flat image", path.display());
    (png.width(), png.height())
}

fn copy_tree(from: &Path, to: &Path) {
    // `fs.cpSync(..., { recursive: true })`: symlinks are copied as links.
    for entry in walkdir::WalkDir::new(from).follow_links(false) {
        let entry = entry.unwrap();
        let target = to.join(entry.path().strip_prefix(from).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else if entry.file_type().is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path()).unwrap(), &target).unwrap();
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// `makeIsolatedInkConsumer`: a project outside the repo with its own copies
/// of ink/react (the packages that must be a single instance) and links for
/// ink's other dependencies. Returns the fixture path.
#[cfg(unix)]
fn make_isolated_ink_consumer(consumer_dir: &Path) -> PathBuf {
    let installed_ink = [package_root(), repo()]
        .iter()
        .map(|root| root.join("node_modules/ink"))
        .find(|candidate| candidate.exists())
        .expect("ink is installed in the workspace")
        .canonicalize()
        .unwrap();
    let installed_modules = installed_ink.parent().unwrap().to_path_buf();
    let consumer_modules = consumer_dir.join("node_modules");
    std::fs::create_dir_all(&consumer_modules).unwrap();
    std::fs::write(
        consumer_dir.join("package.json"),
        r#"{"private":true,"type":"module"}"#,
    )
    .unwrap();

    let copied_packages = ["ink", "react", "react-reconciler", "scheduler", "chalk"];
    for package_name in copied_packages {
        copy_tree(
            &installed_modules.join(package_name),
            &consumer_modules.join(package_name),
        );
    }

    let ink_package: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(installed_ink.join("package.json")).unwrap())
            .unwrap();
    for package_name in ink_package["dependencies"].as_object().unwrap().keys() {
        if copied_packages.contains(&package_name.as_str()) {
            continue;
        }
        let destination = consumer_modules.join(package_name);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(installed_modules.join(package_name), &destination).unwrap();
    }

    let fixture_path = consumer_dir.join("hook-fixture.tsx");
    std::fs::write(
        &fixture_path,
        r##"import React, { useState } from "react";
import { Box, Text } from "ink";

function HookScreen() {
  const [status] = useState("Isolated React hook rendered");
  return (
    <Box borderStyle="round" borderColor="#b9a8ff">
      <Text color="#b9a8ff">{status}</Text>
    </Box>
  );
}

export default {
  cols: 42,
  rows: 6,
  scale: 1,
  expectText: ["Isolated React hook rendered"],
  component: <HookScreen />,
};
"##,
    )
    .unwrap();
    fixture_path
}

// ---------------------------------------------------------------------------
// cli.e2e.test.ts — "tui-shot CLI boundary"

#[cfg(unix)]
#[test]
fn renders_a_separate_consumers_hook_using_ink_runtime_through_the_published_bin() {
    if !node_or_skip() {
        return;
    }
    // Setup a disposable destination outside the package, as an npx caller would.
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("basic.png");
    let fixture_path = make_isolated_ink_consumer(&dir.path().join("consumer"));

    // Cross the actual binary, the Node helper's TSX fixture loader, the
    // consumer's own Ink, and the native rasterizer.
    let result = run(&[
        "ink",
        "shot",
        &fixture_path.to_string_lossy(),
        "-o",
        &out_path.to_string_lossy(),
    ]);

    // The fixture's expectText validates semantic content before capture;
    // inspect the resulting pixels to prove a real, non-flat PNG was written.
    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert!(
        text(&result.stdout).contains(&format!("wrote {}", out_path.display())),
        "{}",
        combined(&result)
    );
    let (width, height) = varied_png_size(&out_path);
    assert!(width > 300, "{width}");
    assert!(height > 100, "{height}");
    assert!(std::fs::metadata(&out_path).unwrap().len() > 4_000);
}

#[test]
fn keeps_nested_batch_destinations_distinct_under_out_dir() {
    if !node_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("artifacts");
    let manifest_path = dir.path().join("batch.json");
    std::fs::write(
        &manifest_path,
        serde_json::json!({
            "shots": [
                { "fixture": fixture("basic.tsx"), "out": "first/screen.png" },
                { "fixture": fixture("basic.tsx"), "out": "second/screen.png" },
            ]
        })
        .to_string(),
    )
    .unwrap();

    let result = run(&[
        "ink",
        "batch",
        &manifest_path.to_string_lossy(),
        "--out-dir",
        &out_dir.to_string_lossy(),
    ]);

    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    let stdout = text(&result.stdout);
    assert!(stdout.contains("done: 2/2 shots"), "{stdout}");
    // Progress lines keep the TS glyphs.
    assert_eq!(stdout.matches("shot fixtures/basic.tsx … → ").count(), 2);
    for name in ["first/screen.png", "second/screen.png"] {
        assert!(std::fs::metadata(out_dir.join(name)).unwrap().len() > 4_000);
    }
}

// ---------------------------------------------------------------------------
// pty-cli.e2e.test.ts — "arbitrary PTY capture boundary"

#[test]
fn drives_a_full_screen_terminal_process_and_captures_the_selected_state() {
    if !node_or_skip() {
        return;
    }
    // Arrange a disposable output while keeping the fixture beside its real
    // child executable, so relative command paths exercise fixture semantics.
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("selected.png");

    // Cross the binary into a real pseudoterminal. The child enters the
    // alternate screen, receives Down and Enter, and redraws using ANSI.
    let result = run(&[
        "pty",
        &fixture("interactive-pty.yaml"),
        "-o",
        &out_path.to_string_lossy(),
    ]);

    // The fixture's final wait and expectText prove the externally visible
    // selected state; PNG pixels prove the rasterizer captured it.
    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert!(text(&result.stdout).contains(&format!("wrote {}", out_path.display())));
    let (width, height) = varied_png_size(&out_path);
    assert!(width > 400, "{width}");
    assert!(height > 150, "{height}");
    assert!(std::fs::metadata(&out_path).unwrap().len() > 4_000);
}

#[test]
fn waits_for_a_clean_terminal_exit_before_capturing_its_final_frame() {
    if !node_or_skip() {
        return;
    }
    // Run without the wrapper override: the PTY's own exit event is used.
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("complete.png");
    let result = run(&[
        "pty",
        &fixture("completing-pty.yaml"),
        "-o",
        &out_path.to_string_lossy(),
    ]);

    // A successful CLI result and real PNG prove waitForExit crossed the
    // process boundary and retained the program's final visible frame.
    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert!(text(&result.stdout).contains(&format!("wrote {}", out_path.display())));
    let (width, height) = varied_png_size(&out_path);
    assert!(width > 300, "{width}");
    assert!(height > 100, "{height}");
    assert!(std::fs::metadata(&out_path).unwrap().len() > 2_000);
}

#[test]
fn rejects_a_crashed_terminal_process_even_when_its_expected_text_rendered() {
    if !node_or_skip() {
        return;
    }
    // Launch a real PTY child that draws a plausible final frame and exits 7.
    // This guards documentation capture from certifying a crash as success.
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("crash.png");
    let result = run_with(
        Path::new(BIN),
        &[
            "pty",
            &fixture("crashing-pty.yaml"),
            "-o",
            &out_path.to_string_lossy(),
        ],
        &package_root(),
        &[
            // Exercise the exit-status bridge (`astroshot __pty-exit-wrapper`).
            ("ASTROSHOT_TEST_FORCE_PTY_EXIT_WRAPPER", "1"),
            // Status file is written immediately; delay the OSC marker so
            // waitForExit must trust the file bridge.
            ("ASTROSHOT_TEST_DELAY_PTY_EXIT_MARKER_MS", "1500"),
        ],
    );

    // The visible assertion passes, but the process boundary is authoritative.
    assert_eq!(result.status.code(), Some(1), "{}", combined(&result));
    assert!(
        text(&result.stderr).contains("exited with code 7 before capture"),
        "{}",
        combined(&result)
    );
    assert!(!out_path.exists());
}

#[test]
fn preserves_a_terminal_interrupt_until_the_wrapped_program_reports_its_exit() {
    if !node_or_skip() {
        return;
    }
    // Force the status bridge, so the PTY child is this same binary run as
    // `__pty-exit-wrapper`, then send a real Ctrl-C through the PTY to both
    // the wrapper and the target process.
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("interrupted.png");
    let result = run_with(
        Path::new(BIN),
        &[
            "pty",
            &fixture("interruptible-pty.yaml"),
            "-o",
            &out_path.to_string_lossy(),
        ],
        &package_root(),
        &[("ASTROSHOT_TEST_FORCE_PTY_EXIT_WRAPPER", "1")],
    );

    // The target's chosen status survives the shared process-group signal.
    assert_eq!(result.status.code(), Some(1), "{}", combined(&result));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("exited with code 42 before capture"),
        "{}",
        combined(&result)
    );
    assert!(!stderr.contains("status wrapper exited without reporting"));
    assert!(!out_path.exists());
}

#[cfg(windows)]
#[test]
fn rejects_windows_batch_scripts_instead_of_introducing_a_hidden_shell() {
    let dir = tempfile::tempdir().unwrap();
    let fixture_path = dir.path().join("batch.yaml");
    let command_path = dir.path().join("astroshot-batch-probe.cmd");
    let out_path = dir.path().join("batch.png");
    std::fs::write(&command_path, "@echo off\r\necho unsafe shell boundary\r\n").unwrap();
    std::fs::write(
        &fixture_path,
        "version: 1\ncommand: astroshot-batch-probe\ncols: 40\nrows: 6",
    )
    .unwrap();
    let path = format!(
        "{};{}",
        dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let result = run_with(
        Path::new(BIN),
        &[
            "pty",
            &fixture_path.to_string_lossy(),
            "-o",
            &out_path.to_string_lossy(),
        ],
        &package_root(),
        &[("PATH", &path)],
    );

    assert_eq!(result.status.code(), Some(1), "{}", combined(&result));
    assert!(text(&result.stderr).contains("Windows batch script, which requires a shell"));
    assert!(!out_path.exists());
}

// ---------------------------------------------------------------------------
// Rust binary surface: `astroshot tui`, the `tui-shot` name, usage errors.

#[test]
fn astroshot_tui_writes_a_png_for_a_bare_ink_fixture_path() {
    if !node_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("nested/basic.png");

    // `tui` is the ink alias and a bare .tsx path implies `shot`.
    let result = run(&[
        "tui",
        &fixture("basic.tsx"),
        "--out",
        &out_path.to_string_lossy(),
    ]);

    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert_eq!(
        text(&result.stdout),
        format!("wrote {}\n", out_path.display())
    );
    assert_eq!(text(&result.stderr), "");
    // basic.tsx: 42x8 cells at scale 1.
    assert_eq!(varied_png_size(&out_path), (435, 203));

    // --cols/--rows/--scale override the fixture's grid and device scale.
    let scaled = dir.path().join("scaled.png");
    let result = run(&[
        "ink",
        "shot",
        &fixture("basic.tsx"),
        "-o",
        &scaled.to_string_lossy(),
        "--cols",
        "60",
        "--rows",
        "10",
        "--scale",
        "2",
    ]);
    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    // ceil(60*15*.62+44) x ceil(10*15*1.32+44) CSS pixels, doubled
    // (150*1.32 is exactly 198 in f64, as in JS; TS writes 1204x484).
    assert_eq!(varied_png_size(&scaled), (602 * 2, 242 * 2));
}

#[test]
fn astroshot_tui_pty_writes_a_png_for_a_pty_fixture() {
    if !node_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("pty.png");

    let result = run(&[
        "tui",
        "pty",
        &fixture("interactive-pty.yaml"),
        "-o",
        &out_path.to_string_lossy(),
    ]);

    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert_eq!(
        text(&result.stdout),
        format!("wrote {}\n", out_path.display())
    );
    // interactive-pty.yaml: 52x10 cells at scale 1. TS writes 528x242.
    assert_eq!(varied_png_size(&out_path), (528, 242));
}

#[cfg(unix)]
#[test]
fn tui_shot_argv0_alias_runs_the_tui_shot_cli() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("tui-shot");
    std::os::unix::fs::symlink(BIN, &alias).unwrap();
    let call = |args: &[&str]| run_with(&alias, args, &package_root(), &[]);

    let help = call(&["shot", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(
        text(&help.stdout)
            .starts_with("tui-shot — deterministic PNG screenshots of terminal interfaces\n")
    );
    assert!(text(&help.stdout).ends_with("  -h, --help             Show this help\n\n"));

    let unknown = call(&["nope"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(text(&unknown.stderr), "Unknown command: nope\n");

    // `tui-shot pty ...` reaches PTY mode.
    let pty = call(&["pty", "a.yaml"]);
    assert_eq!(pty.status.code(), Some(1));
    assert_eq!(text(&pty.stderr), "pty requires -o <out.png>\n");

    if !node_or_skip() {
        return;
    }
    let out_path = dir.path().join("alias.png");
    let shot = call(&[
        "shot",
        &fixture("basic.tsx"),
        "-o",
        &out_path.to_string_lossy(),
    ]);
    assert_eq!(shot.status.code(), Some(0), "{}", combined(&shot));
    assert_eq!(
        text(&shot.stdout),
        format!("wrote {}\n", out_path.display())
    );
    assert_eq!(varied_png_size(&out_path), (435, 203));
}

#[test]
fn usage_errors_print_the_ts_message_and_exit_1() {
    let cases: &[(&[&str], &str)] = &[
        (&["ink", "shot"], "shot requires a fixture path"),
        (&["ink", "shot", "a.tsx"], "shot requires -o <out.png>"),
        (&["tui", "a.tsx"], "shot requires -o <out.png>"),
        (
            &["ink", "a.tsx", "-o", "a.jpg"],
            "Output must use a .png extension: a.jpg",
        ),
        (&["ink", "a.tsx", "-o"], "-o requires a value"),
        (&["ink", "a.tsx", "--bogus"], "Unknown flag: --bogus"),
        (
            &["ink", "shot", "a.tsx", "--width", "10"],
            "Unknown flag: --width",
        ),
        (
            &["ink", "a.tsx", "-o", "a.png", "--cols", "1001"],
            "--cols must be a positive integer no greater than 1000",
        ),
        (
            &["ink", "a.tsx", "-o", "a.png", "--rows", "2.5"],
            "--rows must be a positive integer no greater than 1000",
        ),
        (
            &["ink", "a.tsx", "-o", "a.png", "--scale", "5"],
            "--scale must be a positive number no greater than 4",
        ),
        (&["ink", "batch"], "batch requires a manifest path"),
        (&["ink", "nope"], "Unknown command: nope"),
        (
            &["ink", "install-browser", "extra"],
            "install-browser does not accept arguments",
        ),
        (&["pty", "a.yaml"], "pty requires -o <out.png>"),
        (
            &["pty", "a.yaml", "-o", "a.txt"],
            "Output must use a .png extension: a.txt",
        ),
        (&["pty", "--bogus"], "Unknown flag: --bogus"),
        (&["tui", "pty", "--out-dir"], "--out-dir requires a value"),
    ];
    for (args, message) in cases {
        let result = run(args);
        assert_eq!(result.status.code(), Some(1), "{args:?}");
        assert_eq!(text(&result.stderr), format!("{message}\n"), "{args:?}");
        assert_eq!(text(&result.stdout), "", "{args:?}");
    }

    // A manifest that does not exist is reported with its resolved path.
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let result = run_with(Path::new(BIN), &["ink", "batch", "m.yaml"], &cwd, &[]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        text(&result.stderr),
        format!(
            "ENOENT: no such file or directory, open '{}'\n",
            cwd.join("m.yaml").display()
        )
    );

    // `--help` after a fixture reaches the tui-shot CLI, which prints its own
    // help and exits 0 (`astroshot pty --help` alone is the dispatcher's).
    let help = run(&["pty", "a.yaml", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(text(&help.stdout).contains("tui-shot pty <fixture.yaml|json> -o <out.png> [options]"));
}

/// Help text `tui-shot --help` printed, captured from the TS bin.
const TUI_SHOT_HELP: &str = include_str!("fixtures/help/tui-shot.txt");

#[track_caller]
fn assert_exact(output: &Output, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(text(&output.stdout), stdout, "stdout");
    assert_eq!(text(&output.stderr), stderr, "stderr");
    assert_eq!(output.status.code(), Some(code), "exit code");
}

/// `packages/tui-shot/bin/tui-shot.mjs` ran `cli.ts` with the raw arguments;
/// the binary does the same when it is named `tui-shot`.
#[cfg(unix)]
#[test]
fn tui_shot_bin_help_and_usage_errors_match_the_ts_bin() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("tui-shot");
    std::os::unix::fs::symlink(BIN, &alias).unwrap();
    let call = |args: &[&str]| run_with(&alias, args, dir.path(), &[]);

    // No arguments: help, but a usage error.
    assert_exact(&call(&[]), TUI_SHOT_HELP, "", 1);
    // Help as the command, or as a flag after any command (known or not).
    for args in [
        &["--help"][..],
        &["-h"],
        &["help"],
        &["shot", "--help"],
        &["pty", "--help"],
        &["batch", "--help"],
        &["install-browser", "--help"],
        &["bogus", "--help"],
    ] {
        assert_exact(&call(args), TUI_SHOT_HELP, "", 0);
    }
    // tui-shot has no version flag: the first argument is always the command.
    assert_exact(&call(&["--version"]), "", "Unknown command: --version\n", 1);
    assert_exact(&call(&["-v"]), "", "Unknown command: -v\n", 1);
    assert_exact(&call(&["bogus"]), "", "Unknown command: bogus\n", 1);
    assert_exact(&call(&["--bogus"]), "", "Unknown command: --bogus\n", 1);
    // Flags after the command are parsed before the command is checked.
    assert_exact(&call(&["bogus", "--nope"]), "", "Unknown flag: --nope\n", 1);
    assert_exact(&call(&["pty", "-o"]), "", "-o requires a value\n", 1);
    // A bare fixture path is not a command here; `astroshot ink` adds `shot`.
    assert_exact(&call(&["x.tsx"]), "", "Unknown command: x.tsx\n", 1);
    assert_exact(&call(&["shot"]), "", "shot requires a fixture path\n", 1);
    assert_exact(&call(&["pty"]), "", "pty requires a fixture path\n", 1);
    assert_exact(&call(&["batch"]), "", "batch requires a manifest path\n", 1);
    assert_exact(
        &call(&["shot", "x.tsx"]),
        "",
        "shot requires -o <out.png>\n",
        1,
    );
    assert_exact(
        &call(&["install-browser", "extra"]),
        "",
        "install-browser does not accept arguments\n",
        1,
    );
}

// ---- Fix pass: sizes and failed program launches measured against TS ----

/// TS writes 832x880 for a 40x20 grid at scale 2 (`416 x 440` CSS pixels:
/// `20 * 15 * 1.32` is exactly 396 in f64). f32 arithmetic made it 882 tall.
#[test]
fn pty_png_for_a_twenty_row_grid_at_scale_two_matches_the_ts_size() {
    if !node_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("pty.png");

    let result = run(&[
        "tui",
        "pty",
        &fixture("interactive-pty.yaml"),
        "-o",
        &out_path.to_string_lossy(),
        "--cols",
        "40",
        "--rows",
        "20",
        "--scale",
        "2",
    ]);

    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert_eq!(varied_png_size(&out_path), (832, 880));
}

/// What node-pty's child leaves on the terminal when it cannot execute the
/// program: nothing on macOS (`spawn-helper` exits 1 silently), the
/// `perror("execvp(3) failed.")` line elsewhere, wrapped by the fixture's 40
/// columns.
#[cfg(unix)]
fn failed_exec_frame() -> &'static str {
    if cfg!(target_os = "macos") {
        ""
    } else {
        "execvp(3) failed.: No such file or direc\ntory"
    }
}

#[cfg(unix)]
fn write_missing_command_fixture(dir: &Path, extra: &str) -> PathBuf {
    let path = dir.join("missing.yaml");
    std::fs::write(
        &path,
        format!(
            "version: 1\ncommand: astroshot-no-such-program\ncols: 40\nrows: 6\nscale: 1\ntimeoutMs: 5000\nsettleMs: 50\n{extra}"
        ),
    )
    .unwrap();
    path
}

/// TS (`tui-shot pty` on a fixture whose command is not on PATH) prints
/// exactly this and exits 1: node-pty starts the program from inside the PTY
/// child, so the failure is an exit code, not a spawn error.
#[cfg(unix)]
#[test]
fn a_pty_command_that_is_not_on_path_is_reported_as_exit_code_1() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = write_missing_command_fixture(dir.path(), "");
    let out_path = dir.path().join("missing.png");

    let result = run(&[
        "tui",
        "pty",
        &fixture.to_string_lossy(),
        "-o",
        &out_path.to_string_lossy(),
    ]);

    assert_eq!(result.status.code(), Some(1), "{}", combined(&result));
    assert_eq!(text(&result.stdout), "");
    assert_eq!(
        text(&result.stderr),
        format!(
            "PTY program exited with code 1 before capture. Set allowNonZeroExit: true only when documenting an intentional failure state. Visible frame:\n{}\n",
            failed_exec_frame()
        )
    );
    assert!(!out_path.exists());
}

/// The same fixture while waiting for text: the wait ends on the exit.
#[cfg(unix)]
#[test]
fn waiting_for_text_from_a_pty_command_that_is_not_on_path_reports_the_exit() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = write_missing_command_fixture(dir.path(), "actions:\n  - waitFor: Ready\n");
    let out_path = dir.path().join("missing.png");

    let result = run(&[
        "tui",
        "pty",
        &fixture.to_string_lossy(),
        "-o",
        &out_path.to_string_lossy(),
    ]);

    assert_eq!(result.status.code(), Some(1), "{}", combined(&result));
    assert_eq!(
        text(&result.stderr),
        format!(
            "Timed out waiting for \"Ready\". The program exited with code 1. Visible frame:\n{}\n",
            failed_exec_frame()
        )
    );
    assert!(!out_path.exists());
}

/// With `allowNonZeroExit: true` TS captures the (empty) terminal and exits 0.
#[cfg(unix)]
#[test]
fn a_pty_command_that_is_not_on_path_is_captured_when_non_zero_exit_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = write_missing_command_fixture(dir.path(), "allowNonZeroExit: true\n");
    let out_path = dir.path().join("missing.png");

    let result = run(&[
        "tui",
        "pty",
        &fixture.to_string_lossy(),
        "-o",
        &out_path.to_string_lossy(),
    ]);

    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert_eq!(
        text(&result.stdout),
        format!("wrote {}\n", out_path.display())
    );
    assert_eq!(text(&result.stderr), "");
    // 40x6 cells at scale 1, the size TS writes for this fixture.
    assert_eq!(varied_png_size(&out_path), (416, 163));
}
