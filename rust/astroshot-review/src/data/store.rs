//! Port of `packages/astroshot-review/src/data/store.ts`.
//!
//! The tray's model: every shot and friction log under the roots, newest
//! arrival first, kept fresh by the filesystem watcher, with the review
//! actions the UI exposes. Mutations run one at a time so a rescan and a
//! live event can never interleave.
//!
//! Divergences from the TS class:
//! - `ReviewStore` is shared across threads, so it lives in an `Arc`. The
//!   published snapshot (`state` + listeners) and the scan data (trees,
//!   arrival order, index) sit behind two short-lived `std` mutexes that are
//!   never held across an `.await`; the TS promise queue is a fair
//!   `tokio::sync::Mutex` held for the length of one mutation.
//! - Watcher events arrive on the watcher's timer thread. They are forwarded
//!   through a channel to one task that calls `handle_event` in arrival
//!   order (TS: `void this.handleEvent(event)` in the callback).
//! - `subscribe` returns a [`Subscription`] guard instead of a closure.
//! - `publish` takes a closure (`publish_with`) instead of a partial object.
//! - Logged error text after the `scan <dir>: ` / `event <kind>: ` /
//!   `index save: ` prefixes is the Rust error's, not Node's.
//! - The trees map keeps JS `Map` insertion order with a small `Vec`-backed
//!   map (`Trees`); no `indexmap` dependency.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use anyhow::anyhow;
use chrono::{DateTime, SecondsFormat, Utc};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::data::index_cache::{
    ArrivalShot, IndexDocument, default_index_cache_path, load_index, reconcile_arrival_order,
    save_index,
};
use crate::data::watcher::{RootWatcher, WatchEvent, WatchOptions, watch_roots};
use astroshot_engine::movie_harness::types::Version1;
use astroshot_engine::review_data::friction::{FrictionContext, LOG_FILE, load_user_stories};
use astroshot_engine::review_data::hash_cache::HashCache;
use astroshot_engine::review_data::model::{AstroshotTree, FrictionLog, FrictionRun, Shot};
use astroshot_engine::review_data::paths::{MAX_SCAN_DEPTH, basename, dirname, join};
use astroshot_engine::review_data::review_store::{
    AddCommentOptions, MarkSeenOptions, ReviewWriteRequest, add_comment, mark_seen,
};
use astroshot_engine::review_data::scan::{
    FindOptions, ShotContext, find_astroshot_dirs, rebuild_shot, scan_feature_dir, scan_tree,
};

/// Depth of the quick pass that finds repo-level trees almost instantly.
pub const SHALLOW_DEPTH: usize = 3;
/// A deep walk this recent is skipped at startup; `r` always forces one.
pub const FULL_SCAN_TTL_MS: i64 = 30 * 60 * 1000;
/// Delay between the last mutation and the index write.
const SAVE_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Idle,
    Warm,
    Shallow,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreEventKind {
    NewShot,
    UpdatedShot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoreEvent {
    pub kind: StoreEventKind,
    pub shot: Shot,
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoreState {
    pub roots: Vec<String>,
    pub shots: Vec<Shot>,
    pub friction_logs: Vec<FrictionLog>,
    pub tree_count: usize,
    pub scanning: bool,
    pub phase: ScanPhase,
    pub watching: bool,
    pub unread_count: usize,
    pub last_event: Option<StoreEvent>,
    pub error: Option<String>,
    pub revision: u64,
}

impl StoreState {
    fn initial(roots: Vec<String>) -> Self {
        Self {
            roots,
            shots: Vec::new(),
            friction_logs: Vec::new(),
            tree_count: 0,
            scanning: false,
            phase: ScanPhase::Idle,
            watching: false,
            unread_count: 0,
            last_event: None,
            error: None,
            revision: 0,
        }
    }
}

/// `StoreOptions`. `None` means the TS default (`useIndex` and `watch` on).
#[derive(Clone, Default)]
pub struct StoreOptions {
    pub roots: Vec<String>,
    /// Defaults to `default_index_cache_path()`.
    pub index_path: Option<PathBuf>,
    /// Skip the durable index (tests).
    pub use_index: Option<bool>,
    pub watch: Option<bool>,
    pub settle_ms: Option<u64>,
    pub on_log: Option<LogHandler>,
}

/// `onLog(message)`.
pub type LogHandler = Arc<dyn Fn(&str) + Send + Sync>;

/// `rescan({ force })`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RescanOptions {
    pub force: bool,
}

/// `markManySeen` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkManyResult {
    pub ok: usize,
    pub failed: usize,
}

/// `type Listener = () => void`.
pub type Listener = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    state: Arc<StoreState>,
    listeners: BTreeMap<u64, Listener>,
    next_listener: u64,
}

/// `Map<string, AstroshotTree>` keyed by `astroshot_dir`, in insertion order.
#[derive(Default)]
struct Trees(Vec<AstroshotTree>);

impl Trees {
    fn position(&self, astroshot_dir: &str) -> Option<usize> {
        self.0
            .iter()
            .position(|tree| tree.astroshot_dir == astroshot_dir)
    }

    fn has(&self, astroshot_dir: &str) -> bool {
        self.position(astroshot_dir).is_some()
    }

    /// `Map#set`: an existing key keeps its position.
    fn set(&mut self, tree: AstroshotTree) {
        match self.position(&tree.astroshot_dir) {
            Some(index) => self.0[index] = tree,
            None => self.0.push(tree),
        }
    }

    fn delete(&mut self, astroshot_dir: &str) {
        self.0.retain(|tree| tree.astroshot_dir != astroshot_dir);
    }

    fn keys(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|tree| tree.astroshot_dir.clone())
            .collect()
    }
}

/// Everything the serialized mutations own.
#[derive(Default)]
struct Data {
    trees: Trees,
    shots_by_path: HashMap<String, Shot>,
    arrival_order: Vec<String>,
    index: Option<IndexDocument>,
    full_scan_at: Option<String>,
    /// Arrival order frozen at scan start so batches do not reorder the stream.
    scan_base_order: Option<Vec<String>>,
}

impl Data {
    fn tree_for(&mut self, astroshot_dir: &str) -> &mut AstroshotTree {
        let index = match self.trees.position(astroshot_dir) {
            Some(index) => index,
            None => {
                let worktree_path = dirname(astroshot_dir);
                let worktree = basename(&worktree_path).to_string();
                self.trees.0.push(AstroshotTree {
                    astroshot_dir: astroshot_dir.to_string(),
                    worktree_path,
                    worktree,
                    shots: Vec::new(),
                    friction_logs: Vec::new(),
                });
                self.trees.0.len() - 1
            }
        };
        &mut self.trees.0[index]
    }

    fn ordered_shots(&self) -> Vec<Shot> {
        self.arrival_order
            .iter()
            .filter_map(|shot_path| self.shots_by_path.get(shot_path).cloned())
            .collect()
    }
}

#[derive(Default)]
struct SaveTimer {
    handle: Option<JoinHandle<()>>,
    /// Bumped on every schedule/cancel; a timer that wakes with an old value does nothing.
    generation: u64,
}

pub struct ReviewStore {
    weak: Weak<ReviewStore>,
    inner: Mutex<Inner>,
    data: Mutex<Data>,
    hashes: HashCache,
    watcher: Mutex<Option<Arc<RootWatcher>>>,
    /// Task that feeds watcher events into `handle_event` in arrival order.
    event_pump: Mutex<Option<JoinHandle<()>>>,
    /// Serialize mutations (`enqueue`).
    queue: tokio::sync::Mutex<()>,
    options: StoreOptions,
    disposed: AtomicBool,
    save_timer: Mutex<SaveTimer>,
}

/// Returned by [`ReviewStore::subscribe`]; dropping it unsubscribes (the TS
/// returns an unsubscribe closure).
pub struct Subscription {
    store: Arc<ReviewStore>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.store.lock().listeners.remove(&self.id);
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// `Date.now()`.
fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// `new Date().toISOString()`.
fn now_iso_millis() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Whether the deep walk must run: forced, never done, unparseable, or older than the TTL.
fn full_scan_is_stale(force: bool, full_scan_at: Option<&str>, now_ms: i64) -> bool {
    if force {
        return true;
    }
    let Some(at) = full_scan_at.filter(|at| !at.is_empty()) else {
        return true;
    };
    match DateTime::parse_from_rfc3339(at) {
        Ok(at) => now_ms - at.timestamp_millis() > FULL_SCAN_TTL_MS,
        Err(_) => true,
    }
}

fn event_kind(event: &WatchEvent) -> &'static str {
    match event {
        WatchEvent::Shot { .. } => "shot",
        WatchEvent::Feature { .. } => "feature",
        WatchEvent::Friction { .. } => "friction",
        WatchEvent::Tree { .. } => "tree",
    }
}

fn write_request(shot: &Shot) -> ReviewWriteRequest {
    ReviewWriteRequest {
        directory: shot.feature_dir.clone(),
        file_name: shot.file_name.clone(),
        run_id: shot.run_id.clone(),
        target_path: shot.path.clone(),
    }
}

impl ReviewStore {
    /// A store over `roots` with every other option at its default.
    pub fn new(roots: Vec<String>) -> Arc<Self> {
        Self::with_options(StoreOptions {
            roots,
            ..StoreOptions::default()
        })
    }

    /// `new ReviewStore(options)`.
    pub fn with_options(options: StoreOptions) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            weak: weak.clone(),
            inner: Mutex::new(Inner {
                state: Arc::new(StoreState::initial(options.roots.clone())),
                listeners: BTreeMap::new(),
                next_listener: 0,
            }),
            data: Mutex::new(Data::default()),
            hashes: HashCache::default(),
            watcher: Mutex::new(None),
            event_pump: Mutex::new(None),
            queue: tokio::sync::Mutex::new(()),
            options,
            disposed: AtomicBool::new(false),
            save_timer: Mutex::new(SaveTimer::default()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        guard(&self.inner)
    }

    fn data(&self) -> MutexGuard<'_, Data> {
        guard(&self.data)
    }

    /// The current snapshot. Each publish replaces it, so snapshots are immutable.
    pub fn get_state(&self) -> Arc<StoreState> {
        self.lock().state.clone()
    }

    pub fn subscribe(self: &Arc<Self>, listener: Listener) -> Subscription {
        let mut inner = self.lock();
        let id = inner.next_listener;
        inner.next_listener += 1;
        inner.listeners.insert(id, listener);
        Subscription {
            store: self.clone(),
            id,
        }
    }

    /// `publish(patch)`: apply `patch` to a copy of the state, bump `revision`, notify listeners.
    pub fn publish_with(&self, patch: impl FnOnce(&mut StoreState)) {
        let listeners: Vec<Listener> = {
            let mut inner = self.lock();
            let mut next = (*inner.state).clone();
            patch(&mut next);
            next.revision = inner.state.revision + 1;
            inner.state = Arc::new(next);
            inner.listeners.values().cloned().collect()
        };
        for listener in listeners {
            listener();
        }
    }

    fn log(&self, message: &str) {
        if let Some(on_log) = &self.options.on_log {
            on_log(message);
        }
    }

    fn index_path(&self) -> PathBuf {
        self.options
            .index_path
            .clone()
            .unwrap_or_else(default_index_cache_path)
    }

    fn current_watcher(&self) -> Option<Arc<RootWatcher>> {
        guard(&self.watcher).clone()
    }

    /// Load the index, start watching, then run the first scan. Must run on the tokio runtime.
    pub async fn start(&self) {
        if self.options.use_index != Some(false)
            && let Some(index) = load_index(&self.options.roots, &self.index_path()).await
        {
            self.hashes.seed(index.hashes.clone());
            let mut data = self.data();
            data.arrival_order = index.arrival_order.clone();
            data.full_scan_at = index.full_scan_at.clone();
            data.index = Some(index);
        }
        if self.options.watch != Some(false) {
            let (events, mut received) = mpsc::unbounded_channel::<WatchEvent>();
            let pump_store = self.weak.clone();
            let pump = tokio::spawn(async move {
                while let Some(event) = received.recv().await {
                    let Some(store) = pump_store.upgrade() else {
                        break;
                    };
                    store.handle_event(event).await;
                }
            });
            *guard(&self.event_pump) = Some(pump);
            let error_store = self.weak.clone();
            let watcher = Arc::new(watch_roots(
                &self.options.roots,
                move |event| {
                    let _ = events.send(event);
                },
                WatchOptions {
                    settle_ms: self.options.settle_ms,
                    on_error: Some(Arc::new(move |root: &str, error: anyhow::Error| {
                        let Some(store) = error_store.upgrade() else {
                            return;
                        };
                        store.log(&format!("watch {root}: {error}"));
                        let watching = store
                            .current_watcher()
                            .is_some_and(|watcher| watcher.supported());
                        store.publish_with(|state| {
                            state.watching = watching;
                            state.error = Some(format!("watch: {error}"));
                        });
                    })),
                    recursive_roots: None,
                },
            ));
            *guard(&self.watcher) = Some(watcher.clone());
            let watching = watcher.supported();
            self.publish_with(|state| state.watching = watching);
        }
        self.rescan(RescanOptions::default()).await;
    }

    /// Warm scan from the index, a shallow walk for repo-level trees, then a
    /// deep walk when the index is stale (or when forced by the user).
    pub async fn rescan(&self, options: RescanOptions) {
        let _turn = self.queue.lock().await;
        let cached_dirs: Vec<String> = {
            let mut data = self.data();
            data.scan_base_order = Some(data.arrival_order.clone());
            data.index
                .as_ref()
                .map(|index| index.astroshot_dirs.clone())
                .unwrap_or_default()
        };
        let mut known: HashSet<String> = HashSet::new();
        if !cached_dirs.is_empty() {
            self.publish_with(|state| {
                state.scanning = true;
                state.phase = ScanPhase::Warm;
            });
            self.scan_trees(&cached_dirs).await;
            {
                let data = self.data();
                for dir in &cached_dirs {
                    if data.trees.has(dir) {
                        known.insert(dir.clone());
                    }
                }
            }
            self.recompute();
        }

        let shallow = self
            .discover(SHALLOW_DEPTH, ScanPhase::Shallow, &mut known)
            .await;
        let full_scan_at = self.data().full_scan_at.clone();
        let stale = full_scan_is_stale(options.force, full_scan_at.as_deref(), now_ms());
        let mut complete = shallow;
        if stale {
            complete = self
                .discover(MAX_SCAN_DEPTH, ScanPhase::Full, &mut known)
                .await;
            self.data().full_scan_at = Some(now_iso_millis());
        }
        // Trees that vanished from disk leave the stream; cached deep trees
        // survive a shallow-only start because they were re-verified above.
        let surviving_cached: Vec<String> = {
            let mut data = self.data();
            for dir in data.trees.keys() {
                if !complete.contains(&dir) && (stale || !cached_dirs.contains(&dir)) {
                    data.trees.delete(&dir);
                }
            }
            cached_dirs
                .iter()
                .filter(|dir| data.trees.has(dir))
                .cloned()
                .collect()
        };
        // Cached trees may have changed while the tray was closed.
        self.scan_trees(&surviving_cached).await;
        self.recompute();
        self.data().scan_base_order = None;
        self.publish_with(|state| {
            state.scanning = false;
            state.phase = ScanPhase::Idle;
        });
        self.schedule_save();
    }

    /// One walk of the roots to `max_depth`; trees stream into the tray while the walk continues.
    async fn discover(
        &self,
        max_depth: usize,
        phase: ScanPhase,
        known: &mut HashSet<String>,
    ) -> HashSet<String> {
        self.publish_with(|state| {
            state.scanning = true;
            state.phase = phase;
        });
        let (found, mut pending) = mpsc::unbounded_channel::<String>();
        let options = FindOptions {
            max_depth: Some(max_depth),
            concurrency: Some(if phase == ScanPhase::Full { 6 } else { 16 }),
            on_found: Some(Arc::new(move |astroshot_dir: &str| {
                let _ = found.send(astroshot_dir.to_string());
            })),
            ..FindOptions::default()
        };
        let roots = &self.options.roots;
        // `options` (and with it the sender) drops when the walk ends, which ends the loop below.
        let walk = async move {
            find_astroshot_dirs(roots, &options).await;
        };
        let flushing = async {
            let mut discovered: HashSet<String> = HashSet::new();
            while let Some(first) = pending.recv().await {
                let mut batch: Vec<String> = Vec::new();
                let mut next = Some(first);
                while let Some(astroshot_dir) = next {
                    discovered.insert(astroshot_dir.clone());
                    if known.insert(astroshot_dir.clone()) {
                        batch.push(astroshot_dir);
                    }
                    next = pending.try_recv().ok();
                }
                if batch.is_empty() {
                    continue;
                }
                self.scan_trees(&batch).await;
                self.recompute();
            }
            discovered
        };
        let ((), discovered) = tokio::join!(walk, flushing);
        discovered
    }

    async fn scan_trees(&self, dirs: &[String]) {
        let concurrency = 6;
        let cursor = AtomicUsize::new(0);
        let watcher = self.current_watcher();
        let worker = || async {
            loop {
                let index = cursor.fetch_add(1, Ordering::SeqCst);
                let Some(dir) = dirs.get(index) else {
                    // Keep the cursor at `dirs.len()` like the TS `while (cursor < length)`.
                    cursor.store(dirs.len(), Ordering::SeqCst);
                    break;
                };
                match scan_tree(dir, &self.hashes).await {
                    Ok(tree) => {
                        self.data().trees.set(tree);
                        if let Some(watcher) = &watcher {
                            watcher.watch_tree(dir);
                        }
                    }
                    Err(error) => self.log(&format!("scan {dir}: {error}")),
                }
                if cursor.load(Ordering::SeqCst).is_multiple_of(8) {
                    self.recompute();
                }
            }
        };
        futures::future::join_all((0..concurrency.min(dirs.len())).map(|_| worker())).await;
    }

    /// Rebuild the flat, ordered shot and friction lists from the trees.
    fn recompute(&self) {
        let (shots, mut friction_logs, tree_count) = {
            let mut data = self.data();
            let data = &mut *data;
            let mut all: Vec<Shot> = Vec::new();
            let mut friction_logs: Vec<FrictionLog> = Vec::new();
            for tree in &data.trees.0 {
                all.extend(tree.shots.iter().cloned());
                friction_logs.extend(tree.friction_logs.iter().cloned());
            }
            let arrivals: Vec<ArrivalShot> = all
                .iter()
                .map(|shot| ArrivalShot {
                    path: shot.path.clone(),
                    captured_at: shot.captured_at,
                })
                .collect();
            data.shots_by_path.clear();
            for shot in all {
                data.shots_by_path.insert(shot.path.clone(), shot);
            }
            data.arrival_order = reconcile_arrival_order(
                data.scan_base_order.as_ref().unwrap_or(&data.arrival_order),
                &arrivals,
            );
            (data.ordered_shots(), friction_logs, data.trees.0.len())
        };
        friction_logs.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.publish_with(|state| {
            state.shots = shots;
            state.friction_logs = friction_logs;
            state.tree_count = tree_count;
        });
    }

    fn publish_shots(&self) {
        let shots = self.data().ordered_shots();
        self.publish_with(|state| state.shots = shots);
    }

    /// Debounced index write (TS: an unref'd 500 ms timer). The task holds
    /// only a weak reference, so it never keeps the store alive.
    fn schedule_save(&self) {
        if self.options.use_index == Some(false) {
            return;
        }
        let mut timer = guard(&self.save_timer);
        if let Some(previous) = timer.handle.take() {
            previous.abort();
        }
        timer.generation += 1;
        let generation = timer.generation;
        let weak = self.weak.clone();
        timer.handle = Some(tokio::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            let Some(store) = weak.upgrade() else {
                return;
            };
            {
                let mut timer = guard(&store.save_timer);
                if timer.generation != generation {
                    return;
                }
                timer.handle = None;
            }
            store.save_index_now().await;
        }));
    }

    async fn save_index_now(&self) {
        let document = {
            let data = self.data();
            let keep: HashSet<String> = data.shots_by_path.keys().cloned().collect();
            self.hashes
                .retain(&keep, |file_path| file_path.ends_with("log.jsonl"));
            let mut astroshot_dirs = data.trees.keys();
            // `Array#sort()` compares UTF-16 code units.
            astroshot_dirs.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            IndexDocument {
                version: Version1,
                roots: self.options.roots.clone(),
                astroshot_dirs,
                arrival_order: data.arrival_order.clone(),
                hashes: self.hashes.to_json(),
                updated_at: now_iso_millis(),
                full_scan_at: data.full_scan_at.clone(),
            }
        };
        match save_index(&document, &self.index_path()).await {
            Ok(()) => self.data().index = Some(document),
            Err(error) => self.log(&format!("index save: {error}")),
        }
    }

    /// Apply one watcher event. Events are applied one at a time, in call order.
    pub async fn handle_event(&self, event: WatchEvent) {
        let _turn = self.queue.lock().await;
        let result: std::io::Result<()> = match &event {
            WatchEvent::Shot {
                path,
                feature_dir,
                astroshot_dir,
            } => {
                self.ingest_shot(path, feature_dir, astroshot_dir).await;
                Ok(())
            }
            WatchEvent::Feature {
                feature_dir,
                astroshot_dir,
            } => {
                self.refresh_feature(feature_dir, astroshot_dir).await;
                Ok(())
            }
            WatchEvent::Friction { astroshot_dir } => self.refresh_friction(astroshot_dir).await,
            WatchEvent::Tree { astroshot_dir } => {
                self.refresh_tree(astroshot_dir).await;
                Ok(())
            }
        };
        if let Err(error) = result {
            self.log(&format!("event {}: {error}", event_kind(&event)));
        }
        self.schedule_save();
    }

    /// `treeFor(dir)` reduced to what callers read before their first await.
    fn tree_context(&self, astroshot_dir: &str) -> FrictionContext {
        let mut data = self.data();
        let tree = data.tree_for(astroshot_dir);
        FrictionContext {
            worktree_path: tree.worktree_path.clone(),
            worktree: tree.worktree.clone(),
        }
    }

    async fn ingest_shot(&self, path: &str, feature_dir: &str, astroshot_dir: &str) {
        let context = self.tree_context(astroshot_dir);
        let shot = rebuild_shot(
            path,
            &ShotContext {
                worktree_path: context.worktree_path,
                worktree: context.worktree,
                feature: basename(feature_dir).to_string(),
                feature_dir: feature_dir.to_string(),
            },
            &self.hashes,
        )
        .await;
        let (shots, tree_count, was_known) = {
            let mut data = self.data();
            let data = &mut *data;
            let was_known = data.shots_by_path.contains_key(path);
            let tree = data.tree_for(astroshot_dir);
            tree.shots.retain(|candidate| candidate.path != path);
            match &shot {
                None => {
                    // Image vanished: drop it everywhere.
                    data.shots_by_path.remove(path);
                    data.arrival_order.retain(|entry| entry != path);
                }
                Some(shot) => {
                    tree.shots.push(shot.clone());
                    data.shots_by_path.insert(shot.path.clone(), shot.clone());
                    data.arrival_order.retain(|entry| *entry != shot.path);
                    data.arrival_order.insert(0, shot.path.clone());
                }
            }
            (data.ordered_shots(), data.trees.0.len(), was_known)
        };
        let Some(shot) = shot else {
            self.publish_with(|state| state.shots = shots);
            return;
        };
        let at = now_ms() as f64;
        self.publish_with(|state| {
            state.shots = shots;
            state.tree_count = tree_count;
            if !was_known {
                state.unread_count += 1;
            }
            state.last_event = Some(StoreEvent {
                kind: if was_known {
                    StoreEventKind::UpdatedShot
                } else {
                    StoreEventKind::NewShot
                },
                shot,
                at,
            });
        });
    }

    async fn refresh_feature(&self, feature_dir: &str, astroshot_dir: &str) {
        let context = self.tree_context(astroshot_dir);
        let shots = scan_feature_dir(feature_dir, &context, &self.hashes).await;
        let mut fresh: Vec<Shot> = {
            let mut data = self.data();
            let tree = data.tree_for(astroshot_dir);
            let previous: HashSet<String> = tree
                .shots
                .iter()
                .filter(|shot| shot.feature_dir == feature_dir)
                .map(|shot| shot.path.clone())
                .collect();
            tree.shots.retain(|shot| shot.feature_dir != feature_dir);
            tree.shots.extend(shots.iter().cloned());
            shots
                .into_iter()
                .filter(|shot| !previous.contains(&shot.path))
                .collect()
        };
        self.recompute();
        if !fresh.is_empty() {
            let count = fresh.len();
            fresh.sort_by(|a, b| {
                b.captured_at
                    .partial_cmp(&a.captured_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let newest = fresh.swap_remove(0);
            let at = now_ms() as f64;
            self.publish_with(|state| {
                state.unread_count += count;
                state.last_event = Some(StoreEvent {
                    kind: StoreEventKind::NewShot,
                    shot: newest,
                    at,
                });
            });
        }
    }

    async fn refresh_friction(&self, astroshot_dir: &str) -> std::io::Result<()> {
        let context = self.tree_context(astroshot_dir);
        let friction_logs = load_user_stories(astroshot_dir, &context, &self.hashes).await?;
        self.data().tree_for(astroshot_dir).friction_logs = friction_logs;
        self.recompute();
        Ok(())
    }

    async fn refresh_tree(&self, astroshot_dir: &str) {
        let scanned = scan_tree(astroshot_dir, &self.hashes).await;
        {
            let mut data = self.data();
            match scanned {
                Ok(tree) if !(tree.shots.is_empty() && tree.friction_logs.is_empty()) => {
                    data.trees.set(tree);
                }
                _ => data.trees.delete(astroshot_dir),
            }
        }
        self.recompute();
    }

    /// The user opened the stream: clear the unread badge.
    pub fn mark_opened(&self) {
        if self.get_state().unread_count != 0 {
            self.publish_with(|state| state.unread_count = 0);
        }
    }

    pub async fn mark_shot_seen(&self, shot: &Shot, comment: Option<&str>) -> anyhow::Result<Shot> {
        mark_seen(
            &write_request(shot),
            &MarkSeenOptions {
                comment: comment.map(str::to_string),
                now: None,
            },
        )
        .await?;
        Ok(self.reload_shot(shot).await)
    }

    pub async fn add_shot_comment(&self, shot: &Shot, body: &str) -> anyhow::Result<Shot> {
        add_comment(&write_request(shot), body, &AddCommentOptions::default()).await?;
        Ok(self.reload_shot(shot).await)
    }

    async fn reload_shot(&self, shot: &Shot) -> Shot {
        let _turn = self.queue.lock().await;
        let astroshot_dir = dirname(&shot.feature_dir);
        self.tree_context(&astroshot_dir);
        let Some(rebuilt) = rebuild_shot(
            &shot.path,
            &ShotContext {
                worktree_path: shot.worktree_path.clone(),
                worktree: shot.worktree.clone(),
                feature: shot.feature.clone(),
                feature_dir: shot.feature_dir.clone(),
            },
            &self.hashes,
        )
        .await
        else {
            return shot.clone();
        };
        {
            let mut data = self.data();
            let data = &mut *data;
            let tree = data.tree_for(&astroshot_dir);
            let mut replaced = false;
            for candidate in &mut tree.shots {
                if candidate.path == shot.path {
                    *candidate = rebuilt.clone();
                    replaced = true;
                }
            }
            if !replaced {
                tree.shots.push(rebuilt.clone());
            }
            data.shots_by_path
                .insert(rebuilt.path.clone(), rebuilt.clone());
        }
        // Sidecar writes for one shot also re-scope siblings on a run change.
        self.publish_shots();
        rebuilt
    }

    /// Mark many shots seen; failures are counted, never fatal.
    pub async fn mark_many_seen(&self, shots: &[Shot]) -> MarkManyResult {
        let mut ok = 0;
        let mut failed = 0;
        for shot in shots {
            match self.mark_shot_seen(shot, None).await {
                Ok(_) => ok += 1,
                Err(_) => failed += 1,
            }
        }
        // Reload every touched feature so run resets propagate.
        let mut features: Vec<&str> = Vec::new();
        for shot in shots {
            if !features.contains(&shot.feature_dir.as_str()) {
                features.push(&shot.feature_dir);
            }
        }
        for feature_dir in features {
            self.handle_event(WatchEvent::Feature {
                feature_dir: feature_dir.to_string(),
                astroshot_dir: dirname(feature_dir),
            })
            .await;
        }
        MarkManyResult { ok, failed }
    }

    pub async fn mark_friction_run_seen(
        &self,
        log: &FrictionLog,
        run: &FrictionRun,
    ) -> anyhow::Result<()> {
        let Some(log_path) = run.log_path.as_deref().filter(|path| !path.is_empty()) else {
            return Err(anyhow!("This run has no log.jsonl to acknowledge"));
        };
        mark_seen(
            &ReviewWriteRequest {
                directory: run.directory.clone(),
                file_name: LOG_FILE.to_string(),
                run_id: Some(run.run_id.clone()),
                target_path: log_path.to_string(),
            },
            &MarkSeenOptions::default(),
        )
        .await?;
        self.handle_event(WatchEvent::Friction {
            astroshot_dir: join(&log.worktree_path, ".astroshot"),
        })
        .await;
        Ok(())
    }

    /// Stop watching and flush a pending index write. Later calls do nothing.
    pub async fn dispose(&self) {
        if self.disposed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(watcher) = self.current_watcher() {
            watcher.close();
        }
        if let Some(pump) = guard(&self.event_pump).take() {
            pump.abort();
        }
        let pending = {
            let mut timer = guard(&self.save_timer);
            timer.generation += 1;
            timer.handle.take()
        };
        if let Some(pending) = pending {
            pending.abort();
            self.save_index_now().await;
        }
    }
}

impl Drop for ReviewStore {
    fn drop(&mut self) {
        if let Some(pump) = guard(&self.event_pump).take() {
            pump.abort();
        }
        if let Some(pending) = guard(&self.save_timer).handle.take() {
            pending.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publish_bumps_revision_and_notifies_until_unsubscribed() {
        let store = ReviewStore::new(vec!["/r".into()]);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let sub = store.subscribe(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        store.publish_with(|s| s.scanning = true);
        let state = store.get_state();
        assert_eq!((state.revision, state.scanning), (1, true));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(sub);
        store.publish_with(|s| s.scanning = false);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.get_state().revision, 2);
    }

    // The cases below have no TS counterpart (store.ts has no test file).

    use astroshot_engine::review_data::model::ReviewState;
    use std::path::{Path, PathBuf};
    use tokio::sync::Notify;

    const WAIT: Duration = Duration::from_secs(20);

    fn write(file: &Path, content: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    fn s(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    /// A tempdir with one repo-level tree holding two shots in one feature.
    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new().prefix("store-").tempdir().unwrap();
        let path = dir.path().to_path_buf();
        write(&path.join("app/.astroshot/login/0001-a.png"), "a");
        write(&path.join("app/.astroshot/login/0002-b.png"), "b");
        (dir, path)
    }

    fn options(root: &Path) -> StoreOptions {
        StoreOptions {
            roots: vec![s(root)],
            use_index: Some(false),
            watch: Some(false),
            ..StoreOptions::default()
        }
    }

    fn paths(state: &StoreState) -> Vec<String> {
        let mut paths: Vec<String> = state.shots.iter().map(|shot| shot.path.clone()).collect();
        paths.sort();
        paths
    }

    fn shot(store: &ReviewStore, path: &Path) -> Shot {
        let path = s(path);
        store
            .get_state()
            .shots
            .iter()
            .find(|shot| shot.path == path)
            .cloned()
            .unwrap_or_else(|| panic!("no shot {path}"))
    }

    /// Records the phase of every published snapshot.
    fn record_phases(store: &Arc<ReviewStore>) -> (Subscription, Arc<Mutex<Vec<ScanPhase>>>) {
        let phases = Arc::new(Mutex::new(Vec::new()));
        let seen = phases.clone();
        let weak = Arc::downgrade(store);
        let subscription = store.subscribe(Arc::new(move || {
            if let Some(store) = weak.upgrade() {
                let phase = store.get_state().phase;
                let mut seen = seen.lock().unwrap();
                if seen.last() != Some(&phase) {
                    seen.push(phase);
                }
            }
        }));
        (subscription, phases)
    }

    /// Waits on published snapshots (not the clock) until `ready` holds.
    async fn wait_for(
        store: &Arc<ReviewStore>,
        what: &str,
        ready: impl Fn(&StoreState) -> bool,
    ) -> Arc<StoreState> {
        let published = Arc::new(Notify::new());
        let notify = published.clone();
        let _subscription = store.subscribe(Arc::new(move || notify.notify_one()));
        let waiting = async {
            loop {
                let state = store.get_state();
                if ready(&state) {
                    return state;
                }
                published.notified().await;
            }
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    fn assert_send<T: Send>(value: T) -> T {
        value
    }

    #[tokio::test]
    async fn initial_scan_lists_every_tree_and_walks_shallow_then_full_then_idle() {
        let (_guard, root) = root();
        // Deeper than SHALLOW_DEPTH: only the full walk finds it.
        write(&root.join("a/b/c/d/deep/.astroshot/f/0001-e.png"), "e");
        let store = ReviewStore::with_options(options(&root));
        let (_subscription, phases) = record_phases(&store);
        assert_eq!(store.get_state().phase, ScanPhase::Idle);

        assert_send(store.start()).await;

        let state = store.get_state();
        assert_eq!(state.roots, vec![s(&root)]);
        assert_eq!(
            paths(&state),
            vec![
                s(&root.join("a/b/c/d/deep/.astroshot/f/0001-e.png")),
                s(&root.join("app/.astroshot/login/0001-a.png")),
                s(&root.join("app/.astroshot/login/0002-b.png")),
            ]
        );
        assert_eq!(state.tree_count, 2);
        assert_eq!((state.scanning, state.phase), (false, ScanPhase::Idle));
        assert_eq!((state.watching, state.unread_count), (false, 0));
        assert_eq!(state.last_event, None);
        assert_eq!(state.error, None);
        assert_eq!(
            *phases.lock().unwrap(),
            vec![ScanPhase::Shallow, ScanPhase::Full, ScanPhase::Idle]
        );
    }

    #[tokio::test]
    async fn dispose_writes_the_index_and_the_next_start_warms_from_it_without_a_full_walk() {
        let (_guard, root) = root();
        write(&root.join("a/b/c/d/deep/.astroshot/f/0001-e.png"), "e");
        let index_path = root.join("cache/index.json");
        let with_index = || StoreOptions {
            use_index: None,
            index_path: Some(index_path.clone()),
            ..options(&root)
        };

        let first = ReviewStore::with_options(with_index());
        first.start().await;
        let arrival: Vec<String> = first
            .get_state()
            .shots
            .iter()
            .map(|shot| shot.path.clone())
            .collect();
        // The save is debounced; dispose flushes it.
        first.dispose().await;
        first.dispose().await;

        let text = std::fs::read_to_string(&index_path).unwrap();
        let prefix = format!(
            r#"{{"version":1,"roots":[{}],"astroshotDirs":[{},{}],"arrivalOrder":"#,
            serde_json::to_string(&s(&root)).unwrap(),
            serde_json::to_string(&s(&root.join("a/b/c/d/deep/.astroshot"))).unwrap(),
            serde_json::to_string(&s(&root.join("app/.astroshot"))).unwrap(),
        );
        assert!(text.starts_with(&prefix), "{text}");
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&str> = parsed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec![
                "version",
                "roots",
                "astroshotDirs",
                "arrivalOrder",
                "hashes",
                "updatedAt",
                "fullScanAt"
            ]
        );
        assert_eq!(
            parsed["arrivalOrder"],
            serde_json::to_value(&arrival).unwrap()
        );
        for key in ["updatedAt", "fullScanAt"] {
            // `toISOString()`: milliseconds and `Z`.
            let stamp = parsed[key].as_str().unwrap();
            assert_eq!(stamp.len(), 24, "{stamp}");
            assert!(stamp.ends_with('Z') && stamp.as_bytes()[19] == b'.');
        }

        let second = ReviewStore::with_options(with_index());
        let (_subscription, phases) = record_phases(&second);
        second.start().await;
        // The deep tree is out of the shallow walk's reach but stays: it was re-verified warm.
        assert_eq!(second.get_state().tree_count, 2);
        assert_eq!(
            second
                .get_state()
                .shots
                .iter()
                .map(|shot| shot.path.clone())
                .collect::<Vec<_>>(),
            arrival
        );
        assert_eq!(
            *phases.lock().unwrap(),
            vec![ScanPhase::Warm, ScanPhase::Shallow, ScanPhase::Idle]
        );

        // `r` forces the deep walk.
        phases.lock().unwrap().clear();
        second.rescan(RescanOptions { force: true }).await;
        assert_eq!(
            *phases.lock().unwrap(),
            vec![
                ScanPhase::Warm,
                ScanPhase::Shallow,
                ScanPhase::Full,
                ScanPhase::Idle
            ]
        );
        second.dispose().await;
    }

    #[test]
    fn full_scan_is_stale_when_forced_missing_unparseable_or_past_the_ttl() {
        let at = "2026-09-05T17:42:00.000Z";
        let then = DateTime::parse_from_rfc3339(at).unwrap().timestamp_millis();
        assert!(!full_scan_is_stale(
            false,
            Some(at),
            then + FULL_SCAN_TTL_MS
        ));
        assert!(full_scan_is_stale(
            false,
            Some(at),
            then + FULL_SCAN_TTL_MS + 1
        ));
        assert!(full_scan_is_stale(true, Some(at), then));
        assert!(full_scan_is_stale(false, None, then));
        assert!(full_scan_is_stale(false, Some(""), then));
        assert!(full_scan_is_stale(false, Some("yesterday"), then));
    }

    #[tokio::test]
    async fn rescan_drops_trees_that_vanished_from_disk() {
        let (_guard, root) = root();
        write(&root.join("other/.astroshot/f/0001-c.png"), "c");
        let store = ReviewStore::with_options(options(&root));
        store.start().await;
        assert_eq!(store.get_state().tree_count, 2);

        std::fs::remove_dir_all(root.join("other")).unwrap();
        store.rescan(RescanOptions { force: true }).await;

        let state = store.get_state();
        assert_eq!(state.tree_count, 1);
        assert_eq!(
            paths(&state),
            vec![
                s(&root.join("app/.astroshot/login/0001-a.png")),
                s(&root.join("app/.astroshot/login/0002-b.png")),
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_file_arriving_while_watching_becomes_an_unread_shot_at_the_head_of_the_stream() {
        let (_guard, root) = root();
        let store = ReviewStore::with_options(StoreOptions {
            watch: None,
            settle_ms: Some(20),
            ..options(&root)
        });
        store.start().await;
        assert!(store.get_state().watching);
        assert_eq!(store.get_state().shots.len(), 2);

        let arrived = s(&root.join("app/.astroshot/login/0003-c.png"));
        write(Path::new(&arrived), "c");

        let state = wait_for(&store, "the new shot", |state| {
            state.shots.iter().any(|shot| shot.path == arrived) && state.last_event.is_some()
        })
        .await;
        assert_eq!(state.shots[0].path, arrived);
        assert_eq!(state.shots.len(), 3);
        assert_eq!(state.unread_count, 1);
        let event = state.last_event.clone().unwrap();
        assert_eq!(event.shot.path, arrived);
        assert_eq!(event.kind, StoreEventKind::NewShot);

        // Opening the stream clears the badge, once.
        store.mark_opened();
        let opened = store.get_state();
        assert_eq!(opened.unread_count, 0);
        store.mark_opened();
        assert_eq!(store.get_state().revision, opened.revision);

        store.dispose().await;
    }

    #[tokio::test]
    async fn shot_events_add_update_and_remove_one_shot() {
        let (_guard, root) = root();
        let store = ReviewStore::with_options(options(&root));
        store.start().await;
        let feature_dir = s(&root.join("app/.astroshot/login"));
        let astroshot_dir = s(&root.join("app/.astroshot"));
        let path = s(&root.join("app/.astroshot/login/0003-c.png"));
        let event = || WatchEvent::Shot {
            path: path.clone(),
            feature_dir: feature_dir.clone(),
            astroshot_dir: astroshot_dir.clone(),
        };

        write(Path::new(&path), "c");
        store.handle_event(event()).await;
        let added = store.get_state();
        assert_eq!(added.shots[0].path, path);
        assert_eq!(added.unread_count, 1);
        assert_eq!(
            added.last_event.as_ref().map(|event| event.kind),
            Some(StoreEventKind::NewShot)
        );

        // The same path again is an update: it moves to the head, the badge does not grow.
        store
            .handle_event(WatchEvent::Shot {
                path: s(&root.join("app/.astroshot/login/0001-a.png")),
                feature_dir: feature_dir.clone(),
                astroshot_dir: astroshot_dir.clone(),
            })
            .await;
        let updated = store.get_state();
        assert_eq!(
            updated.shots[0].path,
            s(&root.join("app/.astroshot/login/0001-a.png"))
        );
        assert_eq!(updated.shots[1].path, path);
        assert_eq!((updated.shots.len(), updated.unread_count), (3, 1));
        assert_eq!(
            updated.last_event.as_ref().map(|event| event.kind),
            Some(StoreEventKind::UpdatedShot)
        );

        // Image vanished: dropped everywhere, no new event.
        std::fs::remove_file(&path).unwrap();
        store.handle_event(event()).await;
        let removed = store.get_state();
        assert_eq!(removed.shots.len(), 2);
        assert!(removed.shots.iter().all(|shot| shot.path != path));
        assert_eq!(removed.last_event, updated.last_event);
    }

    #[tokio::test]
    async fn feature_and_tree_events_reconcile_whole_directories() {
        let (_guard, root) = root();
        let store = ReviewStore::with_options(options(&root));
        store.start().await;

        // A feature in a tree the store has never seen.
        write(&root.join("new/.astroshot/f/0001-x.png"), "x");
        write(&root.join("new/.astroshot/f/0002-y.png"), "y");
        store
            .handle_event(WatchEvent::Feature {
                feature_dir: s(&root.join("new/.astroshot/f")),
                astroshot_dir: s(&root.join("new/.astroshot")),
            })
            .await;
        let state = store.get_state();
        assert_eq!((state.tree_count, state.shots.len()), (2, 4));
        assert_eq!(state.unread_count, 2);
        assert_eq!(
            state.last_event.as_ref().map(|event| event.kind),
            Some(StoreEventKind::NewShot)
        );
        assert_eq!(state.shots[0].worktree, "new");

        // Unchanged feature: nothing is fresh.
        store
            .handle_event(WatchEvent::Feature {
                feature_dir: s(&root.join("new/.astroshot/f")),
                astroshot_dir: s(&root.join("new/.astroshot")),
            })
            .await;
        assert_eq!(store.get_state().unread_count, 2);

        // The tree is deleted on disk.
        std::fs::remove_dir_all(root.join("new")).unwrap();
        store
            .handle_event(WatchEvent::Tree {
                astroshot_dir: s(&root.join("new/.astroshot")),
            })
            .await;
        let state = store.get_state();
        assert_eq!((state.tree_count, state.shots.len()), (1, 2));
    }

    #[tokio::test]
    async fn mark_seen_and_add_comment_round_trip_through_review_json() {
        let (_guard, root) = root();
        let store = ReviewStore::with_options(options(&root));
        store.start().await;
        let path = root.join("app/.astroshot/login/0001-a.png");
        let pending = shot(&store, &path);
        assert_ne!(
            pending.review.as_ref().map(|review| review.state),
            Some(ReviewState::Seen)
        );

        let seen = store
            .mark_shot_seen(&pending, Some("ship it"))
            .await
            .unwrap();
        let review = seen.review.clone().unwrap();
        assert_eq!(review.state, ReviewState::Seen);
        assert_eq!(review.decision.as_deref(), Some("seen"));
        assert_eq!(review.comments.len(), 1);
        assert_eq!(review.comments[0].body, "ship it");
        // The published snapshot carries the rebuilt shot.
        assert_eq!(shot(&store, &path), seen);

        let commented = store
            .add_shot_comment(&seen, "one more thing")
            .await
            .unwrap();
        let bodies: Vec<String> = commented
            .review
            .clone()
            .unwrap()
            .comments
            .into_iter()
            .map(|comment| comment.body)
            .collect();
        assert_eq!(bodies, vec!["ship it", "one more thing"]);
        assert_eq!(shot(&store, &path), commented);

        let document: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("app/.astroshot/login/review.json")).unwrap(),
        )
        .unwrap();
        let entry = &document["reviews"]["0001-a.png"];
        assert_eq!(entry["decision"], "seen");
        assert_eq!(entry["comments"].as_array().unwrap().len(), 2);
        assert!(document["reviews"].get("0002-b.png").is_none());

        let error = store.add_shot_comment(&commented, "  ").await.unwrap_err();
        assert_eq!(error.to_string(), "Feedback cannot be empty");
    }

    #[tokio::test]
    async fn mark_many_seen_counts_failures_and_reloads_each_feature() {
        let (_guard, root) = root();
        let store = ReviewStore::with_options(options(&root));
        store.start().await;
        let mut shots: Vec<Shot> = store.get_state().shots.to_vec();
        let mut missing = shots[0].clone();
        missing.path = s(&root.join("app/.astroshot/login/0009-gone.png"));
        missing.file_name = "0009-gone.png".into();
        shots.push(missing);

        let result = store.mark_many_seen(&shots).await;

        assert_eq!(result, MarkManyResult { ok: 2, failed: 1 });
        let state = store.get_state();
        assert_eq!(state.shots.len(), 2);
        assert!(state.shots.iter().all(|shot| {
            shot.review.as_ref().map(|review| review.state) == Some(ReviewState::Seen)
        }));
        // Reloading a known feature finds nothing fresh.
        assert_eq!(state.unread_count, 0);
    }

    #[tokio::test]
    async fn friction_runs_are_acknowledged_through_their_log_and_refuse_without_one() {
        let (_guard, root) = root();
        let run_dir = root.join("app/.astroshot/friction-logs/signup/runs/20260811T153000Z");
        write(&run_dir.join("log.jsonl"), "{\"step\":1,\"id\":\"a\"}\n");
        let store = ReviewStore::with_options(options(&root));
        store.start().await;
        let state = store.get_state();
        assert_eq!(state.friction_logs.len(), 1);
        let log = state.friction_logs[0].clone();
        let run = log.runs[0].clone();
        assert_ne!(
            run.review.as_ref().map(|review| review.state),
            Some(ReviewState::Seen)
        );

        store.mark_friction_run_seen(&log, &run).await.unwrap();

        let reloaded = store.get_state().friction_logs[0].runs[0].clone();
        assert_eq!(
            reloaded.review.map(|review| review.state),
            Some(ReviewState::Seen)
        );
        assert!(run_dir.join("review.json").exists());

        let mut without_log = run.clone();
        without_log.log_path = None;
        let error = store
            .mark_friction_run_seen(&log, &without_log)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "This run has no log.jsonl to acknowledge"
        );
    }

    #[tokio::test]
    async fn stories_runs_are_acknowledged_through_their_log() {
        let (_guard, root) = root();
        let run_dir = root.join("app/.astroshot/stories/signup/runs/20260811T153000Z");
        write(&run_dir.join("log.jsonl"), "{\"step\":1,\"id\":\"a\"}\n");
        let store = ReviewStore::with_options(options(&root));
        store.start().await;
        let log = store.get_state().friction_logs[0].clone();
        assert!(log.directory.ends_with(".astroshot/stories/signup"));
        store
            .mark_friction_run_seen(&log, &log.runs[0])
            .await
            .unwrap();
        let reloaded = store.get_state().friction_logs[0].runs[0].clone();
        assert_eq!(
            reloaded.review.map(|review| review.state),
            Some(ReviewState::Seen)
        );
        assert!(run_dir.join("review.json").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dispose_stops_the_watcher() {
        let (_guard, root) = root();
        let store = ReviewStore::with_options(StoreOptions {
            watch: None,
            settle_ms: Some(20),
            ..options(&root)
        });
        store.start().await;
        // Prove the watch is live first, so the silence below means something.
        write(&root.join("app/.astroshot/login/0003-c.png"), "c");
        wait_for(&store, "the watched shot", |state| state.shots.len() == 3).await;

        store.dispose().await;
        assert!(guard(&store.event_pump).is_none());
        let revision = store.get_state().revision;

        write(&root.join("app/.astroshot/login/0004-d.png"), "d");
        // Nothing can signal "no event"; give a live watcher 15 settle windows to report.
        let published = Arc::new(Notify::new());
        let notify = published.clone();
        let _subscription = store.subscribe(Arc::new(move || notify.notify_one()));
        let silent = tokio::time::timeout(Duration::from_millis(300), published.notified()).await;
        assert!(silent.is_err(), "a disposed store published again");
        assert_eq!(store.get_state().revision, revision);
    }

    #[tokio::test]
    async fn log_lines_reach_on_log() {
        let (_guard, root) = root();
        let messages = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = messages.clone();
        let store = ReviewStore::with_options(StoreOptions {
            on_log: Some(Arc::new(move |message: &str| {
                sink.lock().unwrap().push(message.to_string());
            })),
            ..options(&root)
        });
        store.start().await;
        // A clean scan logs nothing.
        assert!(messages.lock().unwrap().is_empty());
        store.log("scan /x: boom");
        assert_eq!(*messages.lock().unwrap(), vec!["scan /x: boom".to_string()]);
    }
}
