//! Binary-level tests for `astroshot review`. Ports
//! `packages/astroshot-review/src/review.e2e.test.ts` and
//! `packages/astroshot-review/src/capture.e2e.test.ts`, which ran the published
//! bin; here the built `astroshot` binary is the process under test.
//!
//! Like the TS suite, the tray runs inside a real pseudoterminal whose other
//! end emulates a kitty-graphics terminal (`KittyGraphicsTracker` over the
//! headless terminal), and the tests assert that pictures land where the rows
//! are and that `review.json` changes as the macOS app would write it. Every
//! wait polls the emulated screen (or the disk) against a deadline.
//!
//! `herdr.e2e.test.ts` is ported in `tests/review_herdr.rs`.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use astroshot::raster::HeadlessTerminal;
use astroshot::tui_shot::kitty_graphics::{
    GraphicsOverlay, KittyGraphicsTracker, KittyTrackerOptions,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const BIN: &str = env!("CARGO_BIN_EXE_astroshot");

fn demo_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/astroshot/fixtures/demo")
        .join(name)
}

const MANIFEST: &str = r#"{"version":1,"feature":"checkout","run_id":"checkout-e2e","status":"pass","shots":[{"id":"0001","file":"0001-welcome.png","slug":"welcome","title":"Welcome","description":"Landing state.","captured_at":"2026-09-05T14:10:00Z"},{"id":"0002","file":"0002-next-steps.png","slug":"next-steps","title":"Next steps","description":"Confirmation.","captured_at":"2026-09-05T14:12:00Z"},{"id":"0003","kind":"movie","file":"0003-journey.png","video":"0003-journey.webm","slug":"journey","title":"Journey","duration_ms":4240,"source":"frames","captured_at":"2026-09-05T14:20:00Z","chapters":[{"slug":"poster","t_ms":2685}]}]}"#;

/// `seedRoot(root)`: one feature with two stills and a movie.
fn seed_root(root: &Path) -> PathBuf {
    let feature = root.join("demo-app/.astroshot/checkout");
    fs::create_dir_all(&feature).unwrap();
    for (from, to) in [
        ("welcome.png", "0001-welcome.png"),
        ("next-steps.png", "0002-next-steps.png"),
        ("journey.png", "0003-journey.png"),
        ("journey.webm", "0003-journey.webm"),
    ] {
        fs::copy(demo_fixture(from), feature.join(to)).unwrap();
    }
    fs::write(feature.join("manifest.json"), MANIFEST).unwrap();
    feature
}

struct Emulator {
    tracker: KittyGraphicsTracker,
    raw: String,
    /// Bytes of a UTF-8 sequence split across two reads.
    partial: Vec<u8>,
}

struct Session {
    emulator: Arc<Mutex<Emulator>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
}

/// `launch(root, cacheDir)`: the tray in a 140x40 kitty-capable PTY.
fn launch(root: &Path, cache_dir: &Path) -> Session {
    launch_sized(root, cache_dir, 140, 40)
}

fn launch_sized(root: &Path, cache_dir: &Path, cols: u16, rows: u16) -> Session {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(BIN);
    command.args(["review", "--root", &root.to_string_lossy(), "--no-index"]);
    command.cwd(root);
    // This suite emulates a Kitty terminal via the tracker, so force kitty and
    // drop any mosh/tmux/herdr vars the outer session may carry (they would
    // otherwise route the tray to half-block text).
    command.env("TERM", "xterm-kitty");
    command.env("ASTROSHOT_REVIEW_GRAPHICS", "kitty");
    command.env("ASTROSHOT_REVIEW_CACHE_DIR", cache_dir);
    command.env("ASTROSHOT_REVIEW_FFMPEG", "");
    for name in [
        "TMUX",
        "MOSH_SERVER_NETWORK_TMOUT",
        "MOSH_CONNECTION",
        "MOSH_KEY",
        "HERDR_ENV",
        "HERDR_PANE_ID",
        "HERDR_SOCKET_PATH",
        "ASTROSHOT_REVIEW_DEBUG",
        "ASTROSHOT_REVIEW_CELL_PX",
    ] {
        command.env_remove(name);
    }
    let child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(pair.master.take_writer().unwrap()));

    let reply_writer = writer.clone();
    let tracker = KittyGraphicsTracker::new(KittyTrackerOptions {
        terminal: HeadlessTerminal::new(cols, rows),
        cols: u32::from(cols),
        rows: u32::from(rows),
        cell_width: 9,
        cell_height: 20,
        reply: Box::new(move |data| {
            // After the child exits there is nobody to answer.
            let mut writer = reply_writer.lock().unwrap();
            let _ = writer.write_all(data.as_bytes());
            let _ = writer.flush();
        }),
    });
    let emulator = Arc::new(Mutex::new(Emulator {
        tracker,
        raw: String::new(),
        partial: Vec::new(),
    }));
    let sink = emulator.clone();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 65536];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    let mut emulator = sink.lock().unwrap();
                    let mut bytes = std::mem::take(&mut emulator.partial);
                    bytes.extend_from_slice(&buffer[..read]);
                    let valid = match std::str::from_utf8(&bytes) {
                        Ok(_) => bytes.len(),
                        Err(error) => error.valid_up_to(),
                    };
                    // Keep a trailing incomplete sequence; pass anything else on.
                    let cut = if bytes.len() - valid < 4 {
                        valid
                    } else {
                        bytes.len()
                    };
                    let text = String::from_utf8_lossy(&bytes[..cut]).into_owned();
                    emulator.partial = bytes[cut..].to_vec();
                    emulator.raw.push_str(&text);
                    emulator.tracker.write(&text);
                }
            }
        }
    });
    Session {
        emulator,
        writer,
        child,
        _master: pair.master,
    }
}

impl Session {
    fn write(&self, data: &str) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(data.as_bytes()).unwrap();
        writer.flush().unwrap();
    }

    fn screen(&self) -> String {
        self.emulator
            .lock()
            .unwrap()
            .tracker
            .terminal()
            .plain_text()
    }

    fn output(&self) -> String {
        self.emulator.lock().unwrap().raw.clone()
    }

    fn overlays(&self) -> Vec<GraphicsOverlay> {
        self.emulator.lock().unwrap().tracker.overlays().unwrap()
    }

    fn mode(&self) -> TermMode {
        *self
            .emulator
            .lock()
            .unwrap()
            .tracker
            .terminal()
            .screen()
            .mode()
    }

    /// Poll `predicate` until it holds; panics with the screen after 20 s.
    fn wait_for(&self, label: &str, mut predicate: impl FnMut(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if predicate(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("Timed out waiting for {label}. Screen:\n{}", self.screen());
    }

    fn wait_for_screen(&self, text: &str, label: &str) {
        self.wait_for(label, |session| session.screen().contains(text));
    }

    /// The child's exit code, waiting up to `timeout` for it to exit.
    fn wait_exit(&mut self, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status.exit_code());
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Leave any takeover or detail page first; `q` quits only from a list.
    fn close(&mut self) -> Option<u32> {
        for key in ["\x1b", "\x1b", "q"] {
            if let Some(code) = self.wait_exit(Duration::ZERO) {
                return Some(code);
            }
            self.write(key);
            // Pacing between keys, as in the TS; exit is awaited below.
            if let Some(code) = self.wait_exit(Duration::from_millis(150)) {
                return Some(code);
            }
        }
        let code = self.wait_exit(Duration::from_secs(8));
        if code.is_none() {
            let _ = self.child.kill();
        }
        code
    }

    /// Wait until the reader thread has seen everything the child wrote
    /// before it exited: the output stops growing.
    fn drained_output(&self) -> String {
        let mut last = self.output();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            let now = self.output();
            if now == last {
                break;
            }
            last = now;
        }
        last
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn read_review(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn is_sha256(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')))
}

fn temp_root() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("astroshot-review-e2e-")
        .tempdir()
        .unwrap()
}

// ---- review.e2e.test.ts: "astroshot review in a kitty-capable PTY" ---------------

#[test]
fn streams_shots_with_thumbnails_marks_seen_and_records_feedback_like_the_app() {
    let root = temp_root();
    let cache_dir = root.path().join(".cache");
    let feature = seed_root(root.path());
    let mut session = launch(root.path(), &cache_dir);

    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let session = &session;
        session.wait_for_screen("Unseen (3)", "the seeded stream");
        assert!(session.output().contains("\x1b[?1049h"));
        session.wait_for("thumbnails and the detail preview", |s| {
            s.overlays().len() >= 4
        });
        // Thumbnails sit below the header/tab/filter rows, to the right of the marker column.
        for overlay in session.overlays() {
            assert!(overlay.row >= 3.0, "row {}", overlay.row);
            assert!(overlay.col >= 1.0, "col {}", overlay.col);
            assert!(overlay.cols > 0.0);
            assert!(overlay.rows > 0.0);
        }
        let screen = session.screen();
        assert!(screen.contains("checkout · Journey"), "{screen}");
        assert!(screen.contains("Movie · 4.2s"), "{screen}");
        assert!(screen.contains("● Unseen"), "{screen}");

        // Newest first: the movie leads. Feedback goes into review.json as a comment-only entry.
        session.write("c");
        session.wait_for_screen("Share feedback", "the composer");
        session.write("Needs more contrast\r");
        let review_path = feature.join("review.json");
        session.wait_for("the comment to land", |s| {
            let screen = s.screen();
            review_path.exists()
                && screen.contains("Reviewer")
                && !screen.contains("Share feedback")
        });
        let review = read_review(&review_path);
        assert_eq!(review["version"], 1);
        assert_eq!(review["run_id"], "checkout-e2e");
        let entry = &review["reviews"]["0003-journey.png"];
        assert_eq!(entry["comments"][0]["body"], "Needs more contrast");
        assert!(entry.get("decision").is_none(), "{entry}");

        // Seen removes the row from Unseen and records the hash.
        session.write("s");
        session.wait_for_screen("Unseen (2)", "the seen count to drop");
        session.wait_for("the seen decision on disk", |_| {
            read_review(&review_path)["reviews"]["0003-journey.png"]["decision"] == "seen"
        });
        let review = read_review(&review_path);
        let entry = &review["reviews"]["0003-journey.png"];
        assert_eq!(entry["decision"], "seen");
        assert!(is_sha256(&entry["image_sha256"]), "{entry}");
        assert_eq!(entry["comments"].as_array().unwrap().len(), 1);
        // The bytes are the app's format: sorted keys, two-space indent,
        // second-precision UTC stamps, an upper-case UUID, a trailing newline.
        let bytes = fs::read_to_string(&review_path).unwrap();
        let stamp = r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z";
        let expected = regex::escape(
            r#"{
  "reviews": {
    "0003-journey.png": {
      "comments": [
        {
          "body": "Needs more contrast",
          "created_at": "<stamp>",
          "id": "<uuid>"
        }
      ],
      "decision": "seen",
      "image_sha256": "<sha>",
      "reviewed_at": "<stamp>"
    }
  },
  "run_id": "checkout-e2e",
  "updated_at": "<stamp>",
  "version": 1
}
"#,
        )
        .replace("<stamp>", stamp)
        .replace(
            "<uuid>",
            "[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}",
        )
        .replace("<sha>", "[0-9a-f]{64}");
        assert!(
            regex::Regex::new(&format!("^{expected}$"))
                .unwrap()
                .is_match(&bytes),
            "{bytes}"
        );

        // History shows it again as Seen.
        session.write("u");
        session.wait_for_screen("History (1)", "the history filter");
        assert!(session.screen().contains("● Seen"));

        // Full-screen review pages run siblings oldest → newest, seen ones included.
        session.write("u");
        session.wait_for_screen("Unseen (2)", "back to unseen");
        session.write("f");
        session.wait_for_screen("Full-screen review", "the takeover");
        assert!(session.screen().contains("2 / 3"), "{}", session.screen());
        assert!(session.screen().contains("Next steps"));
        session.write("\x1b[D");
        session.wait_for_screen("1 / 3", "the older sibling");
        assert!(session.screen().contains("Welcome"));
        session.write("\x1b[C");
        session.wait_for_screen("2 / 3", "the middle sibling");
        session.write("\x1b[C");
        session.wait_for_screen("3 / 3", "the newest sibling");
        assert!(session.screen().contains("Journey"));
    }));

    // `finally`: the tray quits cleanly whatever happened above.
    let code = session.close();
    let output = session.drained_output();
    if let Err(panic) = body {
        std::panic::resume_unwind(panic);
    }
    assert_eq!(code, Some(0));
    assert!(output.contains("a=d,d=A"));
    assert!(output.contains("\x1b[?1049l"));
}

#[test]
fn ingests_a_new_capture_while_running_and_shows_friction_logs() {
    let root = temp_root();
    let cache_dir = root.path().join(".cache");
    let feature = seed_root(root.path());
    let friction_log = root
        .path()
        .join("demo-app/.astroshot/friction-logs/onboarding");
    let friction_run = friction_log.join("runs/20260811T153000Z");
    fs::create_dir_all(&friction_run).unwrap();
    fs::write(friction_log.join("prompt.md"), "# Onboarding\n").unwrap();
    fs::copy(
        demo_fixture("welcome.png"),
        friction_run.join("0001-land.png"),
    )
    .unwrap();
    fs::write(
        friction_run.join("log.jsonl"),
        "{\"step\":1,\"id\":\"land\",\"title\":\"Land on home\",\"transcript\":\"I land on the home page.\",\"screenshots\":[\"0001-land.png\"],\"good\":[\"Fast\"],\"improve\":[\"Copy is vague\"]}\n",
    )
    .unwrap();
    let mut session = launch(root.path(), &cache_dir);

    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let session = &session;
        session.wait_for_screen("Unseen (3)", "the seeded stream");
        fs::copy(
            demo_fixture("next-steps.png"),
            feature.join("0004-arrived.png"),
        )
        .unwrap();
        session.wait_for("the live arrival", |s| {
            let screen = s.screen();
            screen.contains("Unseen (4)") && screen.contains("checkout · Arrived")
        });
        assert!(session.screen().contains("1 new"), "{}", session.screen());

        session.write("2");
        session.wait_for_screen("Onboarding", "the friction list");
        assert!(session.screen().contains("1 step · 1 improve"));
        session.write("\r");
        session.wait_for_screen("Improve rollup · 1", "the log detail");
        session.write("\r");
        session.wait_for_screen("I land on the home page.", "the step detail");
        assert!(session.screen().contains("Copy is vague"));
        session.wait_for("the step screenshot", |s| !s.overlays().is_empty());
        // Seen lives on the log, like the app's list row and detail header.
        session.write("\x1b");
        session.wait_for_screen("Improve rollup · 1", "back to the log detail");
        session.write("s");
        let sidecar = friction_run.join("review.json");
        session.wait_for("the friction review sidecar", |_| sidecar.exists());
        let review = read_review(&sidecar);
        assert_eq!(review["run_id"], "20260811T153000Z");
        assert_eq!(review["reviews"]["log.jsonl"]["decision"], "seen");
    }));

    session.close();
    if let Err(panic) = body {
        std::panic::resume_unwind(panic);
    }
}

// ---- Quit and teardown ------------------------------------------------------------

/// The terminal is back on the main screen with the cursor shown, both by
/// the bytes written and by the emulated terminal's modes.
fn assert_terminal_restored(session: &Session) {
    let output = session.drained_output();
    let entered = output
        .rfind("\x1b[?1049h")
        .expect("entered the alternate screen");
    let left = output
        .rfind("\x1b[?1049l")
        .expect("left the alternate screen");
    assert!(
        left > entered,
        "the last alternate-screen switch must be a leave"
    );
    let shown = output.rfind("\x1b[?25h").expect("showed the cursor");
    assert!(
        output
            .rfind("\x1b[?25l")
            .is_none_or(|hidden| hidden < shown),
        "the cursor must end up shown"
    );
    let mode = session.mode();
    assert!(!mode.contains(TermMode::ALT_SCREEN), "{mode:?}");
    assert!(mode.contains(TermMode::SHOW_CURSOR), "{mode:?}");
    assert!(!mode.contains(TermMode::BRACKETED_PASTE), "{mode:?}");
}

#[test]
fn q_quits_within_a_bound_and_restores_the_terminal() {
    let root = temp_root();
    seed_root(root.path());
    let mut session = launch(root.path(), &root.path().join(".cache"));
    session.wait_for_screen("Unseen (3)", "the seeded stream");
    assert!(session.mode().contains(TermMode::ALT_SCREEN));
    session.wait_for("a thumbnail", |s| !s.overlays().is_empty());

    session.write("q");
    let code = session.wait_exit(Duration::from_secs(5));
    assert_eq!(code, Some(0), "q must end the process, not just the UI");
    assert_terminal_restored(&session);
    assert!(session.output().contains("a=d,d=A"));
}

#[test]
fn ctrl_c_quits_within_a_bound_and_restores_the_terminal() {
    let root = temp_root();
    seed_root(root.path());
    let mut session = launch(root.path(), &root.path().join(".cache"));
    session.wait_for_screen("Unseen (3)", "the seeded stream");
    // Even from the composer: Ink's exitOnCtrlC ends the app before any input handler.
    session.write("c");
    session.wait_for_screen("Share feedback", "the composer");

    session.write("\x03");
    let code = session.wait_exit(Duration::from_secs(5));
    assert_eq!(code, Some(0));
    assert_terminal_restored(&session);
}

#[test]
fn q_during_the_first_scan_still_exits() {
    let root = temp_root();
    seed_root(root.path());
    let mut session = launch(root.path(), &root.path().join(".cache"));
    // The header is the first thing drawn; the scan and the watcher are still starting.
    session.wait_for_screen("Astroshots", "the first frame");
    session.write("q");
    assert_eq!(session.wait_exit(Duration::from_secs(5)), Some(0));
    assert_terminal_restored(&session);
}

#[test]
fn sigterm_restores_the_terminal_and_exits_143() {
    let root = temp_root();
    seed_root(root.path());
    let mut session = launch(root.path(), &root.path().join(".cache"));
    session.wait_for_screen("Unseen (3)", "the seeded stream");
    let pid = session.child.process_id().expect("child pid");
    let killed = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    assert_eq!(session.wait_exit(Duration::from_secs(5)), Some(143));
    assert_terminal_restored(&session);
}

#[test]
fn a_resized_terminal_is_redrawn_at_the_new_size() {
    let root = temp_root();
    seed_root(root.path());
    let mut session = launch(root.path(), &root.path().join(".cache"));
    session.wait_for_screen("⏎ review", "the split layout");
    session
        ._master
        .resize(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    // The emulator keeps its 140 columns; the tray now draws for 100.
    session.wait_for_screen("⏎ detail", "the single-column layout");
    assert_eq!(session.close(), Some(0));
}

// ---- capture.e2e.test.ts: "astroshot pty captures the review tray with pictures" ----

#[test]
fn renders_thumbnails_into_the_png() {
    let temp = tempfile::Builder::new()
        .prefix("astroshot-review-capture-")
        .tempdir()
        .unwrap();
    let root = temp.path().join("root");
    let feature = root.join("demo-app/.astroshot/welcome");
    fs::create_dir_all(&feature).unwrap();
    fs::copy(
        demo_fixture("welcome.png"),
        feature.join("0001-welcome.png"),
    )
    .unwrap();
    let quote = |path: &Path| serde_json::to_string(&path.to_string_lossy()).unwrap();
    let fixture = temp.path().join("review.yaml");
    fs::write(
        &fixture,
        [
            "version: 1".to_string(),
            format!("command: {}", quote(Path::new(BIN))),
            format!("args: [review, --root, {}, --no-index]", quote(&root)),
            "cols: 120".to_string(),
            "rows: 30".to_string(),
            "scale: 1".to_string(),
            "graphics: kitty".to_string(),
            "timeoutMs: 40000".to_string(),
            "settleMs: 1500".to_string(),
            "env:".to_string(),
            format!(
                "  ASTROSHOT_REVIEW_CACHE_DIR: {}",
                quote(&temp.path().join("cache"))
            ),
            "  ASTROSHOT_REVIEW_GRAPHICS: kitty".to_string(),
            "actions:".to_string(),
            "  - waitFor: Unseen (1)".to_string(),
            "  - pauseMs: 2000".to_string(),
            "expectText: [Astroshots, welcome · Welcome]".to_string(),
        ]
        .join("\n"),
    )
    .unwrap();
    let out_path = temp.path().join("review.png");
    let text_only = temp.path().join("text-only.png");

    let capture = |fixture: &Path, out: &Path| {
        let output = std::process::Command::new(BIN)
            .args([
                "pty",
                &fixture.to_string_lossy(),
                "-o",
                &out.to_string_lossy(),
            ])
            .current_dir(temp.path())
            .env_remove("TMUX")
            .env_remove("HERDR_ENV")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("MOSH_CONNECTION")
            .env_remove("MOSH_SERVER_NETWORK_TMOUT")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "astroshot pty failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    capture(&fixture, &out_path);
    assert!(out_path.exists());
    // A frame with a real thumbnail and preview is far larger than text alone.
    let size = fs::metadata(&out_path).unwrap().len();
    assert!(size > 40_000, "PNG is only {size} bytes");

    // The same tray with pictures off is the "text alone" frame.
    let plain = fs::read_to_string(&fixture)
        .unwrap()
        .replace("args: [review,", "args: [review, --no-graphics,");
    let plain_fixture = temp.path().join("review-text.yaml");
    fs::write(&plain_fixture, plain).unwrap();
    capture(&plain_fixture, &text_only);
    let text_size = fs::metadata(&text_only).unwrap().len();
    assert!(
        size > text_size + 20_000,
        "pictures added only {} bytes over the text frame",
        size.saturating_sub(text_size)
    );
}

/// Help text `astroshot-review --help` printed, captured from the TS bin.
const REVIEW_HELP: &str = include_str!("fixtures/help/astroshot-review.txt");

/// `packages/astroshot-review/bin/astroshot-review.mjs` called `main` with the
/// raw arguments: no default roots, no `help` word, and every argument parsed
/// before `--help` is honoured. The binary does the same when it is named
/// `astroshot-review`. Run without a TTY, as the npm bin was for these cases.
#[test]
fn astroshot_review_bin_help_version_and_usage_errors_match_the_ts_bin() {
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("astroshot-review");
    std::os::unix::fs::symlink(BIN, &alias).unwrap();
    let call = |args: &[&str]| {
        std::process::Command::new(&alias)
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    #[track_caller]
    fn assert_exact(output: &std::process::Output, stdout: &str, stderr: &str, code: i32) {
        assert_eq!(String::from_utf8_lossy(&output.stdout), stdout, "stdout");
        assert_eq!(String::from_utf8_lossy(&output.stderr), stderr, "stderr");
        assert_eq!(output.status.code(), Some(code), "exit code");
    }
    let needs_tty =
        "astroshot review needs an interactive terminal (stdin and stdout must be a TTY).\n";
    let package_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../packages/astroshot-review/package.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let version = format!("{}\n", package_json["version"].as_str().unwrap());

    for args in [&["--help"][..], &["-h"], &["x", "--help"], &["-v", "-h"]] {
        assert_exact(&call(args), REVIEW_HELP, "", 0);
    }
    for args in [&["--version"][..], &["-v"], &["--root", "x", "--version"]] {
        assert_exact(&call(args), &version, "", 0);
    }
    // No arguments, `help` and any other word are folders to watch.
    for args in [&[][..], &["help"], &["bogus"], &["--no-graphics"]] {
        assert_exact(&call(args), "", needs_tty, 1);
    }
    // A parse error wins over --help wherever the flag sits.
    for (args, message) in [
        (&["--bogus"][..], "Unknown option: --bogus"),
        (&["-"], "Unknown option: -"),
        (&["--root"], "--root requires a directory"),
        (
            &["--roots-source"],
            "--roots-source must be app, cli, or cwd",
        ),
        (
            &["--roots-source", "bogus", "--help"],
            "--roots-source must be app, cli, or cwd",
        ),
        (
            &["--help", "--roots-source", "bogus"],
            "--roots-source must be app, cli, or cwd",
        ),
    ] {
        assert_exact(&call(args), "", &format!("{message}\n\n{REVIEW_HELP}"), 1);
    }
}
