//! Port of `packages/react-shot/src/cli.e2e.test.ts`: the real binary renders a
//! TSX fixture through the Node helper and Chrome, and rejects colliding batch
//! outputs before capturing. Skipped (with a printed reason) when Chrome or
//! Node is missing.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use astroshot_engine::browser::find_chrome;
use astroshot_engine::node_helper::find_node;

fn package_root() -> PathBuf {
    // Not canonicalized: the worktree's node_modules may be a symlink.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("packages/react-shot")
}

fn tools_or_skip() -> bool {
    if let Err(error) = find_chrome() {
        common::skip(&error);
        return false;
    }
    if let Err(error) = find_node() {
        common::skip(&error);
        return false;
    }
    true
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_astroshot"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn renders_a_tsx_release_card_through_vite_and_chromium_into_a_cropped_rgba_png() {
    if !tools_or_skip() {
        return;
    }
    // Setup: drive the built binary, not an in-process API.
    let dir = tempfile::tempdir().unwrap();
    let output_path = dir.path().join("release-card.png");
    let fixture = package_root().join("fixtures/cli-e2e.tsx");

    // Boundary crossing: the binary starts the Node helper (Vite) which serves
    // the TSX module to a real Chrome process.
    let result = run(
        &[
            "react",
            "shot",
            &fixture.to_string_lossy(),
            "--out",
            &output_path.to_string_lossy(),
        ],
        &package_root(),
    );

    // Observable outcomes: success and an element-cropped, alpha-capable PNG
    // rather than the 800x600 dimmer.
    let combined = format!("{}\n{}", text(&result.stdout), text(&result.stderr));
    assert_eq!(result.status.code(), Some(0), "{combined}");
    assert!(
        text(&result.stdout).contains(&format!("wrote {}", output_path.display())),
        "{combined}"
    );
    let png = std::fs::read(&output_path).unwrap();
    assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    assert_eq!(be(16), 400);
    assert!(be(20) > 60);
    assert!(be(20) < 180);
    assert_eq!(png[25], 6);
}

#[test]
fn rejects_colliding_batch_outputs_before_capturing_the_first_entry() {
    if !tools_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fixture = package_root().join("fixtures/cli-e2e.tsx");
    let manifest_path = dir.path().join("batch.json");
    let first_output = dir.path().join("result/screen.png");
    std::fs::write(
        &manifest_path,
        serde_json::json!({
            "shots": [
                { "fixture": fixture, "out": "result/screen.png" },
                { "fixture": fixture, "out": "RESULT/SCREEN.PNG" },
            ]
        })
        .to_string(),
    )
    .unwrap();

    let result = run(
        &["react", "batch", &manifest_path.to_string_lossy()],
        &package_root(),
    );

    assert_eq!(result.status.code(), Some(1));
    assert!(text(&result.stderr).contains("same destination"));
    assert!(!first_output.exists());
}

#[test]
fn react_shot_argv0_alias_prints_the_react_shot_help_and_bare_fixture_form_works() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("react-shot");
    #[cfg(unix)]
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_astroshot"), &alias).unwrap();
    #[cfg(not(unix))]
    return;

    let help = Command::new(&alias)
        .args(["shot", "--help"])
        .output()
        .unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(
        text(&help.stdout).starts_with("react-shot - deterministic React component screenshots")
    );

    let missing_out = Command::new(&alias)
        .args(["fixture.tsx"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(missing_out.status.code(), Some(1));
    assert_eq!(
        text(&missing_out.stderr).trim(),
        "A screenshot requires -o <out.png>"
    );

    let unknown = Command::new(&alias).args(["nope"]).output().unwrap();
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(text(&unknown.stderr).trim(), "Unknown command: nope");
}

/// Help text `react-shot --help` printed, captured from the TS bin.
const REACT_SHOT_HELP: &str = include_str!("fixtures/help/react-shot.txt");

#[track_caller]
fn assert_exact(output: &Output, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(text(&output.stdout), stdout, "stdout");
    assert_eq!(text(&output.stderr), stderr, "stderr");
    assert_eq!(output.status.code(), Some(code), "exit code");
}

/// `packages/react-shot/bin/react-shot.mjs` ran `cli.ts` with the raw
/// arguments; the binary does the same when it is named `react-shot`.
#[cfg(unix)]
#[test]
fn react_shot_bin_help_version_and_usage_errors_match_the_ts_bin() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("react-shot");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_astroshot"), &alias).unwrap();
    let call = |args: &[&str]| {
        Command::new(&alias)
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    let package_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(package_root().join("package.json")).unwrap(),
    )
    .unwrap();
    let version = format!("{}\n", package_json["version"].as_str().unwrap());

    // No arguments: help, but a usage error.
    assert_exact(&call(&[]), REACT_SHOT_HELP, "", 1);
    // Help wins wherever the flag sits, including after a subcommand.
    for args in [
        &["--help"][..],
        &["-h"],
        &["help"],
        &["shot", "--help"],
        &["batch", "--help"],
        &["install-browser", "--help"],
        &["x.tsx", "--help"],
        &["bogus", "-h"],
    ] {
        assert_exact(&call(args), REACT_SHOT_HELP, "", 0);
    }
    // Version is checked before help.
    for args in [
        &["--version"][..],
        &["-v"],
        &["-v", "--help"],
        &["--help", "-v"],
    ] {
        assert_exact(&call(args), &version, "", 0);
    }
    // Flags are parsed before the command is looked at.
    assert_exact(&call(&["bogus"]), "", "Unknown command: bogus\n", 1);
    assert_exact(&call(&["--bogus"]), "", "Unknown option: --bogus\n", 1);
    assert_exact(
        &call(&["help", "--bogus"]),
        "",
        "Unknown option: --bogus\n",
        1,
    );
    assert_exact(&call(&["--root"]), "", "--root requires a value\n", 1);
    assert_exact(
        &call(&["--root", "x"]),
        "",
        "Unknown command: undefined\n",
        1,
    );
    assert_exact(&call(&["shot"]), "", "shot requires a fixture path\n", 1);
    assert_exact(&call(&["batch"]), "", "batch requires a manifest path\n", 1);
    assert_exact(
        &call(&["x.tsx"]),
        "",
        "A screenshot requires -o <out.png>\n",
        1,
    );
    assert_exact(
        &call(&["shot", "x.tsx"]),
        "",
        "A screenshot requires -o <out.png>\n",
        1,
    );
}
