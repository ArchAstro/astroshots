//! Ports `packages/tui-shot/src/shot.test.ts` (the `takePtyShot` cases) and
//! covers the behavior of `pty-cli.e2e.test.ts` with real programs in a PTY.
//! The PTY tests use `sh` and are Unix-only.

use super::*;

/// Stands in for the binary's `__pty-exit-wrapper` subcommand: the wrapper
/// invocation re-runs this test binary on this test with the wrapper
/// arguments in `ASTROSHOT_TEST_PTY_WRAPPER_ARGS`. A normal test run (no
/// variable) passes without doing anything.
#[test]
fn pty_exit_wrapper_helper() {
    let Ok(json) = std::env::var("ASTROSHOT_TEST_PTY_WRAPPER_ARGS") else {
        return;
    };
    let args: Vec<String> = serde_json::from_str(&json).unwrap();
    let code =
        super::super::pty_exit_wrapper::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}

fn request(fixture: &Path, out: &Path) -> PtyShotRequest {
    PtyShotRequest {
        fixture_path: display(fixture),
        out_path: display(out),
        ..Default::default()
    }
}

#[tokio::test]
async fn rejects_malformed_pty_appearance_fields_before_launching_a_command() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = temp.path().join("invalid.yaml");
    fs::write(
        &fixture,
        "version: 1\ncommand: never-launched\nbackground:\n  unsafe: value\n",
    )
    .unwrap();
    let error = take_pty_shot(&request(&fixture, &temp.path().join("invalid.png")))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("background must be a string"),
        "{error}"
    );
}

#[tokio::test]
async fn rejects_a_malformed_wait_for_exit_action_before_launching_a_command() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = temp.path().join("invalid.yaml");
    fs::write(
        &fixture,
        "version: 1\ncommand: never-launched\nactions:\n  - waitForExit: false\n",
    )
    .unwrap();
    let error = take_pty_shot(&request(&fixture, &temp.path().join("invalid.png")))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("waitForExit must be true"),
        "{error}"
    );
}

#[test]
fn fixture_validation_messages_match_ts() {
    let temp = tempfile::tempdir().unwrap();
    let check = |body: &str, expected: &str| {
        let path = temp.path().join("fixture.yaml");
        fs::write(&path, body).unwrap();
        let error = load_pty_fixture(&display(&path)).unwrap_err().to_string();
        assert_eq!(
            error,
            format!("Invalid PTY fixture {}: {expected}", display(&path))
        );
    };
    check("- a\n", "the document must be an object");
    check("version: 2\ncommand: x\n", "version must be 1");
    check(
        "version: 1\ncommand: '  '\n",
        "command must be a non-empty string",
    );
    check(
        "version: 1\ncommand: x\nargs: [1]\n",
        "args must be an array of strings",
    );
    check(
        "version: 1\ncommand: x\ngraphics: sixel\n",
        "graphics must be \"kitty\" when set",
    );
    check(
        "version: 1\ncommand: x\nactions: {}\n",
        "actions must be an array",
    );
    check(
        "version: 1\ncommand: x\nactions:\n  - key: f5\n",
        "actions[0].key must be one of enter, up, down, right, left, tab, escape, backspace, space, ctrl-c, ctrl-d",
    );
    check(
        "version: 1\ncommand: x\nactions:\n  - key: enter\n    text: a\n",
        "actions[0] must set exactly one of waitFor, waitForExit, key, text, or pauseMs",
    );
    check(
        "version: 1\ncommand: x\nactions:\n  - waitFor: a\n    timeoutMs: 0\n",
        "actions[0].timeoutMs must be a positive number",
    );
    let missing = temp.path().join("missing.yaml");
    assert_eq!(
        load_pty_fixture(&display(&missing))
            .unwrap_err()
            .to_string(),
        format!("Fixture not found: {}", display(&missing))
    );
}

#[test]
fn keystrokes_match_ts() {
    assert_eq!(keystroke(PtyKey::Enter), "\r");
    assert_eq!(keystroke(PtyKey::Down), "\x1b[B");
    assert_eq!(keystroke(PtyKey::Backspace), "\x7f");
    assert_eq!(keystroke(PtyKey::CtrlD), "\x04");
}

#[test]
fn decodes_utf8_split_across_chunks() {
    let mut pending = Vec::new();
    let bytes = "a—b".as_bytes();
    assert_eq!(decode_utf8_chunk(&mut pending, &bytes[..2]), "a");
    assert_eq!(decode_utf8_chunk(&mut pending, &bytes[2..]), "—b");
    assert_eq!(decode_utf8_chunk(&mut pending, &[0xff, b'x']), "\u{fffd}x");
}

#[cfg(unix)]
mod real_programs {
    use super::*;

    /// Writes a JSON fixture running `sh -c <script>` and returns its path
    /// and the output PNG path.
    fn sh_fixture(dir: &Path, script: &str, extra: Value) -> (PathBuf, PathBuf) {
        let mut fixture = serde_json::json!({
            "version": 1,
            "command": "sh",
            "args": ["-c", script],
            "cols": 40,
            "rows": 6,
            "scale": 1,
            "timeoutMs": 10000,
            "settleMs": 50,
        });
        for (name, value) in extra.as_object().unwrap() {
            fixture[name] = value.clone();
        }
        let path = dir.join("fixture.json");
        fs::write(&path, serde_json::to_string(&fixture).unwrap()).unwrap();
        (path, dir.join("out/shot.png"))
    }

    fn decode(png: &Path) -> image::RgbaImage {
        image::load_from_memory(&fs::read(png).unwrap())
            .unwrap()
            .to_rgba8()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn printf_with_colors_renders_text_and_colored_pixels() {
        // Arrange: a program that prints true-color red text and a green word.
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            r"printf '\033[38;2;255;0;0mREDREDRED\033[0m \033[1;32mgreen\033[0m\n'",
            serde_json::json!({
                "actions": [{"waitFor": "REDREDRED"}, {"waitForExit": true}],
                "expectText": ["REDREDRED green"],
            }),
        );

        // Act: run it in a real PTY and capture.
        let capture = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap();

        // Assert: plain text, output path, PNG size and a red pixel.
        assert_eq!(capture.out_path, display(&out));
        assert_eq!(capture.plain_text, "REDREDRED green");
        let image = decode(&out);
        // ceil(40*15*.62+44) x ceil(6*15*1.32+44) at scale 1.
        assert_eq!((image.width(), image.height()), (416, 163));
        let red = image
            .pixels()
            .filter(|pixel| pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60)
            .count();
        assert!(red > 20, "expected red glyph pixels, found {red}");
        // The inner background is the default #090a12.
        assert_eq!(image.get_pixel(208, 120).0, [0x09, 0x0a, 0x12, 255]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_program_that_reads_a_key_sees_text_and_key_actions() {
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            r#"printf 'Name? '; read n; printf 'Hello %s\n' "$n""#,
            serde_json::json!({
                "actions": [
                    {"waitFor": "Name?"},
                    {"text": "bob"},
                    {"key": "enter"},
                    {"waitFor": "Hello bob"},
                    {"waitForExit": true},
                ],
                "expectText": ["Hello bob"],
            }),
        );
        let capture = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap();
        assert!(
            capture.plain_text.contains("Hello bob"),
            "{}",
            capture.plain_text
        );
        assert!(fs::metadata(&out).unwrap().len() > 1_000);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn arrow_keys_are_sent_as_escape_sequences() {
        let dir = tempfile::tempdir().unwrap();
        // `cat -v` echoes control characters visibly: ESC [ B is `^[[B`.
        let (fixture, out) = sh_fixture(
            dir.path(),
            "stty -icanon -echo; printf 'ready\\n'; head -c 3 | cat -v; printf '\\n'",
            serde_json::json!({
                "actions": [{"waitFor": "ready"}, {"key": "down"}, {"waitFor": "^[[B"}],
            }),
        );
        let capture = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap();
        assert!(
            capture.plain_text.contains("^[[B"),
            "{}",
            capture.plain_text
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_zero_exit_is_captured_and_a_crash_is_rejected_without_a_png() {
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            "printf 'Export complete\\n'",
            serde_json::json!({"actions": [{"waitForExit": true, "timeoutMs": 4000}]}),
        );
        take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap();
        assert!(out.exists());

        // A plausible final frame, then exit 7: the process is authoritative.
        let crash = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            crash.path(),
            "printf 'Fatal terminal state\\n'; exit 7",
            serde_json::json!({
                "actions": [{"waitFor": "Fatal terminal state"}, {"waitForExit": true}],
                "expectText": ["Fatal terminal state"],
            }),
        );
        let error = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(
                "PTY program exited with code 7 before capture. Set allowNonZeroExit: true only when documenting an intentional failure state. Visible frame:\nFatal terminal state"
            ),
            "{error}"
        );
        assert!(!out.exists());

        // allowNonZeroExit documents an intentional failure state.
        let (fixture, out) = sh_fixture(
            crash.path(),
            "printf 'Fatal terminal state\\n'; exit 7",
            serde_json::json!({
                "actions": [{"waitForExit": true}],
                "allowNonZeroExit": true,
            }),
        );
        take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap();
        assert!(out.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_exit_wrapper_reports_the_program_exit_code_even_when_the_marker_is_delayed() {
        // Status file is written immediately; the OSC marker is delayed so
        // waitForExit must trust the file bridge.
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            "printf 'Fatal terminal state\\n'; exit 7",
            serde_json::json!({
                "actions": [{"waitFor": "Fatal terminal state"}, {"waitForExit": true, "timeoutMs": 4000}],
                "env": {"ASTROSHOT_TEST_DELAY_PTY_EXIT_MARKER_MS": "1500"},
            }),
        );
        let error = take_isolated_pty_shot(&request(&fixture, &out), true)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("exited with code 7 before capture"),
            "{error}"
        );
        assert!(!out.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_exit_wrapper_passes_a_clean_exit_and_the_screen_through() {
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            "printf 'All artifacts are ready.\\n'",
            serde_json::json!({
                "actions": [{"waitForExit": true}],
                "expectText": ["All artifacts are ready."],
                // Wide enough that the test harness line stays on one row.
                "cols": 120,
            }),
        );
        let capture = take_isolated_pty_shot(&request(&fixture, &out), true)
            .await
            .unwrap();
        assert!(capture.plain_text.contains("All artifacts are ready."));
        assert!(out.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn waiting_for_text_that_never_appears_times_out_with_the_visible_frame() {
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            "printf 'hello\\n'; sleep 5",
            serde_json::json!({"actions": [{"waitFor": "never", "timeoutMs": 300}]}),
        );
        let error = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "Timed out waiting for \"never\". Visible frame:\nhello"
        );
        assert!(!out.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn waiting_for_exit_of_a_long_running_program_times_out_and_kills_it() {
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            "printf 'busy\\n'; sleep 30",
            serde_json::json!({"actions": [{"waitFor": "busy"}, {"waitForExit": true, "timeoutMs": 300}]}),
        );
        let started = Instant::now();
        let error = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "Timed out waiting for the PTY program to exit within 300ms. Visible frame:\nbusy"
        );
        // The finally block terminated the program rather than waiting 30s.
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_expected_text_is_rejected_with_the_visible_frame() {
        let dir = tempfile::tempdir().unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            "printf 'actual\\n'",
            serde_json::json!({"actions": [{"waitForExit": true}], "expectText": ["expected \"x\""]}),
        );
        let error = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "Fixture did not render expected text \"expected \\\"x\\\"\". Visible frame:\nactual"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_relative_cwd_resolves_against_the_fixture_and_a_bad_cwd_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("work")).unwrap();
        let (fixture, out) = sh_fixture(
            dir.path(),
            "basename \"$(pwd)\"",
            serde_json::json!({"cwd": "work", "actions": [{"waitForExit": true}]}),
        );
        let capture = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap();
        assert_eq!(capture.plain_text, "work");

        let (fixture, out) = sh_fixture(dir.path(), "true", serde_json::json!({"cwd": "nope"}));
        let error = take_isolated_pty_shot(&request(&fixture, &out), false)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            format!(
                "PTY fixture cwd is not a directory: {}",
                display(&dir.path().join("nope"))
            )
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_png_output_path_is_rejected_after_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let (fixture, _) = sh_fixture(dir.path(), "true", serde_json::json!({}));
        let bad = dir.path().join("shot.jpg");
        let error = take_isolated_pty_shot(&request(&fixture, &bad), false)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            format!("Output must use a .png extension: {}", display(&bad))
        );
    }

    #[test]
    fn graphics_mode_answers_device_queries() {
        assert_eq!(terminal_replies("\x1b[c", (0, 0), false), "\x1b[?1;2c");
        assert_eq!(
            terminal_replies("x\x1b[0cy\x1b[5n", (0, 0), false),
            "\x1b[?1;2c\x1b[0n"
        );
        assert_eq!(terminal_replies("\x1b[6n", (4, 2), false), "\x1b[3;5R");
        assert_eq!(terminal_replies("\x1b[31mred", (0, 0), false), "");
    }

    #[test]
    fn conpty_startup_query_gets_only_a_cursor_report() {
        // ConPTY asks for the cursor position and withholds output until
        // answered; the other device queries stay unanswered without graphics.
        assert_eq!(
            terminal_replies("\x1b[c\x1b[5n\x1b[6n", (0, 0), true),
            "\x1b[1;1R"
        );
        assert_eq!(terminal_replies("plain", (0, 0), true), "");
    }
}
