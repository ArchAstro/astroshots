//! Binary-level tests for the `astroshot` dispatcher. Ports
//! `packages/astroshot/test/cli.test.mjs`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_astroshot");

fn run_in(cwd: &Path, args: &[&str]) -> Output {
    run_named(BIN, cwd, args)
}

fn run_named(program: impl AsRef<std::ffi::OsStr>, cwd: &Path, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env("HOME", cwd)
        .output()
        .expect("spawn astroshot")
}

fn run(args: &[&str]) -> Output {
    run_in(&std::env::current_dir().unwrap(), args)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Help text each npm bin printed, captured from the TS bins (`<bin> --help`).
const REACT_SHOT_HELP: &str = include_str!("fixtures/help/react-shot.txt");
const TUI_SHOT_HELP: &str = include_str!("fixtures/help/tui-shot.txt");
const REVIEW_HELP: &str = include_str!("fixtures/help/astroshot-review.txt");
const MOVIE_HELP: &str = include_str!("fixtures/help/astroshot-movie.txt");
const NEEDS_TTY: &str =
    "astroshot review needs an interactive terminal (stdin and stdout must be a TTY).\n";

/// `version` from `packages/<package>/package.json`, as the TS bins read it.
fn package_version(package: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages")
        .join(package)
        .join("package.json");
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    json["version"].as_str().unwrap().to_string()
}

#[track_caller]
fn assert_exact(output: &Output, expected_stdout: &str, expected_stderr: &str, code: i32) {
    assert_eq!(stdout(output), expected_stdout, "stdout");
    assert_eq!(stderr(output), expected_stderr, "stderr");
    assert_eq!(output.status.code(), Some(code), "exit code");
}

/// The binary under an npm bin's name, as a package manager links it.
fn alias(dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::os::unix::fs::symlink(BIN, &path).unwrap();
    path
}

#[test]
fn documents_react_ink_pty_and_movie_modes_from_one_executable() {
    let result = run(&["--help"]);
    assert_eq!(result.status.code(), Some(0));
    let out = stdout(&result);
    for needle in [
        "astroshot react",
        "astroshot ink",
        "astroshot pty",
        "astroshot movie",
        "desktop.window",
        "astroshot init",
        "astroshot demo",
        "astroshot doctor",
        "astroshot review",
        "alias for \"astroshot ink\"",
        "install-browser",
    ] {
        assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
    }
}

#[test]
fn no_arguments_prints_help_and_exits_1() {
    let result = run(&[]);
    assert_eq!(result.status.code(), Some(1));
    assert!(stdout(&result).starts_with("astroshot — one CLI"));
}

#[test]
fn reports_the_unified_package_version() {
    // `cli.test.mjs` compares against packages/astroshot/package.json; so does
    // this, so a crate version that drifts from the npm package fails here.
    let result = run(&["--version"]);
    assert_exact(
        &result,
        &format!("{}\n", package_version("astroshot")),
        "",
        0,
    );
    assert_exact(&run(&["-v"]), &stdout(&result), "", 0);
}

#[test]
fn documents_each_mode_and_keeps_tui_as_an_ink_compatibility_alias() {
    let (react, ink, tui, pty) = (
        run(&["react", "--help"]),
        run(&["ink", "--help"]),
        run(&["tui", "--help"]),
        run(&["pty", "--help"]),
    );
    for result in [&react, &ink, &tui, &pty] {
        assert_eq!(result.status.code(), Some(0), "{}", stderr(result));
    }
    assert!(stdout(&react).contains("astroshot react"));
    assert!(stdout(&ink).contains("astroshot ink"));
    assert!(stdout(&tui).contains("astroshot ink"));
    assert!(stdout(&pty).contains("arbitrary terminal programs"));
    // A bare mode prints its help but is a usage error.
    assert_eq!(run(&["react"]).status.code(), Some(1));
}

#[test]
fn generates_valid_fixture_templates_for_every_capture_mode() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let react = run_in(cwd, &["init", "react"]);
    let ink = run_in(cwd, &["init", "ink", "screens/ready.tsx"]);
    let pty = run_in(cwd, &["init", "pty"]);
    let pty_json = run_in(cwd, &["init", "pty", "screens/ready.json"]);
    for result in [&react, &ink, &pty, &pty_json] {
        assert_eq!(result.status.code(), Some(0), "{}", stderr(result));
    }
    assert!(stdout(&react).starts_with("Created React fixture: "));

    let read = |name: &str| fs::read_to_string(cwd.join(name)).unwrap();
    assert!(read("react.shot.tsx").contains("ReactShotFixture"));
    assert!(read("screens/ready.tsx").contains("InkShotFixture"));
    assert!(read("pty.shot.yaml").contains("command: ./target/debug/my-tui"));
    let json: serde_json::Value = serde_json::from_str(&read("screens/ready.json")).unwrap();
    assert_eq!(json["command"], "./target/debug/my-tui");

    let collision = run_in(cwd, &["init", "react"]);
    assert_eq!(collision.status.code(), Some(1));
    assert!(stderr(&collision).contains("Refusing to overwrite"));
    let forced = run_in(cwd, &["init", "react", "--force"]);
    assert_eq!(forced.status.code(), Some(0), "{}", stderr(&forced));

    let sensitive = cwd.join("sensitive.txt");
    fs::write(&sensitive, "do not replace").unwrap();
    std::os::unix::fs::symlink(&sensitive, cwd.join("linked.tsx")).unwrap();
    let symlink = run_in(cwd, &["init", "react", "linked.tsx", "--force"]);
    assert_eq!(symlink.status.code(), Some(1));
    assert!(stderr(&symlink).contains("Refusing to replace symbolic link"));
    assert_eq!(fs::read_to_string(&sensitive).unwrap(), "do not replace");
}

#[test]
fn init_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    let bare = run_in(dir.path(), &["init"]);
    assert_eq!(bare.status.code(), Some(1));
    assert!(stdout(&bare).contains("astroshot init — generate a fixture template"));
    let help = run_in(dir.path(), &["init", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    let flag = run_in(dir.path(), &["init", "react", "--nope"]);
    assert_eq!(flag.status.code(), Some(1));
    assert!(stderr(&flag).contains("Unknown init flag: --nope"));
    let many = run_in(dir.path(), &["init", "react", "a.tsx", "b.tsx"]);
    assert_eq!(many.status.code(), Some(1));
    assert!(stderr(&many).contains("init accepts at most one output path"));
}

#[test]
fn documents_demo_and_doctor_as_the_first_win_commands() {
    let demo = run(&["demo", "--help"]);
    let doctor = run(&["doctor", "--help"]);
    assert_eq!(demo.status.code(), Some(0), "{}", stderr(&demo));
    assert_eq!(doctor.status.code(), Some(0), "{}", stderr(&doctor));
    assert!(stdout(&demo).contains("no prerequisites"));
    assert!(stdout(&demo).contains("--feature"));
    assert!(stdout(&doctor).contains("read-only"));
    assert!(stdout(&doctor).contains("Exits non-zero when a required check fails"));
}

#[test]
fn demo_dry_run_lists_files_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_in(dir.path(), &["demo", "--dry-run"]);
    assert_eq!(result.status.code(), Some(0), "{}", stderr(&result));
    assert!(stdout(&result).starts_with("astroshot demo → "));
    assert!(stdout(&result).contains("manifest.json"));
    assert!(!dir.path().join(".astroshot").exists());
}

#[test]
fn demo_usage_error_prints_message_and_help() {
    let result = run(&["demo", "--feature"]);
    assert_eq!(result.status.code(), Some(1));
    let err = stderr(&result);
    assert!(err.starts_with("--feature requires a value\n\n"), "{err}");
    assert!(err.contains("no prerequisites"));
}

#[test]
fn doctor_json_has_the_documented_shape() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let result = run_in(dir.path(), &["doctor", "--json", "--root", root]);
    let report: serde_json::Value = serde_json::from_str(&stdout(&result))
        .unwrap_or_else(|e| panic!("{e}: {}", stdout(&result)));
    assert!(report["project"].is_string());
    assert!(report["ok"].is_boolean());
    assert!(report["failures"].is_array());
    assert!(report["warnings"].is_array());
    let checks = report["checks"].as_array().unwrap();
    assert!(!checks.is_empty());
    for check in checks {
        for key in ["id", "title", "required", "status", "detail", "remediation"] {
            assert!(check.get(key).is_some(), "{check} lacks {key}");
        }
    }
    let ok = report["ok"].as_bool().unwrap();
    assert_eq!(result.status.code(), Some(if ok { 0 } else { 1 }));
}

#[test]
fn rejects_an_unknown_screenshot_mode() {
    let result = run(&["browser"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(stderr(&result).contains("Unknown command: browser"));
    assert!(stdout(&result).contains("astroshot — one CLI"));
}

#[test]
fn install_browser_reports_the_chrome_it_found_or_the_hint() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("chrome");
    fs::write(&fake, "").unwrap();
    let found = Command::new(BIN)
        .arg("install-browser")
        .env("ASTROSHOT_CHROME", &fake)
        .output()
        .unwrap();
    assert_eq!(found.status.code(), Some(0));
    assert!(stdout(&found).contains(fake.to_str().unwrap()));

    let missing = Command::new(BIN)
        .arg("install-browser")
        .env("ASTROSHOT_CHROME", dir.path().join("nope"))
        .env("CHROME_PATH", dir.path().join("nope"))
        .env("PATH", "")
        .env("HOME", dir.path())
        .output()
        .unwrap();
    // A machine with Chrome under /Applications still finds it.
    if missing.status.code() == Some(1) {
        assert!(stderr(&missing).contains("ASTROSHOT_CHROME"));
    }
}

#[test]
fn movie_which_source_steers_agents_to_the_right_capture_path() {
    let tui = run(&["movie", "which-source", "ratatui truecolor dashboard"]);
    assert_eq!(tui.status.code(), Some(0), "{}", stderr(&tui));
    assert!(stdout(&tui).contains("\"recommended\": \"pty\""));

    let native = run(&[
        "movie",
        "which-source",
        "SwiftUI native app window bundle id",
    ]);
    assert_eq!(native.status.code(), Some(0), "{}", stderr(&native));
    assert!(stdout(&native).contains("\"recommended\": \"desktop.window\""));

    let help = run(&["movie", "--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", stderr(&help));
    assert!(stdout(&help).contains("Which --source should I use"));
    assert!(stdout(&help).contains("NEVER screenshot Terminal"));
}

#[test]
fn review_documents_the_terminal_tray_and_refuses_to_run_without_a_tty() {
    let help = run(&["review", "--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", stderr(&help));
    assert!(stdout(&help).contains("Astroshots tray in your terminal"));
    assert!(stdout(&help).contains("Kitty"));
    assert!(stdout(&help).contains("herdr"));
    assert!(stdout(&help).contains("--root <dir>"));

    let bare_help = run(&["review", "help"]);
    assert_eq!(bare_help.status.code(), Some(0), "{}", stderr(&bare_help));
    assert!(stdout(&bare_help).contains("astroshot review"));
    assert_eq!(stdout(&bare_help), stdout(&help));

    let package_root = env!("CARGO_MANIFEST_DIR");
    let piped = run(&["review", "--root", package_root]);
    assert_eq!(piped.status.code(), Some(1));
    assert_eq!(
        stderr(&piped),
        "astroshot review needs an interactive terminal (stdin and stdout must be a TTY).\n"
    );
    assert_eq!(stdout(&piped), "");

    // `tray` is the same command.
    let tray = run(&["tray"]);
    assert_eq!(tray.status.code(), Some(1));
    assert!(stderr(&tray).contains("interactive terminal"));
}

#[test]
fn review_usage_errors_print_the_message_then_the_help() {
    let unknown = run(&["review", "--nope"]);
    assert_eq!(unknown.status.code(), Some(1));
    let help = stdout(&run(&["review", "--help"]));
    assert_eq!(
        stderr(&unknown),
        format!("Unknown option: --nope\n\n{help}")
    );
    assert_eq!(stdout(&unknown), "");

    let missing = run(&["review", "--root"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).starts_with("--root requires a directory\n\n"));

    let version = run(&["review", "--root", "x", "--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        stdout(&version),
        format!("{}\n", package_version("astroshot-review"))
    );
}

#[test]
fn react_shot_alias_runs_the_react_shot_cli_with_its_own_help() {
    let dir = tempfile::tempdir().unwrap();
    let react = alias(dir.path(), "react-shot");
    let call = |args: &[&str]| run_named(&react, dir.path(), args);

    // Bare is a usage error that prints react-shot's help, not the
    // dispatcher's "astroshot react" mode help.
    assert_exact(&call(&[]), REACT_SHOT_HELP, "", 1);
    for args in [&["--help"][..], &["-h"], &["help"]] {
        assert_exact(&call(args), REACT_SHOT_HELP, "", 0);
    }
    assert!(
        REACT_SHOT_HELP.starts_with("react-shot - deterministic React component screenshots\n")
    );
    for needle in [
        "react-shot install-browser [--with-deps]",
        "--with-deps ",
        "-v, --version ",
    ] {
        assert!(REACT_SHOT_HELP.contains(needle), "{needle}");
    }
    assert_exact(
        &call(&["--version"]),
        &format!("{}\n", package_version("react-shot")),
        "",
        0,
    );
    assert_exact(
        &call(&["--root", "x"]),
        "",
        "Unknown command: undefined\n",
        1,
    );
    // The dispatcher's own mode help is unchanged.
    assert!(stdout(&run(&["react", "--help"])).starts_with("astroshot react — "));
}

#[test]
fn tui_shot_alias_runs_the_tui_shot_cli_without_fixture_normalisation() {
    let dir = tempfile::tempdir().unwrap();
    let tui = alias(dir.path(), "tui-shot");
    let call = |args: &[&str]| run_named(&tui, dir.path(), args);

    assert_exact(&call(&[]), TUI_SHOT_HELP, "", 1);
    for args in [&["--help"][..], &["-h"], &["help"], &["pty", "--help"]] {
        assert_exact(&call(args), TUI_SHOT_HELP, "", 0);
    }
    assert!(TUI_SHOT_HELP.starts_with(
        "tui-shot — deterministic PNG screenshots of terminal interfaces\n\nUsage:\n"
    ));
    assert_eq!(TUI_SHOT_HELP.matches("\n  tui-shot ").count(), 4);

    // `tui-shot pty` is a usage error of the tui-shot CLI, not pty mode help.
    assert_exact(&call(&["pty"]), "", "pty requires a fixture path\n", 1);
    assert_exact(
        &call(&["pty", "a.yaml"]),
        "",
        "pty requires -o <out.png>\n",
        1,
    );
    // Only `astroshot ink <fixture.tsx>` inserts `shot`; the bin does not.
    assert_exact(&call(&["x.tsx"]), "", "Unknown command: x.tsx\n", 1);
    assert_exact(
        &run(&["ink", "x.tsx"]),
        "",
        "shot requires -o <out.png>\n",
        1,
    );
    assert_exact(&call(&["--root", "x"]), "", "Unknown command: --root\n", 1);
    assert!(stdout(&run(&["pty", "--help"])).starts_with("astroshot pty — "));
}

#[test]
fn astroshot_review_alias_parses_its_arguments_like_the_review_bin() {
    let dir = tempfile::tempdir().unwrap();
    let review = alias(dir.path(), "astroshot-review");
    let call = |args: &[&str]| run_named(&review, dir.path(), args);

    assert_exact(&call(&["--help"]), REVIEW_HELP, "", 0);
    // `help` is a folder name to the bin; only `astroshot review help` is help.
    assert_exact(&call(&["help"]), "", NEEDS_TTY, 1);
    assert_exact(&run(&["review", "help"]), REVIEW_HELP, "", 0);
    // The bin parses every argument before it honours --help.
    let invalid = format!("--roots-source must be app, cli, or cwd\n\n{REVIEW_HELP}");
    assert_exact(
        &call(&["--roots-source", "bogus", "--help"]),
        "",
        &invalid,
        1,
    );
    assert_exact(
        &run(&["review", "--roots-source", "bogus", "--help"]),
        REVIEW_HELP,
        "",
        0,
    );
    assert_exact(&call(&["--root", "x"]), "", NEEDS_TTY, 1);
}

#[test]
fn astroshot_movie_alias_runs_the_movie_cli() {
    let dir = tempfile::tempdir().unwrap();
    let movie = alias(dir.path(), "astroshot-movie");
    let call = |args: &[&str]| run_named(&movie, dir.path(), args);

    // No arguments is help with exit 0, as the npm bin behaved.
    for args in [&[][..], &["--help"], &["-h"], &["help"]] {
        assert_exact(&call(args), MOVIE_HELP, "", 0);
    }
    let result = call(&["which-source", "ratatui dashboard"]);
    assert_eq!(result.status.code(), Some(0), "{}", stderr(&result));
    assert!(stdout(&result).contains("\"recommended\": \"pty\""));
}

#[test]
fn install_browser_forwards_its_arguments_to_the_react_shot_cli() {
    assert_exact(&run(&["install-browser", "--help"]), REACT_SHOT_HELP, "", 0);
    assert_exact(&run(&["install-browser", "-h"]), REACT_SHOT_HELP, "", 0);
    assert_exact(
        &run(&["install-browser", "--bogus"]),
        "",
        "Unknown option: --bogus\n",
        1,
    );
    assert_exact(
        &run(&["install-browser", "--version"]),
        &format!("{}\n", package_version("react-shot")),
        "",
        0,
    );
}

#[test]
fn a_symlink_named_astroshot_behaves_like_the_binary_itself() {
    let dir = tempfile::tempdir().unwrap();
    let plain = alias(dir.path(), "astroshot");
    let result = run_named(&plain, dir.path(), &["--version"]);
    assert_exact(
        &result,
        &format!("{}\n", package_version("astroshot")),
        "",
        0,
    );
    assert_exact(
        &run_named(&plain, dir.path(), &["react", "--help"]),
        &stdout(&run(&["react", "--help"])),
        "",
        0,
    );
}

#[test]
fn pty_exit_wrapper_is_a_hidden_subcommand() {
    let result = run(&["__pty-exit-wrapper"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(stderr(&result).contains("Astroshot PTY status wrapper requires"));
}
