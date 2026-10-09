//! `app.tsx` has no unit test. These cover it three ways:
//!
//! 1. **Frames from the real Ink `<App>`**: `tests/fixtures/review_app_frames.json`
//!    holds scenarios captured by driving `ink.render(<App/>, { debug: true })`
//!    with a fake stdout/stdin over seeded `.astroshot` trees (a real
//!    `ReviewStore` with `watch` and `useIndex` off, graphics `none` so each
//!    picture box shows its label, `Date.now` fixed at 2026-08-11T16:30Z). Each
//!    step records the keys written to stdin and the rows Ink drew once output
//!    settled. A scenario is replayed here against the same tree and must draw
//!    the same rows after every step. Local clock strings in the capture are
//!    recomputed for the machine's timezone; the comment timestamp and the
//!    index path are masked.
//! 2. **One test per key binding** asserting the state transition.
//! 3. **Store round trips** asserting `review.json` on disk.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Deserialize;
use tokio::sync::Notify;

use super::*;
use crate::data::store::StoreOptions;
use crate::ui::context::test_support::{FakeService, services};
use crate::ui::testing::rows;
use crate::ui::theme::{DateInput, abbreviated_date_time, clock_time};
use astroshot_engine::review_data::friction::run_display_title;
use astroshot_engine::review_data::model::{Chapter, FrictionStep, ReviewSnapshot};

// ---- Harness ------------------------------------------------------------------

#[derive(Default)]
struct RecordingDesktop {
    calls: Mutex<Vec<(&'static str, String)>>,
    fail: AtomicBool,
}

impl RecordingDesktop {
    fn record(&self, kind: &'static str, target: String) -> ActionFuture<anyhow::Result<()>> {
        self.calls.lock().unwrap().push((kind, target));
        let fail = self.fail.load(Ordering::SeqCst);
        Box::pin(async move {
            if fail {
                anyhow::bail!("no desktop")
            }
            Ok(())
        })
    }

    fn calls(&self) -> Vec<(&'static str, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Desktop for RecordingDesktop {
    fn reveal_in_file_manager(&self, target: String) -> ActionFuture<anyhow::Result<()>> {
        self.record("reveal", target)
    }

    fn open_with_default_app(&self, target: String) -> ActionFuture<anyhow::Result<()>> {
        self.record("open", target)
    }

    fn copy_image_to_clipboard(&self, image_path: String) -> ActionFuture<anyhow::Result<()>> {
        self.record("copy", image_path)
    }
}

/// How the stubbed store writes behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Writes {
    /// Resolve at once.
    Ok,
    /// Reject with "disk full".
    Fail,
    /// Never resolve (the write stays queued).
    Hang,
    /// `markShotSeen`/`markManySeen` hang and `addShotComment` rejects, like the Ink capture.
    CaptureStub,
}

struct StubActions {
    writes: Mutex<Writes>,
    calls: Mutex<Vec<String>>,
    opened: AtomicUsize,
    gate: Arc<Notify>,
}

impl StubActions {
    fn new(writes: Writes) -> Arc<Self> {
        Arc::new(Self {
            writes: Mutex::new(writes),
            calls: Mutex::default(),
            opened: AtomicUsize::new(0),
            gate: Arc::new(Notify::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn respond<T: Send + 'static>(
        &self,
        call: String,
        comment: bool,
        ok: T,
    ) -> ActionFuture<anyhow::Result<T>> {
        self.calls.lock().unwrap().push(call);
        let writes = match *self.writes.lock().unwrap() {
            Writes::CaptureStub if comment => Writes::Fail,
            Writes::CaptureStub => Writes::Hang,
            other => other,
        };
        let gate = self.gate.clone();
        Box::pin(async move {
            match writes {
                Writes::Fail => anyhow::bail!("disk full"),
                Writes::Hang => {
                    gate.notified().await;
                    Ok(ok)
                }
                _ => Ok(ok),
            }
        })
    }
}

impl ReviewActions for StubActions {
    fn rescan(&self, options: RescanOptions) -> ActionFuture<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("rescan force={}", options.force));
        Box::pin(async {})
    }

    fn mark_opened(&self) {
        self.opened.fetch_add(1, Ordering::SeqCst);
    }

    fn mark_shot_seen(
        &self,
        shot: Shot,
        comment: Option<String>,
    ) -> ActionFuture<anyhow::Result<Shot>> {
        self.respond(format!("seen {} {comment:?}", shot.path), false, shot)
    }

    fn add_shot_comment(&self, shot: Shot, body: String) -> ActionFuture<anyhow::Result<Shot>> {
        self.respond(format!("comment {} {body}", shot.path), true, shot)
    }

    fn mark_many_seen(&self, shots: Vec<Shot>) -> ActionFuture<MarkManyResult> {
        let paths: Vec<&str> = shots.iter().map(|shot| shot.path.as_str()).collect();
        let failed = *self.writes.lock().unwrap() == Writes::Fail;
        let result = MarkManyResult {
            ok: if failed { 0 } else { shots.len() },
            failed: if failed { shots.len() } else { 0 },
        };
        let work = self.respond(format!("many {}", paths.join(",")), false, ());
        Box::pin(async move {
            let _ = work.await;
            result
        })
    }

    fn mark_friction_run_seen(
        &self,
        log: FrictionLog,
        run: FrictionRun,
    ) -> ActionFuture<anyhow::Result<()>> {
        self.respond(format!("log {} {}", log.id, run.run_id), false, ())
    }
}

const FIXED_NOW: f64 = 1_786_465_800_000.0; // 2026-08-11T16:30:00Z

struct Harness {
    app: App,
    notify: Arc<Notify>,
    services: AppServices,
    desktop: Arc<RecordingDesktop>,
    actions: Option<Arc<StubActions>>,
    quits: Arc<AtomicUsize>,
    cols: u16,
    rows: u16,
    _root: Option<tempfile::TempDir>,
}

struct Setup {
    cols: u16,
    rows: u16,
    graphics: GraphicsProtocol,
    roots: Vec<String>,
    start: bool,
    actions: Option<Arc<StubActions>>,
    root: Option<tempfile::TempDir>,
}

impl Harness {
    async fn open(setup: Setup) -> Self {
        let mut services = services(setup.graphics, FakeService::new(false));
        services.store = ReviewStore::with_options(StoreOptions {
            roots: setup.roots,
            use_index: Some(false),
            watch: Some(false),
            ..StoreOptions::default()
        });
        services.version = "0.2.2".into();
        if setup.start {
            services.store.start().await;
        }
        let notify = Arc::new(Notify::new());
        let wake_target = notify.clone();
        let wake: Wake = Arc::new(move || wake_target.notify_one());
        let desktop = Arc::new(RecordingDesktop::default());
        let quits = Arc::new(AtomicUsize::new(0));
        let quit_count = quits.clone();
        let app = App::with_options(
            &services,
            AppOptions {
                props: AppProps {
                    on_quit: Some(Box::new(move || {
                        quit_count.fetch_add(1, Ordering::SeqCst);
                    })),
                },
                desktop: desktop.clone(),
                actions: setup
                    .actions
                    .clone()
                    .map(|actions| actions as Arc<dyn ReviewActions>),
                now: Some(Arc::new(|| FIXED_NOW)),
                ..AppOptions::new(TerminalSize {
                    columns: setup.cols,
                    rows: setup.rows,
                })
            },
            wake,
        );
        Self {
            app,
            notify,
            services,
            desktop,
            actions: setup.actions,
            quits,
            cols: setup.cols,
            rows: setup.rows,
            _root: setup.root,
        }
    }

    /// An app over store state published directly (no disk), with stubbed writes.
    async fn synthetic(cols: u16, graphics: GraphicsProtocol, writes: Writes) -> Self {
        let harness = Self::open(Setup {
            cols,
            rows: 40,
            graphics,
            roots: vec!["/w".into()],
            start: false,
            actions: Some(StubActions::new(writes)),
            root: None,
        })
        .await;
        harness.services.store.publish_with(|state| {
            state.shots = synthetic_shots();
            state.friction_logs = synthetic_logs();
            state.tree_count = 2;
        });
        harness
    }

    async fn wide() -> Self {
        Self::synthetic(140, GraphicsProtocol::None, Writes::Ok).await
    }

    async fn narrow() -> Self {
        Self::synthetic(100, GraphicsProtocol::None, Writes::Ok).await
    }

    async fn kitty(cols: u16) -> Self {
        Self::synthetic(cols, GraphicsProtocol::Kitty, Writes::Ok).await
    }

    fn stub(&self) -> &StubActions {
        self.actions.as_ref().expect("stubbed actions")
    }

    fn frame(&mut self) -> Vec<String> {
        let area = Rect::new(0, 0, self.cols, self.rows);
        let mut buf = Buffer::empty(area);
        self.app.render_into(area, &mut buf);
        rows(&buf)
    }

    fn screen(&mut self) -> String {
        self.frame().join("\n")
    }

    /// Send every key in `keys` (terminal byte sequences, as the Ink capture wrote them).
    fn keys(&mut self, keys: &str) -> Outcome {
        let mut outcome = Outcome::Continue;
        for event in parse_keys(keys) {
            outcome = self.app.handle(Event::Key(event));
            // Ink renders between keys; rendering is what mounts children.
            self.frame();
        }
        outcome
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.app.handle(Event::Resize(cols, rows));
    }

    /// Re-run the app's effects on every wake until `done` holds (bounded).
    async fn until(&mut self, label: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.app.sync();
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {label}\n{}",
                self.screen()
            );
            let _ = tokio::time::timeout(Duration::from_millis(20), self.notify.notified()).await;
        }
    }

    async fn until_toast(&mut self, message: &str) {
        self.until(&format!("toast {message:?}"), |h| h.toast() == message)
            .await;
    }

    fn toast(&self) -> &str {
        self.app
            .ui
            .toast
            .as_ref()
            .map_or("", |toast| toast.message.as_str())
    }

    fn ui(&self) -> &UiState {
        &self.app.ui
    }
}

fn parse_keys(keys: &str) -> Vec<KeyEvent> {
    let chars: Vec<char> = keys.chars().collect();
    let mut events = Vec::new();
    let mut index = 0;
    let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
    while index < chars.len() {
        let c = chars[index];
        index += 1;
        if c == '\x1b' && chars.get(index) == Some(&'[') {
            let mut end = index + 1;
            while end < chars.len() && !chars[end].is_ascii_alphabetic() && chars[end] != '~' {
                end += 1;
            }
            let sequence: String = chars[index + 1..=end].iter().collect();
            index = end + 1;
            events.push(plain(match sequence.as_str() {
                "A" => KeyCode::Up,
                "B" => KeyCode::Down,
                "C" => KeyCode::Right,
                "D" => KeyCode::Left,
                "H" => KeyCode::Home,
                "F" => KeyCode::End,
                "5~" => KeyCode::PageUp,
                "6~" => KeyCode::PageDown,
                other => panic!("unknown escape sequence {other:?}"),
            }));
            continue;
        }
        events.push(match c {
            '\x1b' => plain(KeyCode::Esc),
            '\r' => plain(KeyCode::Enter),
            '\t' => plain(KeyCode::Tab),
            '\x03' => KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            c if c.is_uppercase() => KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT),
            c => plain(KeyCode::Char(c)),
        });
    }
    events
}

// ---- Synthetic store state ----------------------------------------------------------

fn shot(path: &str, worktree_path: &str, sequence: &str, f: impl FnOnce(&mut Shot)) -> Shot {
    let feature_dir = dirname(path);
    let mut shot = Shot {
        id: path.into(),
        path: path.into(),
        file_name: astroshot_engine::review_data::paths::basename(path).to_string(),
        worktree_path: worktree_path.into(),
        worktree: worktree_path.trim_start_matches('/').into(),
        worktree_short: worktree_path.trim_start_matches('/').into(),
        feature: astroshot_engine::review_data::paths::basename(&feature_dir).to_string(),
        feature_dir,
        sequence: Some(sequence.into()),
        slug: "slug".into(),
        title: format!("Shot {sequence}"),
        description: String::new(),
        url: None,
        run_id: Some("r".into()),
        status: None,
        captured_at: 1_788_617_400_000.0,
        mtime_ms: 0.0,
        is_movie: false,
        video_file_name: None,
        video_path: None,
        duration_ms: None,
        source: None,
        chapters: vec![],
        review: None,
    };
    f(&mut shot);
    shot
}

fn seen_review() -> ReviewSnapshot {
    ReviewSnapshot {
        state: ReviewState::Seen,
        decision: Some("seen".into()),
        hash_matches: true,
        is_stale: false,
        comments: vec![],
        reviewed_at: None,
    }
}

const MOVIE: &str = "/w1/.astroshot/f/0003-c.png";
const MOVIE_VIDEO: &str = "/w1/.astroshot/f/0003-c.webm";
const OTHER: &str = "/w2/.astroshot/g/0001-a.png";
const SEEN: &str = "/w1/.astroshot/f/0002-b.png";
const FIRST: &str = "/w1/.astroshot/f/0001-a.png";

/// Newest first, as the store orders them. With the Unseen filter the stream
/// is: header w1, MOVIE, header w2, OTHER, header w1, FIRST.
fn synthetic_shots() -> Vec<Shot> {
    vec![
        shot(MOVIE, "/w1", "0003", |s| {
            s.is_movie = true;
            s.video_path = Some(MOVIE_VIDEO.into());
            s.video_file_name = Some("0003-c.webm".into());
            s.duration_ms = Some(10_000.0);
            s.chapters = vec![
                Chapter {
                    slug: Some("late".into()),
                    title: None,
                    t_ms: Some(6000.0),
                },
                Chapter {
                    slug: Some("early".into()),
                    title: None,
                    t_ms: Some(2000.0),
                },
                Chapter::default(),
            ];
        }),
        shot(OTHER, "/w2", "0001", |_| {}),
        shot(SEEN, "/w1", "0002", |s| s.review = Some(seen_review())),
        shot(FIRST, "/w1", "0001", |_| {}),
    ]
}

fn step(number: f64, screenshots: &[&str]) -> FrictionStep {
    FrictionStep {
        id: format!("step-{number}"),
        step: number,
        step_id: format!("s{number}"),
        title: format!("Step {number}"),
        description: String::new(),
        transcript: "Transcript".into(),
        screenshots: screenshots.iter().map(|s| (*s).to_string()).collect(),
        good: vec![],
        improve: vec![],
        url: None,
        captured_at: None,
    }
}

fn run(run_id: &str, steps: Vec<FrictionStep>) -> FrictionRun {
    FrictionRun {
        run_id: run_id.into(),
        directory: format!("/w1/.astroshot/friction-logs/onboarding/runs/{run_id}"),
        log_path: Some(format!(
            "/w1/.astroshot/friction-logs/onboarding/runs/{run_id}/log.jsonl"
        )),
        captured_at: FIXED_NOW - 3_600_000.0,
        status: None,
        steps,
        review: None,
    }
}

fn log(id: &str, runs: Vec<FrictionRun>) -> FrictionLog {
    FrictionLog {
        id: id.into(),
        slug: id.into(),
        directory: format!("/w1/.astroshot/friction-logs/{id}"),
        worktree_path: "/w1".into(),
        worktree: "w1".into(),
        worktree_short: "w1".into(),
        title: format!("Log {id}"),
        description: String::new(),
        status: None,
        updated_at: FIXED_NOW,
        prompt_path: Some(format!("/w1/.astroshot/friction-logs/{id}/prompt.md")),
        runs,
    }
}

const STEP_SHOT_A: &str = "/w1/.astroshot/friction-logs/onboarding/runs/r2/0001-a.png";
const STEP_SHOT_B: &str = "/w1/.astroshot/friction-logs/onboarding/runs/r2/0001-b.png";

/// `onboarding` has two runs (latest first) of two steps; `empty` has a run without steps.
fn synthetic_logs() -> Vec<FrictionLog> {
    vec![
        log(
            "onboarding",
            vec![
                run(
                    "20260812T153000Z",
                    vec![step(1.0, &[STEP_SHOT_A, STEP_SHOT_B]), step(2.0, &[])],
                ),
                run("20260811T153000Z", vec![step(1.0, &[])]),
            ],
        ),
        log("empty", vec![run("20260810T153000Z", vec![])]),
        log("seen", {
            let mut seen = run("20260809T153000Z", vec![step(1.0, &[])]);
            seen.review = Some(seen_review());
            vec![seen]
        }),
    ]
}

// ---- Frames captured from the Ink <App> --------------------------------------------

#[derive(Deserialize)]
struct Substitution {
    text: String,
    ms: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunTitle {
    text: String,
    run_id: String,
}

#[derive(Deserialize)]
struct SeedOptions {
    friction: bool,
    second: bool,
}

#[derive(Deserialize)]
struct Step {
    name: String,
    keys: Option<String>,
    resize: Option<(u16, u16)>,
    frame: Vec<String>,
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    cols: u16,
    rows: u16,
    graphics: String,
    root: Option<String>,
    seed: Option<SeedOptions>,
    state: Option<serde_json::Value>,
    actions: Option<String>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Truth {
    file_time: f64,
    clocks: Vec<Substitution>,
    abbrs: Vec<Substitution>,
    run_title: RunTitle,
    scenarios: Vec<Scenario>,
}

fn truth() -> Truth {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/review_app_frames.json"
    )))
    .expect("review_app_frames.json parses")
}

fn demo_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/astroshot/fixtures/demo")
        .join(name)
}

const CHECKOUT_MANIFEST: &str = r#"{"version":1,"feature":"checkout","run_id":"checkout-e2e","status":"pass","shots":[{"id":"0001","file":"0001-welcome.png","slug":"welcome","title":"Welcome","description":"Landing state.","captured_at":"2026-09-05T14:10:00Z"},{"id":"0002","file":"0002-next-steps.png","slug":"next-steps","title":"Next steps","description":"Confirmation.","captured_at":"2026-09-05T14:12:00Z"},{"id":"0003","kind":"movie","file":"0003-journey.png","video":"0003-journey.webm","slug":"journey","title":"Journey","duration_ms":4240,"source":"frames","captured_at":"2026-09-05T14:20:00Z","chapters":[{"slug":"poster","t_ms":2685}]}]}"#;
const LOGIN_MANIFEST: &str = r#"{"version":1,"feature":"login","run_id":"login-1","shots":[{"id":"0001","file":"0001-form.png","slug":"form","title":"Form","captured_at":"2026-09-05T14:15:00Z"}]}"#;
const FRICTION_RUN: &str = "20260811T153000Z";
const FRICTION_LOG_LINE: &str = r#"{"step":1,"id":"land","title":"Land on home","transcript":"I land on the home page.","screenshots":["0001-land.png"],"good":["Fast"],"improve":["Copy is vague"]}"#;

fn touch(dir: &Path, time: std::time::SystemTime) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            touch(&path, time);
        }
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }
}

/// The tree the Ink capture seeded; file times are pinned as they were there.
fn seed(root: &Path, options: &SeedOptions, file_time_ms: f64) {
    let feature = root.join("demo-app/.astroshot/checkout");
    std::fs::create_dir_all(&feature).unwrap();
    for (from, to) in [
        ("welcome.png", "0001-welcome.png"),
        ("next-steps.png", "0002-next-steps.png"),
        ("journey.png", "0003-journey.png"),
        ("journey.webm", "0003-journey.webm"),
    ] {
        std::fs::copy(demo_fixture(from), feature.join(to)).unwrap();
    }
    std::fs::write(feature.join("manifest.json"), CHECKOUT_MANIFEST).unwrap();
    if options.second {
        let other = root.join("other-app/.astroshot/login");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::copy(demo_fixture("welcome.png"), other.join("0001-form.png")).unwrap();
        std::fs::write(other.join("manifest.json"), LOGIN_MANIFEST).unwrap();
    }
    if options.friction {
        let log = root.join("demo-app/.astroshot/friction-logs/onboarding");
        let run = log.join("runs").join(FRICTION_RUN);
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(log.join("prompt.md"), "# Onboarding\n").unwrap();
        std::fs::copy(demo_fixture("welcome.png"), run.join("0001-land.png")).unwrap();
        std::fs::write(run.join("log.jsonl"), format!("{FRICTION_LOG_LINE}\n")).unwrap();
    }
    touch(
        root,
        UNIX_EPOCH + Duration::from_millis(file_time_ms as u64),
    );
}

/// A root whose path is as long as the capture's (`/tmp/astroshot-app-TRUTH001`),
/// so rows that show the path lay out the same.
fn capture_sized_root() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("astroshot-app-")
        .rand_bytes(8)
        .tempdir_in("/tmp")
        .unwrap()
}

/// Rows whose content depends on when or where the test runs.
fn mask(row: &str) -> String {
    // A comment's timestamp is the moment it was written.
    if let Some(index) = row.find("Reviewer · ") {
        return format!("{}Reviewer · <when>", &row[..index]);
    }
    // The index path depends on the cache directory.
    if row.starts_with(" Index ") {
        return " Index <path>".to_string();
    }
    row.to_string()
}

async fn replay(name: &str) {
    let truth = truth();
    let scenario = truth
        .scenarios
        .iter()
        .find(|scenario| scenario.name == name)
        .unwrap_or_else(|| panic!("no captured scenario {name}"));
    let root = scenario.root.as_ref().map(|_| capture_sized_root());
    if let (Some(root), Some(options)) = (&root, &scenario.seed) {
        seed(root.path(), options, truth.file_time);
    }
    let root_path = root
        .as_ref()
        .map(|root| root.path().to_string_lossy().into_owned());
    let mut harness = Harness::open(Setup {
        cols: scenario.cols,
        rows: scenario.rows,
        graphics: match scenario.graphics.as_str() {
            "kitty" => GraphicsProtocol::Kitty,
            _ => GraphicsProtocol::None,
        },
        roots: root_path.iter().cloned().collect(),
        start: scenario.state.is_none(),
        actions: scenario
            .actions
            .as_ref()
            .map(|_| StubActions::new(Writes::CaptureStub)),
        root,
    })
    .await;
    if let Some(state) = &scenario.state {
        harness.services.store.publish_with(|store| {
            store.scanning = state["scanning"].as_bool().unwrap_or(false);
            store.phase = match state["phase"].as_str() {
                Some("full") => ScanPhase::Full,
                _ => ScanPhase::Shallow,
            };
            store.watching = state["watching"].as_bool().unwrap_or(false);
            store.unread_count = state["unreadCount"].as_u64().unwrap_or(0) as usize;
            store.error = state["error"].as_str().map(str::to_string);
        });
    }

    let expected_rows = |step: &Step| -> Vec<String> {
        step.frame
            .iter()
            .map(|row| {
                let mut row = row.clone();
                if let (Some(captured), Some(actual)) = (&scenario.root, &root_path) {
                    row = row.replace(captured, actual);
                }
                for clock in &truth.clocks {
                    row = row.replace(&clock.text, &clock_time(clock.ms));
                }
                for abbr in &truth.abbrs {
                    row = row.replace(
                        &abbr.text,
                        &abbreviated_date_time(DateInput::Millis(abbr.ms)),
                    );
                }
                row = row.replace(
                    &truth.run_title.text,
                    &run_display_title(&truth.run_title.run_id),
                );
                mask(&row)
            })
            .collect()
    };

    for step in &scenario.steps {
        if let Some(keys) = &step.keys {
            harness.keys(keys);
        }
        if let Some((cols, rows)) = step.resize {
            harness.resize(cols, rows);
        }
        let expected = expected_rows(step);
        // Writes, toasts and scans finish asynchronously: wait for the frame, bounded.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let actual: Vec<String> = harness.frame().iter().map(|row| mask(row)).collect();
            if actual == expected {
                break;
            }
            if Instant::now() >= deadline {
                let diff: Vec<String> = expected
                    .iter()
                    .zip(&actual)
                    .enumerate()
                    .filter(|(_, (want, got))| want != got)
                    .map(|(row, (want, got))| {
                        format!("row {row}\n  ink : {want:?}\n  rust: {got:?}")
                    })
                    .collect();
                panic!(
                    "{name} / {} (keys {:?}) does not match the Ink frame:\n{}\n--- rust frame ---\n{}",
                    step.name,
                    step.keys,
                    diff.join("\n"),
                    actual.join("\n")
                );
            }
            let _ =
                tokio::time::timeout(Duration::from_millis(20), harness.notify.notified()).await;
        }
    }
}

macro_rules! ink_scenario {
    ($($test:ident => $scenario:literal,)*) => {
        $(
            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn $test() {
                replay($scenario).await;
            }
        )*
    };
}

ink_scenario! {
    ink_frames_wide_stream_groups_filters_help_and_settings => "wide-stream",
    ink_frames_wide_composer_takeover_and_seen => "wide-review",
    ink_frames_wide_friction_list_log_step_and_takeover => "wide-friction",
    ink_frames_narrow_detail_page_friction_and_resize => "narrow",
    ink_frames_short_terminal => "short",
    ink_frames_smallest_terminal_that_fits => "tiny",
    ink_frames_empty_root => "empty",
    ink_frames_empty_root_narrow => "empty-narrow",
    ink_frames_no_roots => "no-roots",
    ink_frames_scanning_shallow => "scanning-shallow",
    ink_frames_scanning_deep_with_unread_and_error => "scanning-full",
    ink_frames_busy_and_error_banners_wide => "busy-wide",
    ink_frames_busy_and_error_banners_narrow => "busy-narrow",
    ink_frames_kitty_inline_player => "kitty",
}

#[test]
fn every_captured_scenario_is_replayed() {
    let replayed = [
        "wide-stream",
        "wide-review",
        "wide-friction",
        "narrow",
        "short",
        "tiny",
        "empty",
        "empty-narrow",
        "no-roots",
        "scanning-shallow",
        "scanning-full",
        "busy-wide",
        "busy-narrow",
        "kitty",
    ];
    let captured: Vec<String> = truth().scenarios.into_iter().map(|s| s.name).collect();
    assert_eq!(captured, replayed);
}

// ---- Key bindings -----------------------------------------------------------------

impl Harness {
    fn cursor(&self) -> Option<String> {
        self.app
            .derive()
            .cursor_shot()
            .map(|shot| shot.path.clone())
    }

    fn active(&self) -> Option<String> {
        self.app.derive().active_shot.map(|shot| shot.path)
    }

    fn selected(&self) -> Option<&str> {
        self.ui().selected_shot.as_deref()
    }

    fn pb(&self) -> PlaybackState {
        self.app.playback()
    }

    fn calls(&self) -> Vec<String> {
        self.stub().calls()
    }

    fn failing_desktop(self) -> Self {
        self.desktop.fail.store(true, Ordering::SeqCst);
        self
    }
}

/// One binding: open the tray, run `setup` keys, press `keys`, then assert.
macro_rules! key_test {
    ($name:ident, $open:expr, $setup:expr, $keys:expr, |$h:ident| $body:block) => {
        #[tokio::test]
        async fn $name() {
            #[allow(unused_mut)]
            let mut $h = $open.await;
            $h.keys($setup);
            $h.keys($keys);
            $body
        }
    };
}

const UP: &str = "\x1b[A";
const DOWN: &str = "\x1b[B";
const RIGHT: &str = "\x1b[C";
const LEFT: &str = "\x1b[D";
const HOME: &str = "\x1b[H";
const END: &str = "\x1b[F";
const PAGE_UP: &str = "\x1b[5~";
const PAGE_DOWN: &str = "\x1b[6~";
const ESC: &str = "\x1b";
const ENTER: &str = "\r";
const CTRL_C: &str = "\x03";

fn some(path: &str) -> Option<String> {
    Some(path.to_string())
}

// Stream list.
key_test!(
    stream_starts_on_the_first_shot_below_its_header,
    Harness::wide(),
    "",
    "",
    |h| {
        assert_eq!(h.cursor(), some(MOVIE));
        assert_eq!(h.active(), some(MOVIE));
        assert_eq!(*h.ui(), UiState::default());
    }
);
key_test!(
    stream_j_moves_to_the_next_shot_skipping_headers,
    Harness::wide(),
    "",
    "j",
    |h| {
        assert_eq!((h.ui().cursor, h.cursor()), (3, some(OTHER)));
    }
);
key_test!(
    stream_down_arrow_moves_down,
    Harness::wide(),
    "",
    DOWN,
    |h| {
        assert_eq!(h.cursor(), some(OTHER));
    }
);
key_test!(stream_k_moves_up, Harness::wide(), "jj", "k", |h| {
    assert_eq!(h.cursor(), some(OTHER));
});
key_test!(stream_up_arrow_moves_up, Harness::wide(), "j", UP, |h| {
    assert_eq!(h.cursor(), some(MOVIE));
});
key_test!(
    stream_page_down_jumps_a_quarter_of_the_list,
    Harness::wide(),
    "",
    PAGE_DOWN,
    |h| {
        // 36 list rows / 4 = 9 items, clamped to the last one.
        assert_eq!((h.ui().cursor, h.cursor()), (5, some(FIRST)));
    }
);
key_test!(
    stream_page_up_jumps_back,
    Harness::wide(),
    "G",
    PAGE_UP,
    |h| {
        assert_eq!(h.cursor(), some(MOVIE));
    }
);
key_test!(
    stream_capital_g_goes_to_the_end,
    Harness::wide(),
    "",
    "G",
    |h| {
        assert_eq!(h.cursor(), some(FIRST));
    }
);
key_test!(stream_end_goes_to_the_end, Harness::wide(), "", END, |h| {
    assert_eq!(h.cursor(), some(FIRST));
});
key_test!(stream_g_goes_to_the_top, Harness::wide(), "G", "g", |h| {
    assert_eq!((h.ui().cursor, h.cursor()), (1, some(MOVIE)));
});
key_test!(
    stream_home_goes_to_the_top,
    Harness::wide(),
    "G",
    HOME,
    |h| {
        assert_eq!(h.cursor(), some(MOVIE));
    }
);
key_test!(
    stream_u_switches_to_history_and_resets_the_cursor,
    Harness::wide(),
    "j",
    "u",
    |h| {
        assert_eq!(h.ui().stream_filter, StreamFilter::History);
        assert_eq!((h.ui().cursor, h.cursor()), (0, some(SEEN)));
        h.keys("u");
        assert_eq!(h.ui().stream_filter, StreamFilter::Unseen);
    }
);
key_test!(
    stream_m_toggles_movies_only_and_resets_the_cursor,
    Harness::wide(),
    "j",
    "m",
    |h| {
        assert!(h.ui().movies_only);
        assert_eq!((h.ui().cursor, h.cursor()), (0, some(MOVIE)));
        h.keys("m");
        assert!(!h.ui().movies_only);
    }
);
key_test!(
    stream_z_collapses_the_group_under_the_cursor,
    Harness::wide(),
    "",
    "z",
    |h| {
        assert_eq!(h.ui().collapsed, HashSet::from([MOVIE.to_string()]));
        // The stored cursor (0) now names the collapsed header, which is a stop.
        assert_eq!(h.cursor(), None);
        h.keys("z");
        assert!(h.ui().collapsed.is_empty());
        assert_eq!(h.cursor(), some(MOVIE));
    }
);
key_test!(
    stream_z_after_moving_leaves_the_cursor_index_on_the_next_group,
    Harness::wide(),
    "jk",
    "z",
    |h| {
        // The cursor index (1) is now the next group's expanded header, so the
        // cursor falls through to that group's shot, as in the Ink tray.
        assert_eq!(h.cursor(), some(OTHER));
        h.keys("z");
        assert_eq!(
            h.ui().collapsed,
            HashSet::from([MOVIE.to_string(), OTHER.to_string()])
        );
    }
);
key_test!(
    stream_enter_expands_a_collapsed_header,
    Harness::wide(),
    "zk",
    ENTER,
    |h| {
        assert!(h.ui().collapsed.is_empty());
        assert_eq!(h.ui().takeover, None);
    }
);
key_test!(
    stream_left_expands_a_collapsed_header,
    Harness::wide(),
    "zk",
    LEFT,
    |h| {
        assert!(h.ui().collapsed.is_empty());
    }
);
key_test!(
    stream_right_expands_a_collapsed_header,
    Harness::wide(),
    "zk",
    RIGHT,
    |h| {
        assert!(h.ui().collapsed.is_empty());
    }
);
key_test!(
    stream_space_expands_a_collapsed_header,
    Harness::wide(),
    "zk",
    " ",
    |h| {
        assert!(h.ui().collapsed.is_empty());
    }
);
key_test!(
    stream_z_expands_a_collapsed_header,
    Harness::wide(),
    "zk",
    "z",
    |h| {
        assert!(h.ui().collapsed.is_empty());
    }
);
key_test!(
    stream_capital_s_on_a_header_marks_that_group_seen,
    Harness::wide(),
    "zk",
    "S",
    |h| {
        assert!(h.ui().bulk_busy);
        h.until_toast("Marked 1 frame seen").await;
        assert_eq!(h.calls(), [format!("many {MOVIE}")]);
        assert!(!h.ui().bulk_busy);
    }
);
key_test!(
    stream_other_keys_on_a_header_do_nothing,
    Harness::wide(),
    "zk",
    "fcsp",
    |h| {
        assert_eq!(
            *h.ui(),
            UiState {
                collapsed: HashSet::from([MOVIE.to_string()]),
                ..UiState::default()
            }
        );
        assert!(h.calls().is_empty());
    }
);
key_test!(
    stream_capital_s_marks_every_listed_shot_seen,
    Harness::wide(),
    "",
    "S",
    |h| {
        h.until_toast("Marked 3 frames seen").await;
        assert_eq!(h.calls(), [format!("many {MOVIE},{OTHER},{FIRST}")]);
    }
);
key_test!(
    stream_capital_s_with_nothing_unseen_does_nothing,
    Harness::wide(),
    "u",
    "S",
    |h| {
        assert!(!h.ui().bulk_busy);
        assert!(h.calls().is_empty());
    }
);
key_test!(
    stream_capital_a_marks_the_cursor_worktree_seen,
    Harness::wide(),
    "j",
    "A",
    |h| {
        h.until_toast("Marked 1 frame seen").await;
        assert_eq!(h.calls(), [format!("many {OTHER}")]);
    }
);
key_test!(
    stream_enter_opens_the_takeover_when_split,
    Harness::wide(),
    "",
    ENTER,
    |h| {
        assert_eq!(h.selected(), Some(MOVIE));
        assert_eq!(
            (h.ui().takeover, h.ui().pane),
            (Some(Takeover::Shot), Pane::Stream)
        );
    }
);
key_test!(
    stream_enter_opens_the_detail_page_when_narrow,
    Harness::narrow(),
    "",
    ENTER,
    |h| {
        assert_eq!(h.selected(), Some(MOVIE));
        assert_eq!((h.ui().takeover, h.ui().pane), (None, Pane::Detail));
    }
);
key_test!(
    stream_f_opens_the_takeover,
    Harness::narrow(),
    "j",
    "f",
    |h| {
        assert_eq!(h.selected(), Some(OTHER));
        assert_eq!(
            (h.ui().takeover, h.ui().pane),
            (Some(Takeover::Shot), Pane::Stream)
        );
    }
);
key_test!(
    stream_space_opens_the_takeover,
    Harness::wide(),
    "",
    " ",
    |h| {
        assert_eq!(
            (h.selected(), h.ui().takeover),
            (Some(MOVIE), Some(Takeover::Shot))
        );
    }
);
key_test!(
    stream_s_marks_the_cursor_shot_seen,
    Harness::wide(),
    "",
    "s",
    |h| {
        assert!(h.ui().busy);
        h.until_toast("Seen").await;
        assert_eq!(h.calls(), [format!("seen {MOVIE} None")]);
        assert!(!h.ui().busy);
        assert_eq!(h.ui().error, None);
    }
);
key_test!(
    stream_s_on_a_seen_shot_only_toasts,
    Harness::wide(),
    "u",
    "s",
    |h| {
        assert_eq!(h.toast(), "Already seen");
        assert!(!h.ui().busy);
        assert!(h.calls().is_empty());
    }
);
key_test!(
    stream_c_opens_the_composer_in_the_split_pane,
    Harness::wide(),
    "",
    "c",
    |h| {
        assert!(h.ui().composer);
        assert_eq!((h.selected(), h.ui().pane), (Some(MOVIE), Pane::Stream));
    }
);
key_test!(
    stream_c_opens_the_detail_page_composer_when_narrow,
    Harness::narrow(),
    "",
    "c",
    |h| {
        assert!(h.ui().composer);
        assert_eq!((h.selected(), h.ui().pane), (Some(MOVIE), Pane::Detail));
    }
);
key_test!(
    stream_p_on_a_still_says_not_a_movie,
    Harness::wide(),
    "j",
    "p",
    |h| {
        assert_eq!(h.toast(), "Not a movie");
        assert_eq!(h.selected(), None);
    }
);
key_test!(
    stream_p_without_kitty_selects_and_explains,
    Harness::narrow(),
    "",
    "p",
    |h| {
        assert_eq!(h.toast(), KITTY_PLAYBACK_HINT);
        assert_eq!((h.selected(), h.ui().pane), (Some(MOVIE), Pane::Detail));
        assert!(!h.ui().inline_player);
    }
);
key_test!(
    stream_p_with_kitty_plays_in_the_split_pane,
    Harness::kitty(140),
    "",
    "p",
    |h| {
        assert_eq!((h.selected(), h.ui().pane), (Some(MOVIE), Pane::Stream));
        assert!(h.ui().inline_player);
        assert_eq!(
            h.pb(),
            PlaybackState {
                playing: true,
                ..PlaybackState::default()
            }
        );
    }
);
key_test!(
    stream_p_with_kitty_plays_on_the_detail_page_when_narrow,
    Harness::kitty(100),
    "",
    "p",
    |h| {
        assert_eq!((h.selected(), h.ui().pane), (Some(MOVIE), Pane::Detail));
        assert!(h.ui().inline_player && h.pb().playing);
    }
);
key_test!(stream_o_reveals_the_shot, Harness::wide(), "", "o", |h| {
    assert_eq!(h.desktop.calls(), [("reveal", MOVIE.to_string())]);
    assert_eq!(h.toast(), "");
});
key_test!(
    stream_capital_o_opens_the_movie,
    Harness::wide(),
    "",
    "O",
    |h| {
        assert_eq!(h.desktop.calls(), [("open", MOVIE_VIDEO.to_string())]);
    }
);
key_test!(
    stream_capital_o_on_a_still_does_nothing,
    Harness::wide(),
    "j",
    "O",
    |h| {
        assert!(h.desktop.calls().is_empty());
    }
);
key_test!(stream_y_copies_the_image, Harness::wide(), "", "y", |h| {
    h.until_toast("Copied image").await;
    assert_eq!(h.desktop.calls(), [("copy", MOVIE.to_string())]);
});
key_test!(
    stream_desktop_failures_toast,
    async { Harness::wide().await.failing_desktop() },
    "",
    "",
    |h| {
        h.keys("o");
        h.until_toast("Couldn’t reveal file").await;
        h.keys("O");
        h.until_toast("Couldn’t open movie").await;
        h.keys("y");
        h.until_toast("Couldn’t copy image").await;
    }
);
key_test!(
    stream_keys_clear_the_unread_badge,
    Harness::wide(),
    "",
    "j",
    |h| {
        assert_eq!(h.stub().opened.load(Ordering::SeqCst), 1);
        h.keys("2j");
        assert_eq!(h.stub().opened.load(Ordering::SeqCst), 1);
    }
);

#[tokio::test]
async fn stream_p_on_a_movie_without_its_video_says_so() {
    let mut h = Harness::wide().await;
    h.services.store.publish_with(|state| {
        state.shots[0].video_path = None;
    });
    h.keys("p");
    assert_eq!(h.toast(), "Video missing on disk");
}

// Detail page (narrow terminal).
key_test!(
    detail_esc_goes_back_to_the_stream,
    Harness::narrow(),
    "\rc",
    "\x1b\x1b",
    |h| {
        assert_eq!((h.ui().pane, h.ui().composer), (Pane::Stream, false));
    }
);
key_test!(
    detail_h_goes_back_when_not_split,
    Harness::narrow(),
    ENTER,
    "h",
    |h| {
        assert_eq!(h.ui().pane, Pane::Stream);
    }
);
key_test!(
    detail_left_pages_to_the_older_shot,
    Harness::narrow(),
    ENTER,
    LEFT,
    |h| {
        assert_eq!((h.selected(), h.active()), (Some(OTHER), some(OTHER)));
    }
);
key_test!(
    detail_right_pages_to_the_newer_shot,
    Harness::narrow(),
    "\r\x1b[D\x1b[D",
    RIGHT,
    |h| {
        assert_eq!(h.selected(), Some(OTHER));
    }
);
key_test!(
    detail_right_stops_at_the_newest_shot,
    Harness::narrow(),
    ENTER,
    RIGHT,
    |h| {
        assert_eq!(h.selected(), Some(MOVIE));
    }
);
key_test!(
    detail_left_stops_at_the_oldest_shot,
    Harness::narrow(),
    "\r\x1b[D\x1b[D\x1b[D",
    LEFT,
    |h| {
        assert_eq!(h.selected(), Some(FIRST));
    }
);
key_test!(
    detail_enter_opens_the_takeover,
    Harness::narrow(),
    ENTER,
    ENTER,
    |h| {
        assert_eq!(
            (h.ui().takeover, h.ui().pane),
            (Some(Takeover::Shot), Pane::Detail)
        );
    }
);
key_test!(
    detail_f_opens_the_takeover,
    Harness::narrow(),
    ENTER,
    "f",
    |h| {
        assert_eq!(h.ui().takeover, Some(Takeover::Shot));
    }
);
key_test!(
    detail_c_opens_the_composer,
    Harness::narrow(),
    ENTER,
    "c",
    |h| {
        assert!(h.ui().composer);
    }
);
key_test!(
    detail_s_marks_seen_and_returns_to_the_stream,
    Harness::narrow(),
    ENTER,
    "s",
    |h| {
        h.until_toast("Seen").await;
        assert_eq!(h.calls(), [format!("seen {MOVIE} None")]);
        assert_eq!(h.ui().pane, Pane::Stream);
    }
);
key_test!(
    detail_p_toggles_the_inline_player,
    Harness::kitty(100),
    ENTER,
    "p",
    |h| {
        assert!(h.ui().inline_player && h.pb().playing);
        h.keys("p");
        assert!(!h.ui().inline_player && !h.pb().playing);
    }
);
key_test!(
    detail_p_without_kitty_explains,
    Harness::narrow(),
    ENTER,
    "p",
    |h| {
        assert_eq!(h.toast(), KITTY_PLAYBACK_HINT);
        assert!(!h.ui().inline_player);
    }
);
key_test!(
    detail_p_on_a_still_says_not_a_movie,
    Harness::kitty(100),
    "j\r",
    "p",
    |h| {
        assert_eq!(h.toast(), "Not a movie");
    }
);
key_test!(
    detail_space_plays_and_pauses,
    Harness::kitty(100),
    ENTER,
    " ",
    |h| {
        assert!(h.ui().inline_player && h.pb().playing);
        h.keys(" ");
        assert!(h.ui().inline_player && !h.pb().playing);
    }
);
key_test!(
    detail_space_restarts_an_ended_movie,
    Harness::kitty(100),
    "\r ",
    "",
    |h| {
        h.app.playback.patch(&PlaybackPatch {
            playing: Some(false),
            ended: Some(true),
            position_ms: Some(10_000.0),
            error: Some(Some("stalled".into())),
            ..PlaybackPatch::default()
        });
        h.keys(" ");
        assert_eq!(
            h.pb(),
            PlaybackState {
                playing: true,
                position_ms: 0.0,
                seek_token: 1,
                ..PlaybackState::default()
            }
        );
    }
);
key_test!(
    detail_comma_seeks_back_while_the_player_is_open,
    Harness::kitty(100),
    "\rp..",
    ",",
    |h| {
        assert_eq!(h.ui().pane, Pane::Detail);
        assert_eq!((h.pb().position_ms, h.pb().seek_token), (5000.0, 3));
        h.keys(",,");
        assert_eq!(h.pb().position_ms, 0.0);
    }
);
key_test!(
    detail_period_seeks_forward_up_to_the_duration,
    Harness::kitty(100),
    "\rp",
    ".",
    |h| {
        assert_eq!(
            (h.pb().position_ms, h.pb().seek_token, h.pb().ended),
            (5000.0, 1, false)
        );
        h.keys("...");
        assert_eq!(h.pb().position_ms, 10_000.0);
    }
);
key_test!(
    detail_comma_opens_settings_while_no_player_is_open,
    Harness::narrow(),
    ENTER,
    ",",
    |h| {
        assert_eq!(h.ui().pane, Pane::Settings);
        assert_eq!(h.pb().seek_token, 0);
    }
);
key_test!(
    detail_period_seeks_even_without_the_player,
    Harness::narrow(),
    ENTER,
    ".",
    |h| {
        assert_eq!((h.ui().pane, h.pb().position_ms), (Pane::Detail, 5000.0));
    }
);
key_test!(
    detail_right_bracket_jumps_to_the_next_chapter,
    Harness::narrow(),
    ENTER,
    "]",
    |h| {
        assert_eq!((h.pb().position_ms, h.pb().seek_token), (2000.0, 1));
        h.keys("]");
        assert_eq!(h.pb().position_ms, 6000.0);
        h.keys("]");
        assert_eq!((h.pb().position_ms, h.pb().seek_token), (6000.0, 3));
    }
);
key_test!(
    detail_left_bracket_jumps_to_the_previous_chapter,
    Harness::narrow(),
    "\r]]",
    "[",
    |h| {
        assert_eq!(h.pb().position_ms, 2000.0);
        h.keys("[");
        assert_eq!(h.pb().position_ms, 0.0);
    }
);
key_test!(
    detail_brackets_do_nothing_without_chapters,
    Harness::narrow(),
    "j\r",
    "][",
    |h| {
        assert_eq!(h.pb(), PlaybackState::default());
    }
);
key_test!(detail_plus_zooms_in, Harness::narrow(), ENTER, "+", |h| {
    assert_eq!(h.ui().zoom, 1.5);
});
key_test!(detail_equals_zooms_in, Harness::narrow(), ENTER, "=", |h| {
    assert_eq!(h.ui().zoom, 1.5);
});
key_test!(
    detail_zoom_stops_at_six,
    Harness::narrow(),
    ENTER,
    "++++++++++++",
    |h| {
        assert_eq!(h.ui().zoom, 6.0);
    }
);
key_test!(
    detail_minus_zooms_out_and_recenters_at_one,
    Harness::narrow(),
    "\r++",
    "-",
    |h| {
        assert_eq!(h.ui().zoom, 1.5);
        h.app.ui.pan_x = 0.2;
        h.keys("-");
        assert_eq!((h.ui().zoom, h.ui().pan_x, h.ui().pan_y), (1.0, 0.5, 0.5));
        h.keys("-");
        assert_eq!(h.ui().zoom, 1.0);
    }
);
key_test!(
    detail_underscore_zooms_out,
    Harness::narrow(),
    "\r++",
    "_",
    |h| {
        assert_eq!(h.ui().zoom, 1.5);
    }
);
key_test!(
    detail_zero_resets_zoom_and_pan,
    Harness::narrow(),
    "\r++",
    "",
    |h| {
        h.app.ui.pan_x = 0.1;
        h.app.ui.pan_y = 0.9;
        h.keys("0");
        assert_eq!((h.ui().zoom, h.ui().pan_x, h.ui().pan_y), (1.0, 0.5, 0.5));
    }
);
key_test!(
    detail_o_reveals_the_shot,
    Harness::narrow(),
    ENTER,
    "o",
    |h| {
        assert_eq!(h.desktop.calls(), [("reveal", MOVIE.to_string())]);
    }
);
key_test!(
    detail_capital_o_opens_the_movie,
    Harness::narrow(),
    ENTER,
    "O",
    |h| {
        assert_eq!(h.desktop.calls(), [("open", MOVIE_VIDEO.to_string())]);
    }
);
key_test!(
    detail_y_copies_the_image,
    Harness::narrow(),
    ENTER,
    "y",
    |h| {
        h.until_toast("Copied image").await;
        assert_eq!(h.desktop.calls(), [("copy", MOVIE.to_string())]);
    }
);

#[tokio::test]
async fn detail_h_does_not_go_back_when_split() {
    let mut h = Harness::narrow().await;
    h.keys(ENTER);
    h.resize(140, 40);
    h.keys("h");
    assert_eq!(h.ui().pane, Pane::Detail);
    h.keys(ESC);
    assert_eq!(h.ui().pane, Pane::Stream);
}

// Full-screen review.
key_test!(takeover_esc_closes, Harness::wide(), "f", ESC, |h| {
    assert_eq!((h.ui().takeover, h.ui().composer), (None, false));
});
key_test!(
    takeover_q_closes_without_quitting,
    Harness::wide(),
    "f",
    "",
    |h| {
        assert_eq!(h.keys("q"), Outcome::Continue);
        assert_eq!(h.ui().takeover, None);
        assert_eq!(h.quits.load(Ordering::SeqCst), 0);
    }
);
key_test!(
    takeover_left_pages_to_the_older_sibling,
    Harness::wide(),
    "f",
    LEFT,
    |h| {
        // Siblings run oldest first, seen ones included: FIRST, SEEN, MOVIE.
        assert_eq!(h.selected(), Some(SEEN));
    }
);
key_test!(
    takeover_h_pages_to_the_older_sibling,
    Harness::wide(),
    "fh",
    "h",
    |h| {
        assert_eq!(h.selected(), Some(FIRST));
        h.keys("h");
        assert_eq!(h.selected(), Some(FIRST));
    }
);
key_test!(
    takeover_right_pages_to_the_newer_sibling,
    Harness::wide(),
    "fhh",
    RIGHT,
    |h| {
        assert_eq!(h.selected(), Some(SEEN));
    }
);
key_test!(
    takeover_l_pages_to_the_newer_sibling,
    Harness::wide(),
    "fh",
    "l",
    |h| {
        assert_eq!(h.selected(), Some(MOVIE));
        h.keys("l");
        assert_eq!(h.selected(), Some(MOVIE));
    }
);
key_test!(
    takeover_up_and_down_do_nothing_unzoomed,
    Harness::wide(),
    "f",
    "\x1b[A\x1b[Bkj",
    |h| {
        assert_eq!(
            (h.selected(), h.ui().pan_x, h.ui().pan_y),
            (Some(MOVIE), 0.5, 0.5)
        );
    }
);
key_test!(
    takeover_left_pans_when_zoomed,
    Harness::wide(),
    "f+",
    LEFT,
    |h| {
        assert_eq!(h.selected(), Some(MOVIE));
        assert!((h.ui().pan_x - 0.38).abs() < 1e-9 && h.ui().pan_y == 0.5);
    }
);
key_test!(
    takeover_h_pans_when_zoomed,
    Harness::wide(),
    "f+",
    "h",
    |h| {
        assert!((h.ui().pan_x - 0.38).abs() < 1e-9);
    }
);
key_test!(
    takeover_right_pans_when_zoomed,
    Harness::wide(),
    "f+",
    RIGHT,
    |h| {
        assert!((h.ui().pan_x - 0.62).abs() < 1e-9);
    }
);
key_test!(
    takeover_l_pans_when_zoomed,
    Harness::wide(),
    "f+",
    "l",
    |h| {
        assert!((h.ui().pan_x - 0.62).abs() < 1e-9);
    }
);
key_test!(
    takeover_up_pans_when_zoomed,
    Harness::wide(),
    "f+",
    UP,
    |h| {
        assert!((h.ui().pan_y - 0.38).abs() < 1e-9 && h.ui().pan_x == 0.5);
    }
);
key_test!(
    takeover_k_pans_when_zoomed,
    Harness::wide(),
    "f+",
    "k",
    |h| {
        assert!((h.ui().pan_y - 0.38).abs() < 1e-9);
    }
);
key_test!(
    takeover_down_pans_when_zoomed,
    Harness::wide(),
    "f+",
    DOWN,
    |h| {
        assert!((h.ui().pan_y - 0.62).abs() < 1e-9);
    }
);
key_test!(
    takeover_j_pans_when_zoomed,
    Harness::wide(),
    "f+",
    "j",
    |h| {
        assert!((h.ui().pan_y - 0.62).abs() < 1e-9);
    }
);
key_test!(
    takeover_pan_stops_at_the_edges,
    Harness::wide(),
    "f+",
    "hhhhhhhh",
    |h| {
        assert_eq!(h.ui().pan_x, 0.0);
    }
);
key_test!(
    takeover_c_opens_the_composer,
    Harness::wide(),
    "f",
    "c",
    |h| {
        assert!(h.ui().composer);
        h.keys("hi");
        assert_eq!(h.app.composer_value(), "hi");
        assert_eq!(h.app.takeover.composer_value(), "hi");
    }
);
key_test!(
    takeover_s_marks_seen_and_closes,
    Harness::wide(),
    "f",
    "s",
    |h| {
        h.until_toast("Seen").await;
        assert_eq!(h.calls(), [format!("seen {MOVIE} None")]);
        assert_eq!(h.ui().takeover, None);
    }
);
key_test!(
    takeover_space_plays_with_kitty,
    Harness::kitty(140),
    "f",
    " ",
    |h| {
        assert!(h.ui().inline_player && h.pb().playing);
    }
);
key_test!(
    takeover_space_without_kitty_explains,
    Harness::wide(),
    "f",
    " ",
    |h| {
        assert_eq!(h.toast(), KITTY_PLAYBACK_HINT);
    }
);
key_test!(
    takeover_space_on_a_still_says_not_a_movie,
    Harness::kitty(140),
    "jf",
    " ",
    |h| {
        assert_eq!(h.toast(), "Not a movie");
    }
);
key_test!(
    takeover_comma_seeks_back,
    Harness::wide(),
    "f..",
    ",",
    |h| {
        assert_eq!((h.pb().position_ms, h.ui().pane), (5000.0, Pane::Stream));
    }
);
key_test!(
    takeover_period_seeks_forward,
    Harness::wide(),
    "f",
    ".",
    |h| {
        assert_eq!((h.pb().position_ms, h.pb().seek_token), (5000.0, 1));
    }
);
key_test!(
    takeover_right_bracket_jumps_to_the_next_chapter,
    Harness::wide(),
    "f",
    "]",
    |h| {
        assert_eq!(h.pb().position_ms, 2000.0);
    }
);
key_test!(
    takeover_left_bracket_jumps_to_the_previous_chapter,
    Harness::wide(),
    "f]]",
    "[",
    |h| {
        assert_eq!(h.pb().position_ms, 2000.0);
    }
);
key_test!(takeover_plus_zooms_in, Harness::wide(), "f", "+", |h| {
    assert_eq!(h.ui().zoom, 1.5);
});
key_test!(takeover_equals_zooms_in, Harness::wide(), "f", "=", |h| {
    assert_eq!(h.ui().zoom, 1.5);
});
key_test!(takeover_minus_zooms_out, Harness::wide(), "f++", "-", |h| {
    assert_eq!(h.ui().zoom, 1.5);
});
key_test!(
    takeover_underscore_zooms_out,
    Harness::wide(),
    "f++",
    "_",
    |h| {
        assert_eq!(h.ui().zoom, 1.5);
    }
);
key_test!(
    takeover_zero_resets_zoom_and_pan,
    Harness::wide(),
    "f++hk",
    "0",
    |h| {
        assert_eq!((h.ui().zoom, h.ui().pan_x, h.ui().pan_y), (1.0, 0.5, 0.5));
    }
);
key_test!(
    takeover_y_copies_the_image,
    Harness::wide(),
    "f",
    "y",
    |h| {
        h.until_toast("Copied image").await;
        assert_eq!(h.desktop.calls(), [("copy", MOVIE.to_string())]);
    }
);
key_test!(
    takeover_o_reveals_the_shot,
    Harness::wide(),
    "f",
    "o",
    |h| {
        assert_eq!(h.desktop.calls(), [("reveal", MOVIE.to_string())]);
    }
);
key_test!(
    takeover_capital_o_opens_the_movie,
    Harness::wide(),
    "f",
    "O",
    |h| {
        assert_eq!(h.desktop.calls(), [("open", MOVIE_VIDEO.to_string())]);
        h.keys("hO");
        assert_eq!(h.desktop.calls().len(), 1);
    }
);
key_test!(
    takeover_swallows_global_keys,
    Harness::wide(),
    "f",
    "12\tr",
    |h| {
        assert_eq!(
            (h.ui().tab, h.ui().takeover),
            (Tab::Shots, Some(Takeover::Shot))
        );
        assert!(h.calls().is_empty());
    }
);

// Friction list.
key_test!(friction_j_moves_down, Harness::wide(), "2", "j", |h| {
    assert_eq!(h.ui().friction_cursor, 1);
    h.keys("j");
    assert_eq!(h.ui().friction_cursor, 1);
});
key_test!(
    friction_down_arrow_moves_down,
    Harness::wide(),
    "2",
    DOWN,
    |h| {
        assert_eq!(h.ui().friction_cursor, 1);
    }
);
key_test!(friction_k_moves_up, Harness::wide(), "2j", "k", |h| {
    assert_eq!(h.ui().friction_cursor, 0);
    h.keys("k");
    assert_eq!(h.ui().friction_cursor, 0);
});
key_test!(friction_up_arrow_moves_up, Harness::wide(), "2j", UP, |h| {
    assert_eq!(h.ui().friction_cursor, 0);
});
key_test!(
    friction_capital_g_goes_to_the_end,
    Harness::wide(),
    "2",
    "G",
    |h| {
        assert_eq!(h.ui().friction_cursor, 1);
    }
);
key_test!(
    friction_end_goes_to_the_end,
    Harness::wide(),
    "2",
    END,
    |h| {
        assert_eq!(h.ui().friction_cursor, 1);
    }
);
key_test!(
    friction_g_goes_to_the_top,
    Harness::wide(),
    "2G",
    "g",
    |h| {
        assert_eq!(h.ui().friction_cursor, 0);
    }
);
key_test!(
    friction_home_goes_to_the_top,
    Harness::wide(),
    "2G",
    HOME,
    |h| {
        assert_eq!(h.ui().friction_cursor, 0);
    }
);
key_test!(
    friction_u_switches_to_history,
    Harness::wide(),
    "2j",
    "u",
    |h| {
        assert_eq!(
            (h.ui().friction_filter, h.ui().friction_cursor),
            (StreamFilter::History, 0)
        );
        h.keys("u");
        assert_eq!(h.ui().friction_filter, StreamFilter::Unseen);
    }
);
key_test!(
    friction_capital_s_marks_every_listed_log_seen,
    Harness::wide(),
    "2",
    "S",
    |h| {
        h.until_toast("Marked 2 stories seen").await;
        assert_eq!(
            h.calls(),
            [
                "log onboarding 20260812T153000Z",
                "log empty 20260810T153000Z"
            ]
        );
    }
);
key_test!(
    friction_enter_opens_the_log,
    Harness::wide(),
    "2",
    ENTER,
    |h| {
        assert_eq!(h.ui().pane, Pane::FrictionLog);
        assert_eq!(h.ui().selected_log.as_deref(), Some("onboarding"));
        assert_eq!(h.ui().selected_run.as_deref(), Some("20260812T153000Z"));
        assert_eq!((h.ui().step_cursor, h.ui().prompt_open), (0, false));
    }
);
key_test!(
    friction_right_opens_the_log,
    Harness::wide(),
    "2j",
    RIGHT,
    |h| {
        assert_eq!(
            (h.ui().pane, h.ui().selected_log.as_deref()),
            (Pane::FrictionLog, Some("empty"))
        );
    }
);
key_test!(friction_l_opens_the_log, Harness::wide(), "2", "l", |h| {
    assert_eq!(h.ui().pane, Pane::FrictionLog);
});
key_test!(
    friction_s_marks_the_log_seen,
    Harness::wide(),
    "2",
    "s",
    |h| {
        h.until_toast("Marked 1 story seen").await;
        assert_eq!(h.calls(), ["log onboarding 20260812T153000Z"]);
    }
);
key_test!(
    friction_s_on_a_seen_log_only_toasts,
    Harness::wide(),
    "2u",
    "s",
    |h| {
        assert_eq!(h.toast(), "Already seen");
        assert!(h.calls().is_empty());
    }
);
key_test!(
    friction_o_reveals_the_log_folder,
    Harness::wide(),
    "2",
    "o",
    |h| {
        assert_eq!(
            h.desktop.calls(),
            [(
                "reveal",
                "/w1/.astroshot/friction-logs/onboarding".to_string()
            )]
        );
    }
);
key_test!(
    friction_reveal_failure_toasts,
    async { Harness::wide().await.failing_desktop() },
    "2",
    "o",
    |h| {
        h.until_toast("Couldn’t reveal folder").await;
    }
);
key_test!(
    friction_s_failure_toasts_the_error,
    Harness::synthetic(140, GraphicsProtocol::None, Writes::Fail),
    "2",
    "s",
    |h| {
        h.until_toast("disk full").await;
        h.keys("S");
        h.until_toast("Couldn’t mark stories as seen").await;
    }
);

// Friction log detail.
key_test!(
    friction_log_esc_goes_back_and_closes_the_prompt,
    Harness::wide(),
    "2\rp",
    ESC,
    |h| {
        assert_eq!((h.ui().pane, h.ui().prompt_open), (Pane::Stream, false));
    }
);
key_test!(friction_log_h_goes_back, Harness::wide(), "2\r", "h", |h| {
    assert_eq!(h.ui().pane, Pane::Stream);
});
key_test!(
    friction_log_j_moves_the_step_cursor,
    Harness::wide(),
    "2\r",
    "j",
    |h| {
        assert_eq!(h.ui().step_cursor, 1);
        h.keys("j");
        assert_eq!(h.ui().step_cursor, 1);
    }
);
key_test!(
    friction_log_down_moves_the_step_cursor,
    Harness::wide(),
    "2\r",
    DOWN,
    |h| {
        assert_eq!(h.ui().step_cursor, 1);
    }
);
key_test!(
    friction_log_k_moves_the_step_cursor_up,
    Harness::wide(),
    "2\rj",
    "k",
    |h| {
        assert_eq!(h.ui().step_cursor, 0);
        h.keys("k");
        assert_eq!(h.ui().step_cursor, 0);
    }
);
key_test!(
    friction_log_up_moves_the_step_cursor_up,
    Harness::wide(),
    "2\rj",
    UP,
    |h| {
        assert_eq!(h.ui().step_cursor, 0);
    }
);
key_test!(
    friction_log_enter_opens_the_step_under_the_cursor,
    Harness::wide(),
    "2\rj",
    ENTER,
    |h| {
        assert_eq!(
            (h.ui().pane, h.ui().step_index, h.ui().image_index),
            (Pane::FrictionStep, 1, 0)
        );
    }
);
key_test!(
    friction_log_right_opens_the_step,
    Harness::wide(),
    "2\r",
    RIGHT,
    |h| {
        assert_eq!((h.ui().pane, h.ui().step_index), (Pane::FrictionStep, 0));
    }
);
key_test!(
    friction_log_enter_needs_a_step,
    Harness::wide(),
    "2j\r",
    ENTER,
    |h| {
        assert_eq!(h.ui().pane, Pane::FrictionLog);
    }
);
key_test!(
    friction_log_p_toggles_the_prompt,
    Harness::wide(),
    "2\r",
    "p",
    |h| {
        assert!(h.ui().prompt_open);
        assert_eq!(
            h.ui().prompt.as_deref(),
            Some("(prompt.md could not be read)")
        );
        h.keys("p");
        assert!(!h.ui().prompt_open);
    }
);
key_test!(
    friction_log_s_marks_the_log_seen,
    Harness::wide(),
    "2\r",
    "s",
    |h| {
        h.until_toast("Marked 1 story seen").await;
        assert_eq!(h.calls(), ["log onboarding 20260812T153000Z"]);
    }
);
key_test!(
    friction_log_right_bracket_switches_to_the_older_run,
    Harness::wide(),
    "2\rj",
    "]",
    |h| {
        assert_eq!(h.ui().selected_run.as_deref(), Some("20260811T153000Z"));
        assert_eq!(h.ui().step_cursor, 0);
        h.keys("]");
        assert_eq!(h.ui().selected_run.as_deref(), Some("20260811T153000Z"));
    }
);
key_test!(
    friction_log_left_bracket_switches_back,
    Harness::wide(),
    "2\r]",
    "[",
    |h| {
        assert_eq!(h.ui().selected_run.as_deref(), Some("20260812T153000Z"));
        h.keys("[");
        assert_eq!(h.ui().selected_run.as_deref(), Some("20260812T153000Z"));
    }
);
key_test!(
    friction_log_brackets_need_more_than_one_run,
    Harness::wide(),
    "2j\r",
    "]",
    |h| {
        assert_eq!(h.ui().selected_run.as_deref(), Some("20260810T153000Z"));
    }
);

// Friction step page.
key_test!(
    friction_step_esc_goes_back_to_the_log,
    Harness::wide(),
    "2\r\r",
    ESC,
    |h| {
        assert_eq!(h.ui().pane, Pane::FrictionLog);
    }
);
key_test!(
    friction_step_h_goes_back_to_the_log,
    Harness::wide(),
    "2\r\r",
    "h",
    |h| {
        assert_eq!(h.ui().pane, Pane::FrictionLog);
    }
);
key_test!(
    friction_step_right_goes_to_the_next_step,
    Harness::wide(),
    "2\r\r]",
    RIGHT,
    |h| {
        assert_eq!(
            (h.ui().step_index, h.ui().step_cursor, h.ui().image_index),
            (1, 1, 0)
        );
        h.keys(RIGHT);
        assert_eq!(h.ui().step_index, 1);
    }
);
key_test!(
    friction_step_left_goes_to_the_previous_step,
    Harness::wide(),
    "2\r\r\x1b[C",
    LEFT,
    |h| {
        assert_eq!((h.ui().step_index, h.ui().step_cursor), (0, 0));
        h.keys(LEFT);
        assert_eq!(h.ui().step_index, 0);
    }
);
key_test!(
    friction_step_right_bracket_shows_the_next_image,
    Harness::wide(),
    "2\r\r",
    "]",
    |h| {
        assert_eq!(h.ui().image_index, 1);
        h.keys("]");
        assert_eq!(h.ui().image_index, 1);
    }
);
key_test!(
    friction_step_left_bracket_shows_the_previous_image,
    Harness::wide(),
    "2\r\r]",
    "[",
    |h| {
        assert_eq!(h.ui().image_index, 0);
        h.keys("[");
        assert_eq!(h.ui().image_index, 0);
    }
);
key_test!(
    friction_step_enter_opens_the_takeover,
    Harness::wide(),
    "2\r\r",
    ENTER,
    |h| {
        assert_eq!(h.ui().takeover, Some(Takeover::Step));
    }
);
key_test!(
    friction_step_f_opens_the_takeover,
    Harness::wide(),
    "2\r\r",
    "f",
    |h| {
        assert_eq!(h.ui().takeover, Some(Takeover::Step));
    }
);
key_test!(
    friction_step_space_opens_the_takeover,
    Harness::wide(),
    "2\r\r",
    " ",
    |h| {
        assert_eq!(h.ui().takeover, Some(Takeover::Step));
    }
);
key_test!(
    friction_step_o_reveals_the_screenshot,
    Harness::wide(),
    "2\r\r]",
    "o",
    |h| {
        assert_eq!(h.desktop.calls(), [("reveal", STEP_SHOT_B.to_string())]);
    }
);
key_test!(
    friction_step_y_copies_the_screenshot,
    Harness::wide(),
    "2\r\r",
    "y",
    |h| {
        h.until_toast("Copied image").await;
        assert_eq!(h.desktop.calls(), [("copy", STEP_SHOT_A.to_string())]);
    }
);
key_test!(
    friction_step_o_and_y_need_a_screenshot,
    Harness::wide(),
    "2\rj\r",
    "oy",
    |h| {
        assert!(h.desktop.calls().is_empty());
    }
);

// Friction step takeover.
key_test!(
    step_takeover_esc_closes,
    Harness::wide(),
    "2\r\rf",
    ESC,
    |h| {
        assert_eq!((h.ui().takeover, h.ui().pane), (None, Pane::FrictionStep));
    }
);
key_test!(
    step_takeover_q_closes_without_quitting,
    Harness::wide(),
    "2\r\rf",
    "",
    |h| {
        assert_eq!(h.keys("q"), Outcome::Continue);
        assert_eq!(h.ui().takeover, None);
    }
);
key_test!(
    step_takeover_right_goes_to_the_next_step,
    Harness::wide(),
    "2\r\rf]",
    RIGHT,
    |h| {
        // Unlike the step page, the takeover leaves the log's step cursor alone.
        assert_eq!(
            (h.ui().step_index, h.ui().step_cursor, h.ui().image_index),
            (1, 0, 0)
        );
        h.keys(RIGHT);
        assert_eq!(h.ui().step_index, 1);
    }
);
key_test!(
    step_takeover_l_goes_to_the_next_step,
    Harness::wide(),
    "2\r\rf",
    "l",
    |h| {
        assert_eq!(h.ui().step_index, 1);
    }
);
key_test!(
    step_takeover_left_goes_to_the_previous_step,
    Harness::wide(),
    "2\r\rfl",
    LEFT,
    |h| {
        assert_eq!(h.ui().step_index, 0);
        h.keys(LEFT);
        assert_eq!(h.ui().step_index, 0);
    }
);
key_test!(
    step_takeover_h_goes_to_the_previous_step,
    Harness::wide(),
    "2\r\rfl",
    "h",
    |h| {
        assert_eq!(h.ui().step_index, 0);
    }
);
key_test!(
    step_takeover_right_bracket_shows_the_next_image,
    Harness::wide(),
    "2\r\rf",
    "]",
    |h| {
        assert_eq!(h.ui().image_index, 1);
        h.keys("]");
        assert_eq!(h.ui().image_index, 1);
    }
);
key_test!(
    step_takeover_left_bracket_shows_the_previous_image,
    Harness::wide(),
    "2\r\rf]",
    "[",
    |h| {
        assert_eq!(h.ui().image_index, 0);
    }
);

// Global keys, help, settings.
key_test!(q_quits_from_a_list, Harness::wide(), "", "", |h| {
    assert_eq!(h.keys("q"), Outcome::Quit);
    assert_eq!(h.quits.load(Ordering::SeqCst), 1);
});
key_test!(
    q_quits_from_the_detail_page,
    Harness::narrow(),
    ENTER,
    "",
    |h| {
        assert_eq!(h.keys("q"), Outcome::Quit);
    }
);
key_test!(ctrl_c_quits, Harness::wide(), "", "", |h| {
    assert_eq!(h.keys(CTRL_C), Outcome::Quit);
});
key_test!(
    ctrl_c_quits_from_the_composer_help_and_takeover,
    Harness::wide(),
    "",
    "",
    |h| {
        h.keys("c");
        assert_eq!(h.keys(CTRL_C), Outcome::Quit);
        h.keys("\x1b?");
        assert_eq!(h.keys(CTRL_C), Outcome::Quit);
        h.keys("?f");
        assert_eq!(h.keys(CTRL_C), Outcome::Quit);
    }
);
key_test!(question_mark_opens_help, Harness::wide(), "", "?", |h| {
    assert!(h.ui().help);
});
key_test!(help_question_mark_closes, Harness::wide(), "?", "?", |h| {
    assert!(!h.ui().help);
});
key_test!(help_esc_closes, Harness::wide(), "?", ESC, |h| {
    assert!(!h.ui().help);
});
key_test!(
    help_q_closes_without_quitting,
    Harness::wide(),
    "?",
    "",
    |h| {
        assert_eq!(h.keys("q"), Outcome::Continue);
        assert!(!h.ui().help);
    }
);
key_test!(
    help_swallows_every_other_key,
    Harness::wide(),
    "?",
    "j2,fsr",
    |h| {
        assert_eq!(
            *h.ui(),
            UiState {
                help: true,
                ..UiState::default()
            }
        );
        assert!(h.calls().is_empty());
    }
);
key_test!(
    two_switches_to_friction_logs,
    Harness::narrow(),
    ENTER,
    "2",
    |h| {
        assert_eq!(
            (h.ui().tab, h.ui().pane, h.ui().takeover),
            (Tab::FrictionLogs, Pane::Stream, None)
        );
    }
);
key_test!(one_switches_to_shots, Harness::wide(), "2\r", "1", |h| {
    assert_eq!((h.ui().tab, h.ui().pane), (Tab::Shots, Pane::Stream));
});
key_test!(
    tab_switches_between_the_tabs,
    Harness::narrow(),
    ENTER,
    "\t",
    |h| {
        assert_eq!((h.ui().tab, h.ui().pane), (Tab::FrictionLogs, Pane::Stream));
        h.keys("\t");
        assert_eq!(h.ui().tab, Tab::Shots);
    }
);
key_test!(comma_toggles_settings, Harness::wide(), "", ",", |h| {
    assert_eq!(h.ui().pane, Pane::Settings);
    h.keys(",");
    assert_eq!(h.ui().pane, Pane::Stream);
});
key_test!(settings_esc_closes, Harness::wide(), ",", ESC, |h| {
    assert_eq!(h.ui().pane, Pane::Stream);
});
key_test!(settings_h_closes, Harness::wide(), ",", "h", |h| {
    assert_eq!(h.ui().pane, Pane::Stream);
});
key_test!(settings_left_closes, Harness::wide(), ",", LEFT, |h| {
    assert_eq!(h.ui().pane, Pane::Stream);
});
key_test!(
    settings_swallows_list_keys,
    Harness::wide(),
    ",",
    "jfsc",
    |h| {
        assert_eq!(
            *h.ui(),
            UiState {
                pane: Pane::Settings,
                ..UiState::default()
            }
        );
        assert_eq!(h.stub().opened.load(Ordering::SeqCst), 0);
    }
);
key_test!(r_rescans_with_force, Harness::wide(), "", "r", |h| {
    assert_eq!(h.toast(), "Scanning…");
    h.until("the rescan call", |h| h.calls() == ["rescan force=true"])
        .await;
});

// ---- Composer ------------------------------------------------------------------------

key_test!(
    composer_takes_every_key_while_open,
    Harness::wide(),
    "c",
    "q?2,j",
    |h| {
        assert_eq!(h.app.composer_value(), "q?2,j");
        assert_eq!(h.app.detail.composer_value(), "q?2,j");
        assert_eq!(
            (h.ui().help, h.ui().tab, h.ui().pane),
            (false, Tab::Shots, Pane::Stream)
        );
        assert_eq!(h.quits.load(Ordering::SeqCst), 0);
    }
);
key_test!(
    composer_esc_cancels_and_drops_the_draft,
    Harness::wide(),
    "cdraft",
    ESC,
    |h| {
        assert!(!h.ui().composer);
        h.keys("c");
        assert_eq!(h.app.composer_value(), "");
    }
);
key_test!(
    composer_enter_sends_the_feedback,
    Harness::wide(),
    "chello",
    "",
    |h| {
        h.keys(ENTER);
        assert!(h.ui().busy && h.ui().composer);
        h.until_toast("Comment added").await;
        assert_eq!(h.calls(), [format!("comment {MOVIE} hello")]);
        assert!(!h.ui().busy && !h.ui().composer);
    }
);
key_test!(
    composer_enter_with_blank_text_only_closes,
    Harness::wide(),
    "c  ",
    ENTER,
    |h| {
        assert!(!h.ui().composer && !h.ui().busy);
        assert!(h.calls().is_empty());
    }
);
key_test!(
    composer_failure_keeps_the_draft_and_shows_the_error,
    Harness::synthetic(140, GraphicsProtocol::None, Writes::Fail),
    "chello",
    ENTER,
    |h| {
        h.until("the error", |h| h.ui().error.is_some()).await;
        assert_eq!(h.ui().error.as_deref(), Some("disk full"));
        assert!(h.ui().composer && !h.ui().busy);
        assert_eq!(h.app.composer_value(), "hello");
        assert!(h.screen().contains("disk full"));
        // Reopening the composer clears the banner.
        h.keys("\x1bc");
        assert_eq!(h.ui().error, None);
    }
);

#[tokio::test]
async fn pastes_go_to_the_open_composer_and_nowhere_else() {
    let mut h = Harness::wide().await;
    h.app.handle(Event::Paste("q".into()));
    h.app.handle(Event::Paste("ignored".into()));
    assert_eq!(*h.ui(), UiState::default());

    h.keys("c");
    h.app.handle(Event::Paste("looks good".into()));
    assert_eq!(h.app.composer_value(), "looks good");
    // A paste that ends in a newline submits, as a typed Enter would.
    h.app.handle(Event::Paste(" to me\n".into()));
    h.until_toast("Comment added").await;
    assert_eq!(h.calls(), [format!("comment {MOVIE} looks good to me")]);

    h.keys("fc");
    h.app.handle(Event::Paste("in review".into()));
    assert_eq!(h.app.takeover.composer_value(), "in review");
}

#[tokio::test]
async fn key_releases_are_ignored() {
    let mut h = Harness::wide().await;
    let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert_eq!(h.app.handle(Event::Key(release)), Outcome::Continue);
    assert_eq!(h.quits.load(Ordering::SeqCst), 0);
}

// ---- Effects ------------------------------------------------------------------------

#[tokio::test]
async fn paging_to_another_shot_resets_player_composer_error_and_zoom() {
    let mut h = Harness::kitty(100).await;
    h.keys("\rp.+");
    h.app.ui.error = Some("old".into());
    h.app.ui.pan_x = 0.1;
    assert!(h.ui().inline_player && h.pb().position_ms == 5000.0);
    h.keys(LEFT);
    assert_eq!(h.active(), some(OTHER));
    assert!(!h.ui().inline_player && !h.ui().composer);
    assert_eq!(h.pb(), PlaybackState::default());
    assert_eq!(
        (h.ui().error.clone(), h.ui().zoom, h.ui().pan_x),
        (None, 1.0, 0.5)
    );
}

#[tokio::test]
async fn the_active_shot_leaving_the_stream_closes_its_composer() {
    let mut h = Harness::wide().await;
    h.keys("cdraft");
    h.services.store.publish_with(|state| {
        state.shots.remove(0);
    });
    h.app.sync();
    assert!(!h.ui().composer);
    assert_eq!(h.active(), some(OTHER));
    h.keys("c");
    assert_eq!(h.app.composer_value(), "");
}

#[tokio::test]
async fn arrivals_are_announced_once_with_a_longer_toast() {
    use crate::data::store::StoreEvent;
    let mut h = Harness::wide().await;
    let arrival = |kind, at| StoreEvent {
        kind,
        shot: synthetic_shots().remove(0),
        at,
    };
    h.services
        .store
        .publish_with(|state| state.last_event = Some(arrival(StoreEventKind::NewShot, 10.0)));
    h.app.sync();
    assert_eq!(h.toast(), "New · w1 · f · Shot 0003");
    let toast = h.ui().toast.clone().unwrap();
    assert_eq!((toast.duration_ms, toast.at), (5500, FIXED_NOW));

    // The same event is not announced again, and updates never are.
    h.app.ui.toast = None;
    h.app.sync();
    assert_eq!(h.toast(), "");
    h.services
        .store
        .publish_with(|state| state.last_event = Some(arrival(StoreEventKind::UpdatedShot, 11.0)));
    h.app.sync();
    assert_eq!(h.toast(), "");
    h.services
        .store
        .publish_with(|state| state.last_event = Some(arrival(StoreEventKind::NewShot, 12.0)));
    h.app.sync();
    assert_eq!(h.toast(), "New · w1 · f · Shot 0003");
}

#[tokio::test]
async fn toasts_clear_themselves_and_an_old_timer_leaves_a_newer_toast_alone() {
    let mut h = Harness::wide().await;
    h.app.toast_for("first".into(), 40);
    assert_eq!(h.screen().lines().last().unwrap().trim(), "first");
    h.until("the toast to clear", |h| h.ui().toast.is_none())
        .await;
    assert!(h.screen().lines().last().unwrap().starts_with("↑↓ move"));

    h.app.toast_for("short".into(), 30);
    h.app.toast_for("long".into(), 60_000);
    tokio::time::sleep(Duration::from_millis(120)).await;
    h.app.sync();
    assert_eq!(h.toast(), "long");
    // An expiry for a toast that is no longer showing changes nothing.
    h.app.apply(Done::ToastExpired(1));
    assert_eq!(h.toast(), "long");
}

#[tokio::test]
async fn a_seen_write_that_lands_after_paging_on_leaves_the_takeover_open() {
    let mut h = Harness::synthetic(140, GraphicsProtocol::None, Writes::Hang).await;
    h.keys("fs");
    assert!(h.ui().busy);
    assert!(h.screen().contains("Saving review…"));
    h.keys(LEFT);
    assert_eq!(h.selected(), Some(SEEN));
    h.stub().gate.notify_one();
    h.until("the write", |h| !h.ui().busy).await;
    assert_eq!(h.toast(), "Seen");
    assert_eq!(h.ui().takeover, Some(Takeover::Shot));
}

#[tokio::test]
async fn failed_writes_show_the_error_and_failed_bulk_marks_toast() {
    let mut h = Harness::synthetic(140, GraphicsProtocol::None, Writes::Fail).await;
    h.keys("fs");
    h.until("the error", |h| h.ui().error.is_some()).await;
    assert_eq!(h.ui().error.as_deref(), Some("disk full"));
    assert!(!h.ui().busy);
    assert_eq!(h.ui().takeover, Some(Takeover::Shot));
    assert!(h.screen().contains("disk full"));

    h.keys("\x1bS");
    h.until_toast("Couldn’t mark frames as seen").await;
    assert!(!h.ui().bulk_busy);
    h.app
        .apply(Done::SeenAll(MarkManyResult { ok: 2, failed: 1 }));
    assert_eq!(h.toast(), "Marked 2 seen; 1 failed");
}

#[tokio::test]
async fn resizing_switches_between_split_and_page_layouts() {
    let mut h = Harness::wide().await;
    assert!(h.frame()[3].contains('│'));
    assert!(h.screen().contains("⏎ review"));
    h.resize(100, 30);
    assert_eq!(
        h.app.size(),
        TerminalSize {
            columns: 100,
            rows: 30
        }
    );
    let frame = h.frame();
    assert_eq!(frame.len(), 30);
    assert!(!frame[3].contains('│'));
    assert!(frame[29].contains("⏎ detail"));
    // A zero size falls back to 80x24, like `stdout.columns || 80`.
    h.app.handle(Event::Resize(0, 0));
    assert_eq!(
        h.app.size(),
        TerminalSize {
            columns: 80,
            rows: 24
        }
    );
}

#[tokio::test]
async fn children_that_leave_the_frame_are_unmounted() {
    let mut h = Harness::kitty(140).await;
    h.frame();
    assert!(h.app.stream_mounted && h.app.detail_mounted && !h.app.takeover_mounted);
    assert!(h.app.detail.picture().is_some());
    h.keys("f");
    assert!(!h.app.stream_mounted && !h.app.detail_mounted && h.app.takeover_mounted);
    // The detail pane's state was replaced: its preview picture is gone.
    assert!(h.app.detail.picture().is_none());
    assert!(h.app.takeover.picture().is_some());
    h.keys("\x1b2\r\r");
    assert!(h.app.friction_step_mounted && !h.app.takeover_mounted);
    assert!(h.app.takeover.picture().is_none());
    h.keys("?");
    assert!(!h.app.friction_step_mounted);
}

#[tokio::test]
async fn tabs_and_header_carry_the_ink_styles() {
    let mut h = Harness::wide().await;
    let area = Rect::new(0, 0, 140, 40);
    let mut buf = Buffer::empty(area);
    h.app.render_into(area, &mut buf);
    use crate::ui::testing::{fg_at, modifier_at};
    // "● Astroshots" brand bold, status green, right side muted.
    assert_eq!(fg_at(&buf, 1, 0), THEME.brand);
    assert_eq!(modifier_at(&buf, 1, 0), Modifier::BOLD);
    assert_eq!(fg_at(&buf, 15, 0), THEME.green);
    assert_eq!(fg_at(&buf, 138, 0), THEME.muted);
    // Active tab is bold + inverse; its unseen count is amber bold; the other tab is muted.
    assert_eq!(fg_at(&buf, 1, 1), THEME.text);
    assert_eq!(modifier_at(&buf, 1, 1), Modifier::BOLD | Modifier::REVERSED);
    assert_eq!(fg_at(&buf, 11, 1), THEME.amber);
    assert_eq!(modifier_at(&buf, 11, 1), Modifier::BOLD);
    assert_eq!(fg_at(&buf, 15, 1), THEME.muted);
    assert_eq!(modifier_at(&buf, 15, 1), Modifier::empty());
    // The split separator is faint.
    assert_eq!(buf[(58, 3)].symbol(), "│");
    assert_eq!(fg_at(&buf, 58, 3), THEME.faint);

    h.keys("2");
    let mut buf = Buffer::empty(area);
    h.app.render_into(area, &mut buf);
    assert_eq!(fg_at(&buf, 1, 1), THEME.muted);
    assert_eq!(
        modifier_at(&buf, 15, 1),
        Modifier::BOLD | Modifier::REVERSED
    );
}

#[test]
fn friction_log_directory_is_the_parent_of_the_log_folder() {
    assert_eq!(
        friction_log_directory(&log("onboarding", vec![])),
        "/w1/.astroshot/friction-logs"
    );
}

// ---- Store round trips (review.json on disk) -------------------------------------------

struct Disk {
    h: Harness,
    feature: PathBuf,
    friction_run: PathBuf,
}

async fn disk(cols: u16) -> Disk {
    let root = capture_sized_root();
    seed(
        root.path(),
        &SeedOptions {
            friction: true,
            second: false,
        },
        truth().file_time,
    );
    let feature = root.path().join("demo-app/.astroshot/checkout");
    let friction_run = root
        .path()
        .join("demo-app/.astroshot/friction-logs/onboarding/runs")
        .join(FRICTION_RUN);
    let h = Harness::open(Setup {
        cols,
        rows: 40,
        graphics: GraphicsProtocol::None,
        roots: vec![root.path().to_string_lossy().into_owned()],
        start: true,
        actions: None,
        root: Some(root),
    })
    .await;
    Disk {
        h,
        feature,
        friction_run,
    }
}

fn review_json(directory: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(directory.join("review.json")).unwrap()).unwrap()
}

fn is_sha256(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s_writes_a_seen_decision_with_the_image_hash_to_review_json() {
    let Disk { mut h, feature, .. } = disk(140).await;
    assert!(h.screen().contains("Unseen (3)"));
    h.keys("s");
    // The "Seen" toast is set when the write's completion is applied, which
    // can come after the store has already reloaded the shot (the shot leaves
    // Unseen first). Wait for the completion itself, then for the reload.
    h.until_toast("Seen").await;
    h.until("the shot to leave Unseen", |h| {
        h.screen().contains("Unseen (2)")
    })
    .await;
    let review = review_json(&feature);
    assert_eq!(review["version"], 1);
    assert_eq!(review["run_id"], "checkout-e2e");
    let entry = &review["reviews"]["0003-journey.png"];
    assert_eq!(entry["decision"], "seen");
    assert!(is_sha256(&entry["image_sha256"]), "{entry}");
    assert_eq!(review["reviews"].as_object().unwrap().len(), 1);
    // The store reloaded the shot: it is in History now.
    h.keys("u");
    assert!(h.screen().contains("History (1)"));
    assert!(h.screen().contains("● Seen"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feedback_writes_a_comment_only_entry_to_review_json() {
    let Disk { mut h, feature, .. } = disk(100).await;
    h.keys("cNeeds more contrast\r");
    h.until("the comment to show", |h| {
        let screen = h.screen();
        screen.contains("Reviewer") && !screen.contains("Share feedback")
    })
    .await;
    let review = review_json(&feature);
    let entry = &review["reviews"]["0003-journey.png"];
    assert_eq!(entry["comments"][0]["body"], "Needs more contrast");
    assert_eq!(entry["comments"].as_array().unwrap().len(), 1);
    assert!(entry.get("decision").is_none(), "{entry}");
    assert_eq!(review["run_id"], "checkout-e2e");
    assert_eq!(h.ui().pane, Pane::Detail);
    assert!(h.screen().contains("FEEDBACK 1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capital_s_marks_every_unseen_shot_in_review_json() {
    let Disk { mut h, feature, .. } = disk(140).await;
    h.keys("S");
    h.until_toast("Marked 3 frames seen").await;
    let review = review_json(&feature);
    let reviews = review["reviews"].as_object().unwrap();
    assert_eq!(
        reviews.keys().collect::<Vec<_>>(),
        // `review.json` is written with sorted keys.
        [
            "0001-welcome.png",
            "0002-next-steps.png",
            "0003-journey.png"
        ]
    );
    for entry in reviews.values() {
        assert_eq!(entry["decision"], "seen");
        assert!(is_sha256(&entry["image_sha256"]));
    }
    assert!(h.screen().contains("Unseen (0)"));
    assert!(h.screen().contains("You’re all caught up"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s_on_a_friction_log_writes_the_run_sidecar() {
    let Disk {
        mut h,
        friction_run,
        ..
    } = disk(140).await;
    h.keys("2\r");
    assert!(h.screen().contains("Improve rollup · 1"));
    // `p` reads prompt.md from disk.
    h.keys("p");
    assert_eq!(h.ui().prompt.as_deref(), Some("# Onboarding\n"));
    h.keys("ps");
    h.until_toast("Marked 1 story seen").await;
    let review = review_json(&friction_run);
    assert_eq!(review["run_id"], FRICTION_RUN);
    assert_eq!(review["reviews"]["log.jsonl"]["decision"], "seen");
    h.until("the tab count to clear", |h| {
        h.frame()[0].contains("Live review stream") && !h.screen().contains("s Seen")
    })
    .await;
}
