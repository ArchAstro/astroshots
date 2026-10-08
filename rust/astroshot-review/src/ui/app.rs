//! Port of `packages/astroshot-review/src/ui/app.tsx`.
//!
//! The tray shell: header, tabs, the active pane, and the keymap. Navigation
//! follows the macOS app: Detail pages the whole stream newest-first while
//! full-screen review pages run siblings oldest-first.
//!
//! # React component -> state struct
//!
//! - `useState<UiState>` is [`UiState`], owned by [`App`]. `ui.playback` is the
//!   shared [`Playback`] handle the detail pane and the takeover also hold.
//! - `useInput` is [`App::handle`]; the JSX is [`App::render`].
//! - `useEffect`s (toast expiry, arrival toasts, scroll windows, the per-shot
//!   reset) run in [`App::sync`], which `handle` and `render` both call, so
//!   state is settled before a frame is drawn and before the next key.
//! - `void store.markShotSeen(...)` and the other unawaited promises are
//!   `tokio::spawn`ed; each task posts a completion to a queue that `sync`
//!   drains, then wakes the event loop.
//! - A child that is not part of a frame is unmounted in React. Here its state
//!   struct is replaced by a fresh one after the frame, which drops the
//!   pictures it had registered with the image layer.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{StatefulWidget, Widget};
use tokio::task::JoinHandle;

use super::chrome::{HintBar, KeyHint, Toast};
use super::context::AppServices;
use super::detail::{DetailMode, DetailPane, DetailPaneProps, DetailPaneState, DetailPosition};
use super::friction::{
    FrictionFilterBar, FrictionList, FrictionListProps, FrictionLogDetail, FrictionLogDetailProps,
    FrictionStepDetail, FrictionStepDetailProps, FrictionStepState, FrictionStepTakeover, StepMode,
    friction_scroll,
};
use super::help::HelpOverlay;
use super::hooks::{ClockHook, NowFn, StoreStateHook, TerminalSize, TerminalSizeHook, Wake};
use super::inline::space_between;
use super::movie_player::{Playback, PlaybackPatch, PlaybackState};
use super::put_spans;
use super::selectors::{
    StreamCounts, StreamFilter, contiguous_groups, filter_friction_logs, filter_shots,
    friction_state, latest_run, review_siblings, review_state_of, stream_counts,
};
use super::settings::SettingsPane;
use super::stream::{
    FilterBar, StreamItem, StreamList, StreamListState, StreamProps, flatten_stream,
    next_navigable, scroll_window,
};
use super::system::{copy_image_to_clipboard, open_with_default_app, reveal_in_file_manager};
use super::takeover::{ReviewTakeover, ReviewTakeoverProps, ReviewTakeoverState};
use super::text_input;
use super::theme::{THEME, truncate};
use crate::data::store::{
    MarkManyResult, RescanOptions, ReviewStore, ScanPhase, StoreEventKind, StoreState,
};
use crate::terminal::probe::GraphicsProtocol;
use astroshot_engine::review_data::model::{FrictionLog, FrictionRun, ReviewState, Shot};
use astroshot_engine::review_data::paths::dirname;

/// `type Tab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Shots,
    FrictionLogs,
}

/// `type Pane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Stream,
    Detail,
    Settings,
    FrictionLog,
    FrictionStep,
}

/// `type Takeover` (`null` is `Option::None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takeover {
    Shot,
    Step,
}

/// `ui.toast`. `id` stands for the object identity the TS timer compares.
#[derive(Debug, Clone, PartialEq)]
pub struct ToastState {
    pub message: String,
    pub at: f64,
    pub duration_ms: u64,
    id: u64,
}

/// `interface UiState`, minus `playback` (the shared [`Playback`] handle).
#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub tab: Tab,
    pub pane: Pane,
    pub stream_filter: StreamFilter,
    pub movies_only: bool,
    pub collapsed: HashSet<String>,
    pub cursor: usize,
    pub scroll_top: usize,
    pub selected_shot: Option<String>,
    pub friction_filter: StreamFilter,
    pub friction_cursor: usize,
    pub friction_scroll_top: usize,
    pub selected_log: Option<String>,
    pub selected_run: Option<String>,
    /// Signed: the TS clamps against `steps.length - 1`, which is `-1` for a run with no steps.
    pub step_cursor: isize,
    pub step_index: isize,
    pub image_index: usize,
    pub prompt_open: bool,
    pub prompt: Option<String>,
    pub takeover: Option<Takeover>,
    pub composer: bool,
    pub help: bool,
    pub toast: Option<ToastState>,
    pub busy: bool,
    pub bulk_busy: bool,
    pub error: Option<String>,
    pub inline_player: bool,
    pub zoom: f64,
    pub pan_x: f64,
    pub pan_y: f64,
}

const SPLIT_BREAKPOINT: u16 = 120;
const SEEK_STEP_MS: f64 = 5000.0;
const DEFAULT_TOAST_MS: u64 = 1600;
const KITTY_PLAYBACK_HINT: &str =
    "In-tray playback needs Kitty graphics — press O to open the movie";

/// `initialPlayback()`.
fn initial_playback() -> PlaybackState {
    PlaybackState::default()
}

impl Default for UiState {
    /// `initialState`.
    fn default() -> Self {
        Self {
            tab: Tab::Shots,
            pane: Pane::Stream,
            stream_filter: StreamFilter::Unseen,
            movies_only: false,
            collapsed: HashSet::new(),
            cursor: 0,
            scroll_top: 0,
            selected_shot: None,
            friction_filter: StreamFilter::Unseen,
            friction_cursor: 0,
            friction_scroll_top: 0,
            selected_log: None,
            selected_run: None,
            step_cursor: 0,
            step_index: 0,
            image_index: 0,
            prompt_open: false,
            prompt: None,
            takeover: None,
            composer: false,
            help: false,
            toast: None,
            busy: false,
            bulk_busy: false,
            error: None,
            inline_player: false,
            zoom: 1.0,
            pan_x: 0.5,
            pan_y: 0.5,
        }
    }
}

/// `interface AppProps`.
#[derive(Default)]
pub struct AppProps {
    pub on_quit: Option<Box<dyn FnMut() + Send>>,
}

/// What the event loop should do after [`App::handle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// `exit()`: leave the loop and tear the terminal down.
    Quit,
}

pub type ActionFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The desktop integrations the keymap launches (`o`, `O`, `y`). Injected so
/// tests record the calls instead of spawning `open` / `osascript`.
pub trait Desktop: Send + Sync {
    fn reveal_in_file_manager(&self, target: String) -> ActionFuture<anyhow::Result<()>>;
    fn open_with_default_app(&self, target: String) -> ActionFuture<anyhow::Result<()>>;
    fn copy_image_to_clipboard(&self, image_path: String) -> ActionFuture<anyhow::Result<()>>;
}

/// [`Desktop`] over `ui::system`.
pub struct SystemDesktop;

impl Desktop for SystemDesktop {
    fn reveal_in_file_manager(&self, target: String) -> ActionFuture<anyhow::Result<()>> {
        Box::pin(async move { reveal_in_file_manager(&target).await })
    }

    fn open_with_default_app(&self, target: String) -> ActionFuture<anyhow::Result<()>> {
        Box::pin(async move { open_with_default_app(&target).await })
    }

    fn copy_image_to_clipboard(&self, image_path: String) -> ActionFuture<anyhow::Result<()>> {
        Box::pin(async move { copy_image_to_clipboard(&image_path).await })
    }
}

/// The store mutations the keymap starts. The app reads state from the
/// `ReviewStore` in [`AppServices`]; writes go through this seam so tests can
/// hold a write open (busy) or fail it (error banner).
pub trait ReviewActions: Send + Sync {
    fn rescan(&self, options: RescanOptions) -> ActionFuture<()>;
    fn mark_opened(&self);
    fn mark_shot_seen(
        &self,
        shot: Shot,
        comment: Option<String>,
    ) -> ActionFuture<anyhow::Result<Shot>>;
    fn add_shot_comment(&self, shot: Shot, body: String) -> ActionFuture<anyhow::Result<Shot>>;
    fn mark_many_seen(&self, shots: Vec<Shot>) -> ActionFuture<MarkManyResult>;
    fn mark_friction_run_seen(
        &self,
        log: FrictionLog,
        run: FrictionRun,
    ) -> ActionFuture<anyhow::Result<()>>;
}

/// [`ReviewActions`] over the real store.
pub struct StoreActions(pub Arc<ReviewStore>);

impl ReviewActions for StoreActions {
    fn rescan(&self, options: RescanOptions) -> ActionFuture<()> {
        let store = self.0.clone();
        Box::pin(async move { store.rescan(options).await })
    }

    fn mark_opened(&self) {
        self.0.mark_opened();
    }

    fn mark_shot_seen(
        &self,
        shot: Shot,
        comment: Option<String>,
    ) -> ActionFuture<anyhow::Result<Shot>> {
        let store = self.0.clone();
        Box::pin(async move { store.mark_shot_seen(&shot, comment.as_deref()).await })
    }

    fn add_shot_comment(&self, shot: Shot, body: String) -> ActionFuture<anyhow::Result<Shot>> {
        let store = self.0.clone();
        Box::pin(async move { store.add_shot_comment(&shot, &body).await })
    }

    fn mark_many_seen(&self, shots: Vec<Shot>) -> ActionFuture<MarkManyResult> {
        let store = self.0.clone();
        Box::pin(async move { store.mark_many_seen(&shots).await })
    }

    fn mark_friction_run_seen(
        &self,
        log: FrictionLog,
        run: FrictionRun,
    ) -> ActionFuture<anyhow::Result<()>> {
        let store = self.0.clone();
        Box::pin(async move { store.mark_friction_run_seen(&log, &run).await })
    }
}

/// Everything [`App::with_options`] lets a caller replace.
pub struct AppOptions {
    pub props: AppProps,
    pub size: TerminalSize,
    pub desktop: Arc<dyn Desktop>,
    /// Defaults to [`StoreActions`] over `services.store`.
    pub actions: Option<Arc<dyn ReviewActions>>,
    /// `Date.now` for the clock hook and toast timestamps.
    pub now: Option<NowFn>,
}

impl AppOptions {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            props: AppProps::default(),
            size,
            desktop: Arc::new(SystemDesktop),
            actions: None,
            now: None,
        }
    }
}

/// A finished background action, applied by [`App::sync`].
enum Done {
    Seen {
        path: String,
        result: Result<(), String>,
    },
    Feedback {
        path: String,
        result: Result<(), String>,
    },
    SeenAll(MarkManyResult),
    LogSeen(Result<(), String>),
    LogsSeen {
        ok: usize,
    },
    Desktop {
        ok: bool,
        success: &'static str,
        failure: &'static str,
    },
    ToastExpired(u64),
}

type DoneQueue = Arc<Mutex<VecDeque<Done>>>;

/// The item under the stream cursor, detached from the borrowed item list.
struct CursorItem {
    header: bool,
    group_id: String,
    group_shots: Vec<Shot>,
    shot: Option<Shot>,
}

/// The values `App()` derives on every render, as owned data.
struct Derived {
    state: Arc<StoreState>,
    filtered: Vec<Shot>,
    counts: StreamCounts,
    item_count: usize,
    cursor: usize,
    cursor_item: Option<CursorItem>,
    active_shot: Option<Shot>,
    friction_filtered: Vec<FrictionLog>,
    friction_pending: usize,
    friction_seen: usize,
    friction_cursor: usize,
    selected_log: Option<FrictionLog>,
    selected_run: Option<FrictionRun>,
    /// `selectedLog.runs.indexOf(selectedRun)`.
    selected_run_index: Option<usize>,
    split: bool,
    body_height: u16,
    list_width: u16,
    pane_width: u16,
    list_height: u16,
    scroll_top: usize,
    friction_top: usize,
}

impl Derived {
    fn cursor_shot(&self) -> Option<&Shot> {
        self.cursor_item
            .as_ref()
            .and_then(|item| item.shot.as_ref())
    }

    /// `nextNavigable(items, collapsed, from, direction)` over this render's items.
    fn navigable(&self, collapsed: &HashSet<String>, from: usize, direction: isize) -> usize {
        let groups = contiguous_groups(&self.filtered);
        let items = flatten_stream(&groups, collapsed);
        next_navigable(&items, collapsed, from, direction)
    }
}

/// `useInput`'s `(input, key)` pair for one crossterm key event.
struct Key {
    input: Option<char>,
    ctrl: bool,
    escape: bool,
    enter: bool,
    tab: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    page_up: bool,
    page_down: bool,
    home: bool,
    end: bool,
}

impl Key {
    fn from_event(event: &KeyEvent) -> Self {
        let code = event.code;
        Self {
            input: match code {
                KeyCode::Char(c) => Some(c),
                _ => None,
            },
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            escape: code == KeyCode::Esc,
            enter: code == KeyCode::Enter,
            tab: matches!(code, KeyCode::Tab | KeyCode::BackTab),
            left: code == KeyCode::Left,
            right: code == KeyCode::Right,
            up: code == KeyCode::Up,
            down: code == KeyCode::Down,
            page_up: code == KeyCode::PageUp,
            page_down: code == KeyCode::PageDown,
            home: code == KeyCode::Home,
            end: code == KeyCode::End,
        }
    }

    fn is(&self, c: char) -> bool {
        self.input == Some(c)
    }
}

/// Which child owns keyboard input while the composer is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposerHost {
    Takeover,
    Detail,
}

fn fg(color: Color) -> Style {
    Style::new().fg(color)
}

fn toggle(collapsed: &mut HashSet<String>, id: &str) {
    if !collapsed.remove(id) {
        collapsed.insert(id.to_string());
    }
}

/// `stepIndex={Math.min(ui.stepIndex, selectedRun.steps.length - 1)}`.
fn step_detail_props<'a>(
    step_index: isize,
    image_index: usize,
    log: &'a FrictionLog,
    run: &'a FrictionRun,
    mode: StepMode,
) -> FrictionStepDetailProps<'a> {
    FrictionStepDetailProps {
        log,
        run,
        step_index: step_index.min(run.steps.len() as isize - 1).max(0) as usize,
        image_index,
        mode,
    }
}

/// `Math.round` for the non-negative values used here.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

pub struct App {
    services: AppServices,
    actions: Arc<dyn ReviewActions>,
    desktop: Arc<dyn Desktop>,
    wake: Wake,
    now: NowFn,
    store_state: StoreStateHook,
    size: TerminalSizeHook,
    clock: ClockHook,
    pub ui: UiState,
    playback: Playback,
    stream: StreamListState,
    stream_mounted: bool,
    friction_step: FrictionStepState,
    friction_step_mounted: bool,
    detail: DetailPaneState,
    detail_mounted: bool,
    takeover: ReviewTakeoverState,
    takeover_mounted: bool,
    done: DoneQueue,
    /// `lastEventRef`.
    last_event_at: f64,
    /// `activePathRef`.
    active_path: Option<String>,
    next_toast_id: u64,
    toast_timer: Option<JoinHandle<()>>,
    on_quit: Option<Box<dyn FnMut() + Send>>,
}

impl App {
    /// `<App onQuit>` inside `<ServicesContext.Provider value={services}>`.
    /// Must be created inside a tokio runtime.
    pub fn new(services: &AppServices, props: AppProps, size: TerminalSize, wake: Wake) -> Self {
        Self::with_options(
            services,
            AppOptions {
                props,
                ..AppOptions::new(size)
            },
            wake,
        )
    }

    pub fn with_options(services: &AppServices, options: AppOptions, wake: Wake) -> Self {
        let now: NowFn = options
            .now
            .unwrap_or_else(|| Arc::new(super::theme::now_ms));
        let playback = Playback::new(initial_playback(), wake.clone());
        Self {
            services: services.clone(),
            actions: options
                .actions
                .unwrap_or_else(|| Arc::new(StoreActions(services.store.clone()))),
            desktop: options.desktop,
            now: now.clone(),
            store_state: StoreStateHook::new(services, wake.clone()),
            size: TerminalSizeHook::new(options.size),
            clock: ClockHook::with_now(Duration::from_millis(30_000), wake.clone(), now),
            ui: UiState::default(),
            stream: StreamListState::new(services, wake.clone()),
            stream_mounted: false,
            friction_step: FrictionStepState::new(services, wake.clone()),
            friction_step_mounted: false,
            detail: DetailPaneState::new(services, playback.clone(), wake.clone()),
            detail_mounted: false,
            takeover: ReviewTakeoverState::new(services, playback.clone(), wake.clone()),
            takeover_mounted: false,
            playback,
            done: Arc::default(),
            last_event_at: 0.0,
            active_path: None,
            next_toast_id: 0,
            toast_timer: None,
            on_quit: options.props.on_quit,
            wake,
        }
    }

    /// `ui.playback`.
    pub fn playback(&self) -> PlaybackState {
        self.playback.snapshot()
    }

    pub fn size(&self) -> TerminalSize {
        self.size.size()
    }

    /// The composer text of whichever child is hosting it.
    pub fn composer_value(&self) -> String {
        match self.composer_host(&self.derive()) {
            Some(ComposerHost::Takeover) => self.takeover.composer_value(),
            Some(ComposerHost::Detail) => self.detail.composer_value(),
            None => String::new(),
        }
    }

    // ---- Derived stream data --------------------------------------------------

    fn derive(&self) -> Derived {
        let ui = &self.ui;
        let state = self.store_state.state();
        let shots = &state.shots;
        let filtered: Vec<Shot> = filter_shots(shots, ui.stream_filter, ui.movies_only)
            .into_iter()
            .cloned()
            .collect();
        let counts = stream_counts(shots, ui.movies_only);
        let (item_count, cursor, cursor_item, scroll_top);
        let size = self.size.size();
        let split = size.columns >= SPLIT_BREAKPOINT;
        let body_height = size.rows.saturating_sub(4).max(6);
        let list_width = if split {
            ((f64::from(size.columns) * 0.42).floor() as u16).max(52)
        } else {
            size.columns
        };
        let pane_width = if split {
            size.columns - list_width - 1
        } else {
            size.columns
        };
        let list_height = body_height;
        {
            let groups = contiguous_groups(&filtered);
            let items = flatten_stream(&groups, &ui.collapsed);
            item_count = items.len();
            cursor = if items.is_empty() {
                0
            } else {
                next_navigable(&items, &ui.collapsed, ui.cursor.min(items.len() - 1), 1)
            };
            cursor_item = items.get(cursor).map(|item| CursorItem {
                header: matches!(item, StreamItem::Header { .. }),
                group_id: item.group().id.to_string(),
                group_shots: item.group().shots.iter().map(|s| (*s).clone()).collect(),
                shot: match item {
                    StreamItem::Shot { shot, .. } => Some((*shot).clone()),
                    StreamItem::Header { .. } => None,
                },
            });
            scroll_top = scroll_window(&items, cursor, ui.scroll_top, usize::from(list_height));
        }
        let cursor_shot = cursor_item.as_ref().and_then(|item| item.shot.clone());
        let selected_shot = ui
            .selected_shot
            .as_ref()
            .and_then(|path| shots.iter().find(|shot| &shot.path == path))
            .cloned();
        let active_shot = if ui.pane == Pane::Detail || ui.takeover == Some(Takeover::Shot) {
            selected_shot.or(cursor_shot)
        } else {
            cursor_shot.or(selected_shot)
        };

        let friction_logs = &state.friction_logs;
        let friction_filtered: Vec<FrictionLog> =
            filter_friction_logs(friction_logs, ui.friction_filter)
                .into_iter()
                .cloned()
                .collect();
        let friction_seen = friction_logs
            .iter()
            .filter(|log| friction_state(log) == ReviewState::Seen)
            .count();
        let friction_pending = friction_logs.len() - friction_seen;
        let friction_cursor = ui
            .friction_cursor
            .min(friction_filtered.len().saturating_sub(1));
        let selected_log = ui
            .selected_log
            .as_ref()
            .and_then(|id| friction_logs.iter().find(|log| &log.id == id))
            .or_else(|| friction_filtered.get(friction_cursor))
            .cloned();
        let selected_run_index = selected_log.as_ref().and_then(|log| {
            log.runs
                .iter()
                .position(|run| Some(&run.run_id) == ui.selected_run.as_ref())
                .or_else(|| {
                    let latest = latest_run(log)?;
                    log.runs.iter().position(|run| std::ptr::eq(run, latest))
                })
        });
        let selected_run = selected_log
            .as_ref()
            .zip(selected_run_index)
            .map(|(log, index)| log.runs[index].clone());
        let friction_top = friction_scroll(
            friction_filtered.len(),
            friction_cursor,
            ui.friction_scroll_top,
            usize::from(list_height),
        );

        Derived {
            state,
            filtered,
            counts,
            item_count,
            cursor,
            cursor_item,
            active_shot,
            friction_filtered,
            friction_pending,
            friction_seen,
            friction_cursor,
            selected_log,
            selected_run,
            selected_run_index,
            split,
            body_height,
            list_width,
            pane_width,
            list_height,
            scroll_top,
            friction_top,
        }
    }

    fn can_inline_play(&self) -> bool {
        matches!(
            self.services.capabilities.graphics,
            GraphicsProtocol::Kitty | GraphicsProtocol::Herdr
        )
    }

    /// The child whose `<TextInput>` is mounted, following the render branches.
    fn composer_host(&self, view: &Derived) -> Option<ComposerHost> {
        let ui = &self.ui;
        if !ui.composer || ui.help || view.active_shot.is_none() {
            return None;
        }
        if ui.takeover == Some(Takeover::Shot) {
            return Some(ComposerHost::Takeover);
        }
        if ui.takeover == Some(Takeover::Step)
            && view
                .selected_run
                .as_ref()
                .is_some_and(|run| !run.steps.is_empty())
            && view.selected_log.is_some()
        {
            return None;
        }
        if ui.pane == Pane::Settings || ui.tab != Tab::Shots {
            return None;
        }
        (view.split || ui.pane == Pane::Detail).then_some(ComposerHost::Detail)
    }

    // ---- Effects ------------------------------------------------------------------

    /// Run the component's effects until state is settled: apply finished
    /// actions, announce arrivals, keep the scroll windows around the cursors,
    /// and reset per-shot state when the active shot changes.
    pub fn sync(&mut self) {
        let finished: Vec<Done> = self
            .done
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect();
        for done in finished {
            self.apply(done);
        }

        // Overlay stand-in: announce arrivals while the tray is open.
        let state = self.store_state.state();
        if let Some(event) = &state.last_event
            && event.at != self.last_event_at
        {
            self.last_event_at = event.at;
            if event.kind == StoreEventKind::NewShot {
                let shot = &event.shot;
                self.toast_for(
                    format!(
                        "New · {} · {} · {}",
                        shot.worktree_short, shot.feature, shot.title
                    ),
                    5500,
                );
            }
        }

        if !self.ui.composer {
            // React unmounts the `<TextInput>`; its draft does not survive.
            self.detail.reset_composer();
            self.takeover.reset_composer();
        }

        let view = self.derive();
        self.ui.scroll_top = view.scroll_top;
        self.ui.friction_scroll_top = view.friction_top;

        // Reset per-shot transient state when the active shot changes.
        let current = view.active_shot.as_ref().map(|shot| shot.path.clone());
        if current != self.active_path {
            self.active_path = current;
            self.ui.inline_player = false;
            self.reset_playback(false);
            self.ui.composer = false;
            self.detail.reset_composer();
            self.takeover.reset_composer();
            self.ui.error = None;
            self.ui.zoom = 1.0;
            self.ui.pan_x = 0.5;
            self.ui.pan_y = 0.5;
        }
    }

    fn apply(&mut self, done: Done) {
        match done {
            Done::Seen { path, result } => match result {
                Ok(()) => {
                    self.toast("Seen");
                    // Only leave the surface the user was on for THIS shot; if they paged
                    // on while the write was queued, stay put.
                    let still_here = self.ui.selected_shot.as_deref() == Some(path.as_str());
                    self.ui.busy = false;
                    if still_here {
                        self.ui.composer = false;
                        if self.ui.takeover == Some(Takeover::Shot) {
                            self.ui.takeover = None;
                        }
                        if self.ui.pane == Pane::Detail {
                            self.ui.pane = Pane::Stream;
                        }
                    }
                }
                Err(message) => {
                    self.ui.busy = false;
                    self.ui.error = Some(message);
                }
            },
            Done::Feedback { path, result } => match result {
                Ok(()) => {
                    self.toast("Comment added");
                    self.ui.busy = false;
                    if self.ui.selected_shot.as_deref() == Some(path.as_str()) {
                        self.ui.composer = false;
                    }
                }
                Err(message) => {
                    self.ui.busy = false;
                    self.ui.error = Some(message);
                }
            },
            Done::SeenAll(result) => {
                self.ui.bulk_busy = false;
                if result.failed == 0 {
                    if result.ok == 1 {
                        self.toast("Marked 1 frame seen");
                    } else {
                        self.toast(format!("Marked {} frames seen", result.ok));
                    }
                } else if result.ok == 0 {
                    self.toast("Couldn’t mark frames as seen");
                } else {
                    self.toast(format!(
                        "Marked {} seen; {} failed",
                        result.ok, result.failed
                    ));
                }
            }
            Done::LogSeen(Ok(())) => self.toast("Marked 1 story seen"),
            Done::LogSeen(Err(message)) => self.toast(message),
            Done::LogsSeen { ok } => match ok {
                0 => self.toast("Couldn’t mark stories as seen"),
                1 => self.toast("Marked 1 story seen"),
                ok => self.toast(format!("Marked {ok} stories seen")),
            },
            Done::Desktop {
                ok,
                success,
                failure,
            } => {
                if !ok {
                    self.toast(failure);
                } else if !success.is_empty() {
                    self.toast(success);
                }
            }
            Done::ToastExpired(id) => {
                if self.ui.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.ui.toast = None;
                }
            }
        }
    }

    // ---- Actions ----------------------------------------------------------------

    fn toast(&mut self, message: impl Into<String>) {
        self.toast_for(message.into(), DEFAULT_TOAST_MS);
    }

    /// Toasts clear themselves like the app's 1.6 s pill; arrivals linger like the overlay.
    fn toast_for(&mut self, message: String, duration_ms: u64) {
        self.next_toast_id += 1;
        let id = self.next_toast_id;
        self.ui.toast = Some(ToastState {
            message,
            at: (self.now)(),
            duration_ms,
            id,
        });
        if let Some(timer) = self.toast_timer.take() {
            timer.abort();
        }
        let (done, wake) = (self.done.clone(), self.wake.clone());
        self.toast_timer = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(duration_ms)).await;
            done.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(Done::ToastExpired(id));
            wake();
        }));
    }

    /// Run `work` in the background and hand its result to `sync`.
    fn spawn(&self, work: impl Future<Output = Done> + Send + 'static) {
        let (done, wake) = (self.done.clone(), self.wake.clone());
        tokio::spawn(async move {
            let finished = work.await;
            done.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(finished);
            wake();
        });
    }

    fn quit(&mut self) -> Outcome {
        if let Some(on_quit) = self.on_quit.as_mut() {
            on_quit();
        }
        Outcome::Quit
    }

    fn step_detail(&mut self, view: &Derived, delta: isize) {
        let shots = &view.state.shots;
        let current = self
            .ui
            .selected_shot
            .clone()
            .or_else(|| view.cursor_shot().map(|shot| shot.path.clone()));
        let Some(index) = shots
            .iter()
            .position(|shot| Some(&shot.path) == current.as_ref())
        else {
            return;
        };
        let next = index as isize + delta;
        if next < 0 || next >= shots.len() as isize {
            return;
        }
        self.ui.selected_shot = Some(shots[next as usize].path.clone());
    }

    fn step_takeover(&mut self, view: &Derived, delta: isize) {
        let Some(active) = &view.active_shot else {
            return;
        };
        let siblings = review_siblings(&view.state.shots, active);
        let index = siblings
            .iter()
            .position(|shot| shot.path == active.path)
            .map_or(-1, |index| index as isize);
        let next = index + delta;
        if next < 0 || next >= siblings.len() as isize {
            return;
        }
        self.ui.selected_shot = Some(siblings[next as usize].path.clone());
    }

    fn mark_seen(&mut self, shot: &Shot, comment: Option<&str>) {
        let has_comment = comment.is_some_and(|comment| !comment.trim().is_empty());
        if review_state_of(shot) == ReviewState::Seen && !has_comment {
            self.toast("Already seen");
            return;
        }
        self.ui.busy = true;
        self.ui.error = None;
        let work = self
            .actions
            .mark_shot_seen(shot.clone(), comment.map(str::to_string));
        let path = shot.path.clone();
        self.spawn(async move {
            Done::Seen {
                path,
                result: work.await.map(|_| ()).map_err(|error| error.to_string()),
            }
        });
    }

    fn send_feedback(&mut self, shot: &Shot, body: &str) {
        if body.trim().is_empty() {
            self.ui.composer = false;
            return;
        }
        self.ui.busy = true;
        self.ui.error = None;
        let work = self
            .actions
            .add_shot_comment(shot.clone(), body.to_string());
        let path = shot.path.clone();
        self.spawn(async move {
            Done::Feedback {
                path,
                result: work.await.map(|_| ()).map_err(|error| error.to_string()),
            }
        });
    }

    fn seen_all(&mut self, pool: &[Shot]) {
        let targets: Vec<Shot> = pool
            .iter()
            .filter(|shot| review_state_of(shot) != ReviewState::Seen)
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        self.ui.bulk_busy = true;
        let work = self.actions.mark_many_seen(targets);
        self.spawn(async move { Done::SeenAll(work.await) });
    }

    fn mark_log_seen(&mut self, log: &FrictionLog) {
        let Some(run) = latest_run(log) else {
            return;
        };
        if friction_state(log) == ReviewState::Seen {
            self.toast("Already seen");
            return;
        }
        let work = self
            .actions
            .mark_friction_run_seen(log.clone(), run.clone());
        self.spawn(async move { Done::LogSeen(work.await.map_err(|error| error.to_string())) });
    }

    fn seen_all_logs(&mut self, logs: &[FrictionLog]) {
        let targets: Vec<(FrictionLog, FrictionRun)> = logs
            .iter()
            .filter(|log| friction_state(log) != ReviewState::Seen)
            .filter_map(|log| Some((log.clone(), latest_run(log)?.clone())))
            .collect();
        if targets.is_empty() {
            return;
        }
        let actions = self.actions.clone();
        self.spawn(async move {
            let mut ok = 0;
            for (log, run) in targets {
                // Failures are counted by omission.
                if actions.mark_friction_run_seen(log, run).await.is_ok() {
                    ok += 1;
                }
            }
            Done::LogsSeen { ok }
        });
    }

    fn desktop(
        &mut self,
        task: ActionFuture<anyhow::Result<()>>,
        success: &'static str,
        failure: &'static str,
    ) {
        self.spawn(async move {
            Done::Desktop {
                ok: task.await.is_ok(),
                success,
                failure,
            }
        });
    }

    fn reveal(&mut self, target: &str, failure: &'static str) {
        let task = self.desktop.reveal_in_file_manager(target.to_string());
        self.desktop(task, "", failure);
    }

    fn open_movie(&mut self, video_path: &str) {
        let task = self.desktop.open_with_default_app(video_path.to_string());
        self.desktop(task, "", "Couldn’t open movie");
    }

    fn copy_image(&mut self, image_path: &str) {
        let task = self.desktop.copy_image_to_clipboard(image_path.to_string());
        self.desktop(task, "Copied image", "Couldn’t copy image");
    }

    fn toggle_prompt(&mut self, view: &Derived) {
        let Some(prompt_path) = view
            .selected_log
            .as_ref()
            .and_then(|log| log.prompt_path.as_deref())
            .filter(|path| !path.is_empty())
        else {
            return;
        };
        if self.ui.prompt_open {
            self.ui.prompt_open = false;
            return;
        }
        let prompt = std::fs::read(prompt_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_else(|_| "(prompt.md could not be read)".to_string());
        self.ui.prompt_open = true;
        self.ui.prompt = Some(prompt);
    }

    /// `playback: initialPlayback()`, optionally already playing.
    fn reset_playback(&mut self, playing: bool) {
        let initial = initial_playback();
        self.playback.patch(&PlaybackPatch {
            playing: Some(playing),
            position_ms: Some(initial.position_ms),
            duration_ms: Some(initial.duration_ms),
            seek_token: Some(initial.seek_token),
            error: Some(initial.error),
            ended: Some(initial.ended),
        });
    }

    fn seek_by(&mut self, view: &Derived, delta_ms: f64) {
        let previous = self.playback.snapshot();
        let duration = previous
            .duration_ms
            .or(view.active_shot.as_ref().and_then(|shot| shot.duration_ms))
            .filter(|duration| *duration != 0.0 && !duration.is_nan());
        let moved = previous.position_ms + delta_ms;
        let target = duration
            .map_or(moved, |duration| duration.min(moved))
            .max(0.0);
        self.playback.patch(&PlaybackPatch {
            position_ms: Some(target),
            seek_token: Some(previous.seek_token + 1),
            ended: Some(false),
            ..PlaybackPatch::default()
        });
    }

    fn seek_chapter(&mut self, view: &Derived, direction: isize) {
        let Some(active) = &view.active_shot else {
            return;
        };
        let mut marks: Vec<f64> = active
            .chapters
            .iter()
            .filter_map(|chapter| chapter.t_ms)
            .collect();
        marks.sort_by(f64::total_cmp);
        let Some(last) = marks.last().copied() else {
            return;
        };
        let previous = self.playback.snapshot();
        let position = previous.position_ms;
        let target = if direction > 0 {
            marks.iter().copied().find(|mark| *mark > position + 200.0)
        } else {
            marks
                .iter()
                .rev()
                .copied()
                .find(|mark| *mark < position - 200.0)
        };
        let resolved = target.unwrap_or(if direction > 0 { last } else { 0.0 });
        self.playback.patch(&PlaybackPatch {
            position_ms: Some(resolved),
            seek_token: Some(previous.seek_token + 1),
            ended: Some(false),
            ..PlaybackPatch::default()
        });
    }

    fn zoom_by(&mut self, delta: f64) {
        let zoom = (js_round((self.ui.zoom + delta) * 100.0) / 100.0).clamp(1.0, 6.0);
        self.ui.zoom = zoom;
        // Snapping back to 1 recenters so the next zoom-in starts from the middle.
        if zoom == 1.0 {
            self.ui.pan_x = 0.5;
            self.ui.pan_y = 0.5;
        }
    }

    fn pan_by(&mut self, dx: f64, dy: f64) {
        if self.ui.zoom <= 1.0 {
            return;
        }
        let step = 0.18 / self.ui.zoom;
        self.ui.pan_x = (self.ui.pan_x + dx * step).clamp(0.0, 1.0);
        self.ui.pan_y = (self.ui.pan_y + dy * step).clamp(0.0, 1.0);
    }

    fn reset_zoom(&mut self) {
        self.ui.zoom = 1.0;
        self.ui.pan_x = 0.5;
        self.ui.pan_y = 0.5;
    }

    /// The toast shown instead of playing, when the shot cannot play in the tray.
    fn playback_blocker(&self, shot: Option<&Shot>) -> Option<&'static str> {
        if shot.and_then(|shot| shot.video_path.as_deref()).is_none() {
            return Some(if shot.is_some_and(|shot| shot.is_movie) {
                "Video missing on disk"
            } else {
                "Not a movie"
            });
        }
        (!self.can_inline_play()).then_some(KITTY_PLAYBACK_HINT)
    }

    fn toggle_play(&mut self, view: &Derived) {
        if let Some(blocker) = self.playback_blocker(view.active_shot.as_ref()) {
            self.toast(blocker);
            return;
        }
        self.ui.inline_player = true;
        let previous = self.playback.snapshot();
        if previous.ended {
            self.playback.patch(&PlaybackPatch {
                playing: Some(true),
                position_ms: Some(0.0),
                seek_token: Some(previous.seek_token + 1),
                ended: Some(false),
                error: Some(None),
                ..PlaybackPatch::default()
            });
        } else {
            self.playback.patch(&PlaybackPatch {
                playing: Some(!previous.playing),
                error: Some(None),
                ..PlaybackPatch::default()
            });
        }
    }

    // ---- Keymap -------------------------------------------------------------------

    /// Handle one terminal event. Keys follow `useInput` in `app.tsx`; while
    /// the composer is open they go to the child that hosts it.
    pub fn handle(&mut self, event: Event) -> Outcome {
        // React has rendered (and run its effects) before a key can arrive.
        self.sync();
        let outcome = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
            Event::Paste(text) => {
                self.handle_paste(&text);
                Outcome::Continue
            }
            Event::Resize(columns, rows) => {
                if self.size.on_resize(columns, rows) {
                    self.services.layer.invalidate();
                }
                Outcome::Continue
            }
            _ => Outcome::Continue,
        };
        self.sync();
        outcome
    }

    fn handle_paste(&mut self, text: &str) {
        if !self.ui.composer {
            return;
        }
        let view = self.derive();
        let outcome = match self.composer_host(&view) {
            Some(ComposerHost::Takeover) => self.takeover.handle_paste(text),
            Some(ComposerHost::Detail) => self.detail.handle_paste(text),
            None => return,
        };
        self.composer_outcome(&view, outcome);
    }

    fn composer_outcome(&mut self, view: &Derived, outcome: text_input::Outcome) {
        match outcome {
            text_input::Outcome::Pending => {}
            text_input::Outcome::Cancel => self.ui.composer = false,
            text_input::Outcome::Submit(text) => {
                if let Some(shot) = &view.active_shot {
                    self.send_feedback(shot, &text);
                }
            }
        }
    }

    fn handle_key(&mut self, event: KeyEvent) -> Outcome {
        let key = Key::from_event(&event);
        // Ink's `exitOnCtrlC` ends the app before any `useInput` handler runs,
        // composer included.
        if key.ctrl && key.is('c') {
            return self.quit();
        }
        let view = self.derive();
        if self.ui.composer {
            let outcome = match self.composer_host(&view) {
                Some(ComposerHost::Takeover) => self.takeover.handle(event),
                Some(ComposerHost::Detail) => self.detail.handle(event),
                None => return Outcome::Continue,
            };
            self.composer_outcome(&view, outcome);
            return Outcome::Continue;
        }

        if self.ui.help {
            if key.is('?') || key.escape || key.is('q') {
                self.ui.help = false;
            }
            return Outcome::Continue;
        }
        if key.is('?') {
            self.ui.help = true;
            return Outcome::Continue;
        }

        // Full-screen takeovers.
        if self.ui.takeover == Some(Takeover::Shot)
            && let Some(active) = view.active_shot.clone()
        {
            self.takeover_shot_key(&view, &key, &active);
            return Outcome::Continue;
        }
        if self.ui.takeover == Some(Takeover::Step)
            && let Some(run) = &view.selected_run
        {
            self.takeover_step_key(&key, run);
            return Outcome::Continue;
        }

        // Global keys.
        if key.is('q') {
            return self.quit();
        }
        let active = view.active_shot.clone();
        let active_video = active.as_ref().and_then(|shot| shot.video_path.clone());
        // Detail-pane seek keys outrank the global settings toggle while a movie is loaded.
        if self.ui.tab == Tab::Shots
            && self.ui.pane == Pane::Detail
            && self.ui.inline_player
            && active_video.is_some()
        {
            if key.is(',') {
                self.seek_by(&view, -SEEK_STEP_MS);
                return Outcome::Continue;
            }
            if key.is('.') {
                self.seek_by(&view, SEEK_STEP_MS);
                return Outcome::Continue;
            }
        }
        if key.is('1') {
            self.ui.tab = Tab::Shots;
            self.ui.pane = Pane::Stream;
            self.ui.takeover = None;
        } else if key.is('2') {
            self.ui.tab = Tab::FrictionLogs;
            self.ui.pane = Pane::Stream;
            self.ui.takeover = None;
        } else if key.tab {
            self.ui.tab = if self.ui.tab == Tab::Shots {
                Tab::FrictionLogs
            } else {
                Tab::Shots
            };
            self.ui.pane = Pane::Stream;
        } else if key.is(',') {
            self.ui.pane = if self.ui.pane == Pane::Settings {
                Pane::Stream
            } else {
                Pane::Settings
            };
        } else if key.is('r') {
            self.toast("Scanning…");
            // `void store.rescan({ force: true })`.
            tokio::spawn(self.actions.rescan(RescanOptions { force: true }));
        } else if self.ui.pane == Pane::Settings {
            if key.escape || key.is('h') || key.left {
                self.ui.pane = Pane::Stream;
            }
        } else if self.ui.tab == Tab::Shots {
            match active {
                Some(active) if self.ui.pane == Pane::Detail => {
                    self.detail_key(&view, &key, &active);
                }
                _ => self.stream_key(&view, &key),
            }
        } else {
            self.friction_key(&view, &key);
        }
        Outcome::Continue
    }

    fn takeover_shot_key(&mut self, view: &Derived, key: &Key, active: &Shot) {
        if key.escape || key.is('q') {
            self.ui.takeover = None;
            self.ui.composer = false;
            return;
        }
        // When zoomed in, the arrows pan the image; otherwise they page.
        let zoomed = self.ui.zoom > 1.0;
        if key.left || key.is('h') {
            if zoomed {
                self.pan_by(-1.0, 0.0);
            } else {
                self.step_takeover(view, -1);
            }
        } else if key.right || key.is('l') {
            if zoomed {
                self.pan_by(1.0, 0.0);
            } else {
                self.step_takeover(view, 1);
            }
        } else if key.up || key.is('k') {
            if zoomed {
                self.pan_by(0.0, -1.0);
            }
        } else if key.down || key.is('j') {
            if zoomed {
                self.pan_by(0.0, 1.0);
            }
        } else if key.is('c') {
            self.ui.composer = true;
            self.ui.error = None;
        } else if key.is('s') {
            self.mark_seen(active, None);
        } else if key.is(' ') {
            self.toggle_play(view);
        } else if key.is(',') {
            self.seek_by(view, -SEEK_STEP_MS);
        } else if key.is('.') {
            self.seek_by(view, SEEK_STEP_MS);
        } else if key.is('[') {
            self.seek_chapter(view, -1);
        } else if key.is(']') {
            self.seek_chapter(view, 1);
        } else if key.is('+') || key.is('=') {
            self.zoom_by(0.5);
        } else if key.is('-') || key.is('_') {
            self.zoom_by(-0.5);
        } else if key.is('0') {
            self.reset_zoom();
        } else if key.is('y') {
            self.copy_image(&active.path);
        } else if key.is('o') {
            self.reveal(&active.path, "Couldn’t reveal file");
        } else if key.is('O')
            && let Some(video_path) = &active.video_path
        {
            self.open_movie(video_path);
        }
    }

    fn takeover_step_key(&mut self, key: &Key, run: &FrictionRun) {
        let last_step = run.steps.len() as isize - 1;
        if key.escape || key.is('q') {
            self.ui.takeover = None;
        } else if key.left || key.is('h') {
            self.ui.step_index = (self.ui.step_index - 1).max(0);
            self.ui.image_index = 0;
        } else if key.right || key.is('l') {
            self.ui.step_index = last_step.min(self.ui.step_index + 1);
            self.ui.image_index = 0;
        } else if key.is('[') {
            self.ui.image_index = self.ui.image_index.saturating_sub(1);
        } else if key.is(']') {
            self.next_image(run);
        }
    }

    /// `imageIndex: Math.min(Math.max(0, count - 1), state.imageIndex + 1)`.
    fn next_image(&mut self, run: &FrictionRun) {
        let count = usize::try_from(self.ui.step_index)
            .ok()
            .and_then(|index| run.steps.get(index))
            .map_or(0, |step| step.screenshots.len());
        self.ui.image_index = count.saturating_sub(1).min(self.ui.image_index + 1);
    }

    fn detail_key(&mut self, view: &Derived, key: &Key, active: &Shot) {
        if key.escape || (key.is('h') && !view.split) {
            self.ui.pane = Pane::Stream;
            self.ui.composer = false;
        } else if key.left {
            self.step_detail(view, 1);
        } else if key.right {
            self.step_detail(view, -1);
        } else if key.enter || key.is('f') {
            self.ui.takeover = Some(Takeover::Shot);
        } else if key.is('c') {
            self.ui.composer = true;
            self.ui.error = None;
        } else if key.is('s') {
            self.mark_seen(active, None);
        } else if key.is('p') {
            if let Some(blocker) = self.playback_blocker(Some(active)) {
                self.toast(blocker);
                return;
            }
            let showing = self.ui.inline_player;
            self.ui.inline_player = !showing;
            self.playback.patch(&PlaybackPatch {
                playing: Some(!showing),
                error: Some(None),
                ..PlaybackPatch::default()
            });
        } else if key.is(' ') {
            self.toggle_play(view);
        } else if key.is(',') {
            self.seek_by(view, -SEEK_STEP_MS);
        } else if key.is('.') {
            self.seek_by(view, SEEK_STEP_MS);
        } else if key.is('[') {
            self.seek_chapter(view, -1);
        } else if key.is(']') {
            self.seek_chapter(view, 1);
        } else if key.is('+') || key.is('=') {
            self.zoom_by(0.5);
        } else if key.is('-') || key.is('_') {
            self.zoom_by(-0.5);
        } else if key.is('0') {
            self.reset_zoom();
        } else if key.is('o') {
            self.reveal(&active.path, "Couldn’t reveal file");
        } else if key.is('O') {
            if let Some(video_path) = &active.video_path {
                self.open_movie(video_path);
            }
        } else if key.is('y') {
            self.copy_image(&active.path);
        }
    }

    fn stream_key(&mut self, view: &Derived, key: &Key) {
        self.actions.mark_opened();
        let last = view.item_count as isize - 1;
        let page = (view.list_height / 4).max(1) as isize;
        let delta = if key.down || key.is('j') {
            Some(1)
        } else if key.up || key.is('k') {
            Some(-1)
        } else if key.page_down {
            Some(page)
        } else if key.page_up {
            Some(-page)
        } else {
            None
        };
        if let Some(delta) = delta {
            let target = (view.cursor as isize + delta).min(last).max(0) as usize;
            self.ui.cursor =
                view.navigable(&self.ui.collapsed, target, if delta >= 0 { 1 } else { -1 });
            return;
        }
        if key.is('g') || key.home {
            self.ui.cursor = view.navigable(&self.ui.collapsed, 0, 1);
            return;
        }
        if key.is('G') || key.end {
            self.ui.cursor = view.navigable(&self.ui.collapsed, last.max(0) as usize, -1);
            return;
        }
        if key.is('u') {
            self.ui.stream_filter = if self.ui.stream_filter == StreamFilter::Unseen {
                StreamFilter::History
            } else {
                StreamFilter::Unseen
            };
            self.ui.cursor = 0;
            return;
        }
        if key.is('m') {
            self.ui.movies_only = !self.ui.movies_only;
            self.ui.cursor = 0;
            return;
        }
        let Some(item) = &view.cursor_item else {
            if key.is('S') {
                self.seen_all(&view.filtered);
            }
            return;
        };
        if key.is('z') {
            toggle(&mut self.ui.collapsed, &item.group_id);
            return;
        }
        if item.header {
            if key.enter || key.left || key.right || key.is(' ') {
                toggle(&mut self.ui.collapsed, &item.group_id);
            } else if key.is('S') {
                self.seen_all(&item.group_shots);
            }
            return;
        }
        if key.is('S') {
            self.seen_all(&view.filtered);
            return;
        }
        if key.is('A') {
            self.seen_all(&item.group_shots);
            return;
        }
        let Some(shot) = &item.shot else {
            return;
        };
        let select = |ui: &mut UiState| ui.selected_shot = Some(shot.path.clone());
        if key.enter {
            select(&mut self.ui);
            if view.split {
                self.ui.takeover = Some(Takeover::Shot);
            } else {
                self.ui.pane = Pane::Detail;
            }
        } else if key.is('f') || key.is(' ') {
            select(&mut self.ui);
            self.ui.takeover = Some(Takeover::Shot);
        } else if key.is('s') {
            self.mark_seen(shot, None);
        } else if key.is('c') {
            select(&mut self.ui);
            self.ui.composer = true;
            self.ui.error = None;
            if !view.split {
                self.ui.pane = Pane::Detail;
            }
        } else if key.is('p') {
            if shot.video_path.is_none() {
                self.toast(if shot.is_movie {
                    "Video missing on disk"
                } else {
                    "Not a movie"
                });
                return;
            }
            select(&mut self.ui);
            if !view.split {
                self.ui.pane = Pane::Detail;
            }
            if !self.can_inline_play() {
                self.toast(KITTY_PLAYBACK_HINT);
                return;
            }
            self.ui.inline_player = true;
            self.reset_playback(true);
        } else if key.is('o') {
            self.reveal(&shot.path, "Couldn’t reveal file");
        } else if key.is('O') {
            if let Some(video_path) = &shot.video_path {
                self.open_movie(video_path);
            }
        } else if key.is('y') {
            self.copy_image(&shot.path);
        }
    }

    fn friction_key(&mut self, view: &Derived, key: &Key) {
        let log = view.selected_log.as_ref();
        let run = view.selected_run.as_ref();
        if self.ui.pane == Pane::FrictionStep
            && let (Some(_), Some(run)) = (log, run)
        {
            let last_step = run.steps.len() as isize - 1;
            if key.escape || key.is('h') {
                self.ui.pane = Pane::FrictionLog;
            } else if key.left {
                let previous = (self.ui.step_index - 1).max(0);
                self.ui.step_index = previous;
                self.ui.image_index = 0;
                self.ui.step_cursor = previous;
            } else if key.right {
                let next = last_step.min(self.ui.step_index + 1);
                self.ui.step_index = next;
                self.ui.image_index = 0;
                self.ui.step_cursor = next;
            } else if key.is('[') {
                self.ui.image_index = self.ui.image_index.saturating_sub(1);
            } else if key.is(']') {
                self.next_image(run);
            } else if key.enter || key.is('f') || key.is(' ') {
                self.ui.takeover = Some(Takeover::Step);
            } else {
                let shot_path = usize::try_from(self.ui.step_index)
                    .ok()
                    .and_then(|index| run.steps.get(index))
                    .and_then(|step| step.screenshots.get(self.ui.image_index))
                    .filter(|path| !path.is_empty())
                    .cloned();
                if let Some(shot_path) = shot_path {
                    if key.is('o') {
                        self.reveal(&shot_path, "Couldn’t reveal file");
                    } else if key.is('y') {
                        self.copy_image(&shot_path);
                    }
                }
            }
            return;
        }
        if self.ui.pane == Pane::FrictionLog
            && let Some(log) = log
        {
            let step_count = run.map_or(0, |run| run.steps.len()) as isize;
            if key.escape || key.is('h') {
                self.ui.pane = Pane::Stream;
                self.ui.prompt_open = false;
            } else if key.down || key.is('j') {
                self.ui.step_cursor = (step_count - 1).max(0).min(self.ui.step_cursor + 1);
            } else if key.up || key.is('k') {
                self.ui.step_cursor = (self.ui.step_cursor - 1).max(0);
            } else if (key.enter || key.right) && step_count > 0 {
                self.ui.pane = Pane::FrictionStep;
                self.ui.step_index = self.ui.step_cursor;
                self.ui.image_index = 0;
            } else if key.is('p') {
                self.toggle_prompt(view);
            } else if key.is('s') {
                self.mark_log_seen(log);
            } else if (key.is('[') || key.is(']'))
                && log.runs.len() > 1
                && let Some(index) = view.selected_run_index
            {
                let next = if key.is(']') {
                    (log.runs.len() - 1).min(index + 1)
                } else {
                    index.saturating_sub(1)
                };
                self.ui.selected_run = Some(log.runs[next].run_id.clone());
                self.ui.step_cursor = 0;
            }
            return;
        }
        // Friction list.
        let last = view.friction_filtered.len().saturating_sub(1);
        if key.down || key.is('j') {
            self.ui.friction_cursor = last.min(view.friction_cursor + 1);
        } else if key.up || key.is('k') {
            self.ui.friction_cursor = view.friction_cursor.saturating_sub(1);
        } else if key.is('g') || key.home {
            self.ui.friction_cursor = 0;
        } else if key.is('G') || key.end {
            self.ui.friction_cursor = last;
        } else if key.is('u') {
            self.ui.friction_filter = if self.ui.friction_filter == StreamFilter::Unseen {
                StreamFilter::History
            } else {
                StreamFilter::Unseen
            };
            self.ui.friction_cursor = 0;
        } else if key.is('S') {
            self.seen_all_logs(&view.friction_filtered);
        } else if let Some(log) = view.friction_filtered.get(view.friction_cursor) {
            if key.enter || key.right || key.is('l') {
                self.ui.selected_log = Some(log.id.clone());
                self.ui.selected_run = latest_run(log).map(|run| run.run_id.clone());
                self.ui.pane = Pane::FrictionLog;
                self.ui.step_cursor = 0;
                self.ui.prompt_open = false;
            } else if key.is('s') {
                self.mark_log_seen(log);
            } else if key.is('o') {
                self.reveal(&log.directory, "Couldn’t reveal folder");
            }
        }
    }

    // ---- Render ---------------------------------------------------------------------

    fn hints(&self, view: &Derived) -> Vec<KeyHint> {
        let ui = &self.ui;
        let hint = KeyHint::new;
        let other = |filter: StreamFilter| match filter {
            StreamFilter::Unseen => "history",
            StreamFilter::History => "unseen",
        };
        match ui.takeover {
            Some(Takeover::Shot) => {
                let mut hints = vec![
                    hint("esc", "close"),
                    hint("← →", "older · newer"),
                    hint("c", "feedback"),
                    hint("s", "seen"),
                ];
                if view.active_shot.as_ref().is_some_and(|shot| shot.is_movie) {
                    hints.extend([
                        hint("space", "play"),
                        hint(", .", "seek"),
                        hint("[ ]", "chapter"),
                    ]);
                }
                hints.extend([hint("y", "copy"), hint("o", "reveal")]);
                hints
            }
            Some(Takeover::Step) => vec![
                hint("esc", "close"),
                hint("← →", "steps"),
                hint("[ ]", "images"),
            ],
            None if ui.pane == Pane::Settings => vec![
                hint("esc", "back"),
                hint("r", "rescan"),
                hint("?", "help"),
                hint("q", "quit"),
            ],
            None if ui.tab == Tab::Shots => {
                if ui.pane == Pane::Detail {
                    vec![
                        hint("esc", "back"),
                        hint("← →", "older · newer"),
                        hint("f", "full screen"),
                        hint("s", "seen"),
                        hint("c", "feedback"),
                        hint("p", "play"),
                        hint("?", "help"),
                    ]
                } else {
                    vec![
                        hint("↑↓", "move"),
                        hint("⏎", if view.split { "review" } else { "detail" }),
                        hint("f", "full screen"),
                        hint("s", "seen"),
                        hint("c", "feedback"),
                        hint("u", other(ui.stream_filter)),
                        hint("m", "movies"),
                        hint("?", "help"),
                        hint("q", "quit"),
                    ]
                }
            }
            None => match ui.pane {
                Pane::FrictionStep => vec![
                    hint("esc", "back"),
                    hint("← →", "steps"),
                    hint("[ ]", "images"),
                    hint("f", "full screen"),
                ],
                Pane::FrictionLog => vec![
                    hint("esc", "back"),
                    hint("↑↓", "steps"),
                    hint("⏎", "open step"),
                    hint("[ ]", "runs"),
                    hint("p", "prompt"),
                    hint("s", "seen"),
                ],
                _ => vec![
                    hint("↑↓", "move"),
                    hint("⏎", "open"),
                    hint("s", "seen"),
                    hint("u", other(ui.friction_filter)),
                    hint("?", "help"),
                    hint("q", "quit"),
                ],
            },
        }
    }

    /// Draw the tray into `frame`.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        self.render_into(area, frame.buffer_mut());
    }

    /// Draw the tray into `buf`. The layout follows the reported terminal
    /// size; rows that fall outside `area` are clipped.
    pub fn render_into(&mut self, area: Rect, buf: &mut Buffer) {
        self.sync();
        let view = self.derive();
        let size = self.size.size();
        let columns = size.columns;
        let body_height = view.body_height;
        // A rect in terminal coordinates, clipped to what can be drawn.
        let rect = |x: u16, y: u16, width: u16, height: u16| {
            Rect::new(area.x + x, area.y + y, width, height).intersection(area)
        };
        let state = &view.state;
        let hints = self.hints(&view);
        let now = self.clock.now();
        let Self {
            services,
            ui,
            playback,
            stream,
            stream_mounted,
            friction_step,
            friction_step_mounted,
            detail,
            detail_mounted,
            takeover,
            takeover_mounted,
            wake,
            ..
        } = self;
        let services: &AppServices = services;
        let (mut used_stream, mut used_step, mut used_detail, mut used_takeover) =
            (false, false, false, false);

        // ---- Header ----
        let status: (&str, Color) = if state.roots.is_empty() {
            ("Choose watch folders", THEME.amber)
        } else if state.scanning {
            (
                if state.phase == ScanPhase::Full {
                    "Scanning for captures (deep)"
                } else {
                    "Scanning for captures"
                },
                THEME.amber,
            )
        } else if state.shots.is_empty() && state.friction_logs.is_empty() {
            ("Waiting for captures", THEME.muted)
        } else {
            ("Live review stream", THEME.green)
        };
        let mut left = vec![
            Span::styled("● Astroshots", fg(THEME.brand).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", status.0), fg(status.1)),
        ];
        if state.unread_count > 0 {
            left.push(Span::styled(
                format!("  {} new", state.unread_count),
                fg(THEME.amber),
            ));
        }
        if let Some(error) = state.error.as_deref().filter(|error| !error.is_empty()) {
            left.push(Span::styled(
                format!("  {}", truncate(error, 40)),
                fg(THEME.red),
            ));
        }
        let right = Span::styled(
            format!(
                "{} trees · {} {}{}",
                state.tree_count,
                state.roots.len(),
                if state.roots.len() == 1 {
                    "root"
                } else {
                    "roots"
                },
                if state.watching {
                    ""
                } else {
                    " · not watching"
                }
            ),
            fg(THEME.muted),
        );
        let padded = |y: u16| rect(1, y, columns.saturating_sub(2), 1);
        let header = padded(0);
        if !header.is_empty() {
            space_between(buf, header, header.y, &left, &[right]);
        }

        // ---- Tabs ----
        let show_tabs = ui.takeover.is_none() && !ui.help && ui.pane != Pane::Settings;
        let tabs_row = padded(1);
        if !tabs_row.is_empty() {
            let spans: Vec<Span<'static>> = if show_tabs {
                let tab = |label: &'static str, active: bool| {
                    let mut style = fg(if active { THEME.text } else { THEME.muted });
                    if active {
                        style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
                    }
                    Span::styled(label, style)
                };
                let count = |count: usize| {
                    Span::styled(
                        format!(" {count}"),
                        fg(THEME.amber).add_modifier(Modifier::BOLD),
                    )
                };
                let mut spans = vec![tab(" 1 Shots ", ui.tab == Tab::Shots)];
                if view.counts.pending > 0 {
                    spans.push(count(view.counts.pending));
                }
                spans.push(Span::raw("   "));
                spans.push(tab(" 2 User stories ", ui.tab == Tab::FrictionLogs));
                if view.friction_pending > 0 {
                    spans.push(count(view.friction_pending));
                }
                spans
            } else {
                let label = if ui.help {
                    "Help"
                } else if ui.pane == Pane::Settings {
                    "Settings"
                } else if ui.takeover == Some(Takeover::Shot) {
                    "Full-screen review"
                } else {
                    "Story step review"
                };
                vec![Span::styled(label, fg(THEME.muted))]
            };
            put_spans(buf, tabs_row, tabs_row.x, tabs_row.y, &spans);
        }

        // ---- Body ----
        let body = rect(0, 2, columns, body_height + 1);
        let below = rect(0, 3, columns, body_height);
        let step_index = ui.step_index;
        let image_index = ui.image_index;
        let step_props =
            move |log, run, mode| step_detail_props(step_index, image_index, log, run, mode);
        // The run as an element of `selected_log.runs`: the friction views
        // find its index by identity, as `log.runs.indexOf(run)` does.
        let selected_run: Option<&FrictionRun> = view
            .selected_log
            .as_ref()
            .zip(view.selected_run_index)
            .map(|(log, index)| &log.runs[index]);
        let has_steps = selected_run.is_some_and(|run| !run.steps.is_empty());
        if ui.help {
            HelpOverlay.render(body, buf);
        } else if let (Some(Takeover::Shot), Some(active)) = (ui.takeover, &view.active_shot) {
            let siblings = review_siblings(&state.shots, active);
            let position = DetailPosition {
                index: siblings
                    .iter()
                    .position(|shot| shot.path == active.path)
                    .map_or(0, |index| index + 1),
                count: siblings.len(),
            };
            used_takeover = true;
            ReviewTakeover::new(ReviewTakeoverProps {
                composer: ui.composer,
                playing: ui.inline_player,
                busy: ui.busy,
                error: ui.error.as_deref(),
                zoom: ui.zoom,
                pan_x: ui.pan_x,
                pan_y: ui.pan_y,
                ..ReviewTakeoverProps::new(active, position)
            })
            .render(body, buf, takeover);
        } else if let (Some(Takeover::Step), Some(log), Some(run), true) =
            (ui.takeover, &view.selected_log, selected_run, has_steps)
        {
            used_step = true;
            FrictionStepTakeover::new(step_props(log, run, StepMode::Takeover)).render(
                body,
                buf,
                friction_step,
            );
        } else if ui.pane == Pane::Settings {
            SettingsPane::new(&state.roots, services).render(body, buf);
        } else if ui.tab == Tab::Shots {
            let page = !view.split && ui.pane == Pane::Detail;
            let pane_area = if page {
                below
            } else {
                rect(view.list_width + 1, 3, view.pane_width, body_height)
            };
            if page || view.split {
                match &view.active_shot {
                    Some(active) => {
                        let position = DetailPosition {
                            index: state
                                .shots
                                .iter()
                                .position(|shot| shot.path == active.path)
                                .map_or(0, |index| index + 1),
                            count: state.shots.len(),
                        };
                        used_detail = true;
                        DetailPane::new(DetailPaneProps {
                            mode: if view.split {
                                DetailMode::Pane
                            } else {
                                DetailMode::Page
                            },
                            composer: ui.composer,
                            inline_player: ui.inline_player,
                            busy: ui.busy,
                            error: ui.error.as_deref(),
                            zoom: ui.zoom,
                            pan_x: ui.pan_x,
                            pan_y: ui.pan_y,
                            ..DetailPaneProps::new(active, position)
                        })
                        .render(pane_area, buf, detail);
                    }
                    None => {
                        const PLACEHOLDER: &str = "Select a frame to review it here";
                        let width = PLACEHOLDER.chars().count() as u16;
                        let x = pane_area.x + view.pane_width.saturating_sub(width) / 2;
                        let y = pane_area.y + body_height.saturating_sub(1) / 2;
                        put_spans(
                            buf,
                            pane_area,
                            x,
                            y,
                            &[Span::styled(PLACEHOLDER, fg(THEME.muted))],
                        );
                    }
                }
            }
            if !page {
                FilterBar {
                    filter: ui.stream_filter,
                    movies_only: ui.movies_only,
                    counts: view.counts,
                    bulk_busy: ui.bulk_busy,
                }
                .render(rect(0, 2, view.list_width, 1), buf);
                let groups = contiguous_groups(&view.filtered);
                let items = flatten_stream(&groups, &ui.collapsed);
                used_stream = true;
                StreamList::new(StreamProps {
                    items: &items,
                    collapsed: &ui.collapsed,
                    cursor: view.cursor,
                    scroll_top: view.scroll_top,
                    filter: ui.stream_filter,
                    movies_only: ui.movies_only,
                    counts: view.counts,
                    focused: true,
                    scanning: state.scanning,
                    has_roots: !state.roots.is_empty(),
                    total_shots: state.shots.len(),
                    bulk_busy: ui.bulk_busy,
                })
                .render(
                    rect(0, 3, view.list_width, view.list_height),
                    buf,
                    stream,
                );
                if view.split {
                    let separator = rect(view.list_width, 3, 1, body_height);
                    for y in separator.top()..separator.bottom() {
                        put_spans(
                            buf,
                            separator,
                            separator.x,
                            y,
                            &[Span::styled("│", fg(THEME.faint))],
                        );
                    }
                }
            }
        } else if let (Pane::FrictionStep, Some(log), Some(run), true) =
            (ui.pane, &view.selected_log, selected_run, has_steps)
        {
            used_step = true;
            FrictionStepDetail::new(step_props(log, run, StepMode::Page)).render(
                below,
                buf,
                friction_step,
            );
        } else if let (Pane::FrictionLog, Some(log)) = (ui.pane, &view.selected_log) {
            FrictionLogDetail::new(FrictionLogDetailProps {
                log,
                run: selected_run,
                step_cursor: ui.step_cursor.max(0) as usize,
                prompt_open: ui.prompt_open,
                prompt: ui.prompt.as_deref(),
            })
            .render(below, buf);
        } else {
            FrictionFilterBar {
                filter: ui.friction_filter,
                pending: view.friction_pending,
                seen: view.friction_seen,
            }
            .render(rect(0, 2, columns, 1), buf);
            let logs: Vec<&FrictionLog> = view.friction_filtered.iter().collect();
            FrictionList::new(FrictionListProps {
                logs: &logs,
                cursor: view.friction_cursor,
                scroll_top: view.friction_top,
                filter: ui.friction_filter,
                pending: view.friction_pending,
                seen: view.friction_seen,
                focused: true,
                total: state.friction_logs.len(),
                now,
            })
            .render(rect(0, 3, columns, view.list_height), buf);
        }

        // ---- Toast or hints ----
        let footer = rect(0, body_height + 3, columns, 1);
        if !footer.is_empty() {
            match &ui.toast {
                Some(toast) => Toast {
                    message: Some(&toast.message),
                }
                .render(footer, buf),
                None => HintBar { hints: &hints }.render(footer, buf),
            }
        }

        // Children that were not part of this frame are unmounted: a fresh
        // state drops the pictures the old one registered.
        if !used_stream && *stream_mounted {
            *stream = StreamListState::new(services, wake.clone());
        }
        if !used_step && *friction_step_mounted {
            *friction_step = FrictionStepState::new(services, wake.clone());
        }
        if !used_detail && *detail_mounted {
            *detail = DetailPaneState::new(services, playback.clone(), wake.clone());
        }
        if !used_takeover && *takeover_mounted {
            *takeover = ReviewTakeoverState::new(services, playback.clone(), wake.clone());
        }
        *stream_mounted = used_stream;
        *friction_step_mounted = used_step;
        *detail_mounted = used_detail;
        *takeover_mounted = used_takeover;
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(timer) = self.toast_timer.take() {
            timer.abort();
        }
    }
}

/// `frictionLogDirectory(log)`.
pub fn friction_log_directory(log: &FrictionLog) -> String {
    dirname(&log.directory)
}

#[cfg(test)]
mod tests;
