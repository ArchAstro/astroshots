//! Port of `packages/astroshot-review/src/data/watcher.ts`.
//!
//! Recursive filesystem watching over the roots, routed the same way the app
//! routes FSEvents: image files ingest individually, sidecars refresh their
//! feature, friction-log paths refresh the whole friction namespace, and new
//! or vanished `.astroshot` directories rescan the tree.
//!
//! `fs.watch` -> `notify`, one `RecommendedWatcher` per watched target (the
//! TS keeps one `FSWatcher` per target). Divergences from `fs.watch`:
//!
//! - notify reports absolute paths; they are re-rooted under the target as
//!   given (macOS reports `/private/var/...` for a `/var/...` target) so event
//!   paths match what TS builds with `path.join(target, filename)`.
//! - Access events (opens, closes) are ignored; `fs.watch` never emits them.
//! - Where notify delivers several events for one change (create + modify),
//!   the per-key debounce collapses them, same as for `fs.watch`.
//! - Watcher errors arrive without the watcher handle, so a failed target is
//!   removed on a helper thread (dropping a notify watcher joins its thread,
//!   which cannot happen from inside its own callback).
//! - Debounce timers live on one worker thread instead of one timer each;
//!   behavior (per-key, resets on every event, fires once after `settle_ms`)
//!   is the same.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::paths::{
    ASTROSHOT_DIR, FRICTION_DIR, VIDEO_EXTENSIONS, dirname, extension_of, is_image_file, join,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Shot {
        path: String,
        feature_dir: String,
        astroshot_dir: String,
    },
    Feature {
        feature_dir: String,
        astroshot_dir: String,
    },
    Friction {
        astroshot_dir: String,
    },
    Tree {
        astroshot_dir: String,
    },
}

const SEP: char = '/';

pub fn classify_path(full_path: &str) -> Option<WatchEvent> {
    let parts: Vec<&str> = full_path.split(SEP).collect();
    let index = parts.iter().position(|part| *part == ASTROSHOT_DIR)?;
    let astroshot_dir = parts[..=index].join("/");
    let rest = &parts[index + 1..];
    let Some(first) = rest.first() else {
        return Some(WatchEvent::Tree { astroshot_dir });
    };
    if *first == FRICTION_DIR {
        return Some(WatchEvent::Friction { astroshot_dir });
    }
    if first.starts_with('.') {
        return None;
    }
    let feature_dir = join(&astroshot_dir, first);
    match rest.len() {
        1 => Some(WatchEvent::Feature {
            feature_dir,
            astroshot_dir,
        }),
        2 => {
            let name = rest[1];
            if name == "manifest.json" || name == "review.json" {
                return Some(WatchEvent::Feature {
                    feature_dir,
                    astroshot_dir,
                });
            }
            if is_image_file(name) {
                return Some(WatchEvent::Shot {
                    path: full_path.to_string(),
                    feature_dir,
                    astroshot_dir,
                });
            }
            if VIDEO_EXTENSIONS.contains(&extension_of(name).as_str()) {
                return Some(WatchEvent::Feature {
                    feature_dir,
                    astroshot_dir,
                });
            }
            None
        }
        _ => None,
    }
}

pub fn event_key(event: &WatchEvent) -> String {
    match event {
        WatchEvent::Shot { path, .. } => format!("shot:{path}"),
        WatchEvent::Feature { feature_dir, .. } => format!("feature:{feature_dir}"),
        WatchEvent::Friction { astroshot_dir } => format!("friction:{astroshot_dir}"),
        WatchEvent::Tree { astroshot_dir } => format!("tree:{astroshot_dir}"),
    }
}

/// `onError(root, error)`.
pub type WatchErrorHandler = Arc<dyn Fn(&str, anyhow::Error) + Send + Sync>;

pub const DEFAULT_SETTLE_MS: u64 = 250;

#[derive(Clone, Default)]
pub struct WatchOptions {
    /// Debounce window; defaults to [`DEFAULT_SETTLE_MS`].
    pub settle_ms: Option<u64>,
    pub on_error: Option<WatchErrorHandler>,
    /// Watch each root recursively. Cheap where the OS offers a single
    /// recursive stream (FSEvents on macOS, ReadDirectoryChangesW on Windows);
    /// elsewhere every directory costs an inotify watch, so only discovered
    /// `.astroshot` trees are watched and new trees need a rescan. Defaults to
    /// macOS / Windows.
    pub recursive_roots: Option<bool>,
}

struct Debounce {
    pending: HashMap<String, (WatchEvent, Instant)>,
    closed: bool,
}

struct Shared {
    settle: Duration,
    on_event: Box<dyn Fn(WatchEvent) + Send + Sync>,
    on_error: Option<WatchErrorHandler>,
    debounce: Mutex<Debounce>,
    wake: Condvar,
    watchers: Mutex<HashMap<String, RecommendedWatcher>>,
    supported: AtomicBool,
}

impl Shared {
    fn schedule(&self, event: WatchEvent) {
        let key = event_key(&event);
        let deadline = Instant::now() + self.settle;
        let mut state = self.debounce.lock().expect("debounce lock");
        if state.closed {
            return;
        }
        state.pending.insert(key, (event, deadline));
        self.wake.notify_all();
    }

    fn run_timers(&self) {
        let mut state = self.debounce.lock().expect("debounce lock");
        loop {
            if state.closed {
                return;
            }
            let now = Instant::now();
            let mut due: Vec<(Instant, WatchEvent)> = Vec::new();
            state.pending.retain(|_, (event, deadline)| {
                if *deadline <= now {
                    due.push((*deadline, event.clone()));
                    false
                } else {
                    true
                }
            });
            if !due.is_empty() {
                due.sort_by_key(|(deadline, _)| *deadline);
                drop(state);
                for (_, event) in due {
                    (self.on_event)(event);
                }
                state = self.debounce.lock().expect("debounce lock");
                continue;
            }
            let next = state.pending.values().map(|(_, deadline)| *deadline).min();
            state = match next {
                Some(deadline) => {
                    self.wake
                        .wait_timeout(state, deadline.saturating_duration_since(now))
                        .expect("debounce lock")
                        .0
                }
                None => self.wake.wait(state).expect("debounce lock"),
            };
        }
    }

    fn report(&self, target: &str, error: anyhow::Error) {
        if let Some(on_error) = &self.on_error {
            on_error(target, error);
        }
    }
}

pub struct RootWatcher {
    shared: Arc<Shared>,
    recursive_roots: bool,
}

/// Maps a path notify reports back under `target` as the caller spelled it.
fn rebase(target: &str, canonical: Option<&str>, reported: &str) -> String {
    let target = target.trim_end_matches(SEP);
    for prefix in std::iter::once(target).chain(canonical) {
        let prefix = prefix.trim_end_matches(SEP);
        if let Some(rest) = reported.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with(SEP))
        {
            return format!("{target}{rest}");
        }
    }
    reported.to_string()
}

impl RootWatcher {
    fn attach(&self, target: &str, recursive: bool) -> bool {
        let shared = &self.shared;
        if shared
            .watchers
            .lock()
            .expect("watchers lock")
            .contains_key(target)
        {
            return true;
        }
        let canonical = std::fs::canonicalize(target)
            .ok()
            .and_then(|path| path.to_str().map(str::to_string));
        let handler_shared = Arc::downgrade(shared);
        let handler_target = target.to_string();
        let handler = move |result: notify::Result<notify::Event>| {
            let Some(shared) = handler_shared.upgrade() else {
                return;
            };
            match result {
                Ok(event) => {
                    if matches!(event.kind, EventKind::Access(_)) {
                        return;
                    }
                    for path in &event.paths {
                        let Some(reported) = path.to_str() else {
                            continue;
                        };
                        let full_path = rebase(&handler_target, canonical.as_deref(), reported);
                        if let Some(classified) = classify_path(&full_path) {
                            shared.schedule(classified);
                        }
                    }
                }
                Err(error) => {
                    let (removed, empty) = {
                        let mut watchers = shared.watchers.lock().expect("watchers lock");
                        (watchers.remove(&handler_target), watchers.is_empty())
                    };
                    if empty {
                        shared.supported.store(false, Ordering::SeqCst);
                    }
                    if let Some(watcher) = removed {
                        std::thread::spawn(move || drop(watcher));
                    }
                    shared.report(&handler_target, anyhow!(error));
                }
            }
        };
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        let attached = notify::recommended_watcher(handler).and_then(|mut watcher| {
            watcher.watch(std::path::Path::new(target), mode)?;
            Ok(watcher)
        });
        match attached {
            Ok(watcher) => {
                shared
                    .watchers
                    .lock()
                    .expect("watchers lock")
                    .insert(target.to_string(), watcher);
                true
            }
            Err(error) => {
                shared.report(target, anyhow!(error));
                false
            }
        }
    }

    /// Whether at least one watch is live.
    pub fn supported(&self) -> bool {
        self.shared.supported.load(Ordering::SeqCst)
    }

    /// Follow one `.astroshot` tree (no-op when roots are watched recursively).
    pub fn watch_tree(&self, astroshot_dir: &str) {
        if self.recursive_roots {
            return;
        }
        if self.attach(astroshot_dir, true) {
            self.shared.supported.store(true, Ordering::SeqCst);
        }
        // A sibling `.astroshot` appearing next to a known tree shows up too.
        self.attach(&dirname(astroshot_dir), false);
    }

    pub fn close(&self) {
        {
            let mut state = self.shared.debounce.lock().expect("debounce lock");
            state.closed = true;
            state.pending.clear();
            self.shared.wake.notify_all();
        }
        // Drop the watchers outside the lock: dropping joins notify's thread,
        // which may be waiting on that lock.
        let watchers = std::mem::take(&mut *self.shared.watchers.lock().expect("watchers lock"));
        drop(watchers);
    }
}

impl Drop for RootWatcher {
    fn drop(&mut self) {
        self.close();
    }
}

pub fn watch_roots(
    roots: &[String],
    on_event: impl Fn(WatchEvent) + Send + Sync + 'static,
    options: WatchOptions,
) -> RootWatcher {
    let recursive_roots = options
        .recursive_roots
        .unwrap_or(cfg!(any(target_os = "macos", target_os = "windows")));
    let shared = Arc::new(Shared {
        settle: Duration::from_millis(options.settle_ms.unwrap_or(DEFAULT_SETTLE_MS)),
        on_event: Box::new(on_event),
        on_error: options.on_error,
        debounce: Mutex::new(Debounce {
            pending: HashMap::new(),
            closed: false,
        }),
        wake: Condvar::new(),
        watchers: Mutex::new(HashMap::new()),
        supported: AtomicBool::new(true),
    });
    let timers = Arc::downgrade(&shared);
    std::thread::spawn(move || {
        if let Some(shared) = timers.upgrade() {
            shared.run_timers();
        }
    });
    let watcher = RootWatcher {
        shared,
        recursive_roots,
    };
    let attached = roots
        .iter()
        .filter(|root| watcher.attach(root, recursive_roots))
        .count();
    watcher
        .shared
        .supported
        .store(attached > 0, Ordering::SeqCst);
    watcher
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn routes_images_sidecars_videos_friction_logs_and_trees() {
        assert_eq!(
            classify_path("/w/.astroshot/f/0001-a.png"),
            Some(WatchEvent::Shot {
                path: "/w/.astroshot/f/0001-a.png".into(),
                feature_dir: "/w/.astroshot/f".into(),
                astroshot_dir: "/w/.astroshot".into(),
            })
        );
        assert!(matches!(
            classify_path("/w/.astroshot/f/manifest.json"),
            Some(WatchEvent::Feature { feature_dir, .. }) if feature_dir == "/w/.astroshot/f"
        ));
        assert!(matches!(
            classify_path("/w/.astroshot/f/0001-a.webm"),
            Some(WatchEvent::Feature { .. })
        ));
        assert_eq!(
            classify_path("/w/.astroshot/friction-logs/x/runs/1/log.jsonl"),
            Some(WatchEvent::Friction {
                astroshot_dir: "/w/.astroshot".into()
            })
        );
        assert_eq!(
            classify_path("/w/.astroshot"),
            Some(WatchEvent::Tree {
                astroshot_dir: "/w/.astroshot".into()
            })
        );
        assert!(matches!(
            classify_path("/w/.astroshot/f"),
            Some(WatchEvent::Feature { .. })
        ));
        assert_eq!(classify_path("/w/src/index.ts"), None);
        assert_eq!(classify_path("/w/.astroshot/f/notes.txt"), None);
        assert_eq!(classify_path("/w/.astroshot/f/deep/0001-a.png"), None);
    }

    #[test]
    fn keys_events_per_subject() {
        assert_eq!(
            event_key(&WatchEvent::Friction {
                astroshot_dir: "/w/.astroshot".into()
            }),
            "friction:/w/.astroshot"
        );
    }

    #[test]
    fn ignores_hidden_feature_directories() {
        assert_eq!(classify_path("/w/.astroshot/.tmp/0001-a.png"), None);
    }

    #[test]
    fn rebases_canonical_paths_under_the_target_as_given() {
        assert_eq!(
            rebase(
                "/var/x",
                Some("/private/var/x"),
                "/private/var/x/.astroshot/f"
            ),
            "/var/x/.astroshot/f"
        );
        assert_eq!(rebase("/var/x", None, "/var/x"), "/var/x");
        assert_eq!(rebase("/var/x", None, "/var/xy/z"), "/var/xy/z");
    }

    #[test]
    fn debounces_a_burst_into_one_shot_event_under_the_given_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let root_path = root.path().to_str().expect("utf8").to_string();
        let (sender, receiver) = mpsc::channel();
        let watcher = watch_roots(
            std::slice::from_ref(&root_path),
            move |event| {
                let _ = sender.send(event);
            },
            WatchOptions {
                settle_ms: Some(100),
                recursive_roots: Some(true),
                ..WatchOptions::default()
            },
        );
        assert!(watcher.supported());
        let feature = root.path().join(".astroshot").join("f");
        std::fs::create_dir_all(&feature).expect("mkdir");
        let image = feature.join("0001-a.png");
        for round in 0..3 {
            std::fs::write(&image, [round]).expect("write");
        }
        let image = format!("{root_path}/.astroshot/f/0001-a.png");
        let mut shots = 0;
        let mut saw_shot = false;
        while let Ok(event) = receiver.recv_timeout(Duration::from_secs(5)) {
            if let WatchEvent::Shot { path, .. } = &event {
                assert_eq!(path, &image);
                shots += 1;
                saw_shot = true;
            }
            if saw_shot {
                // Drain anything already due, then stop.
                std::thread::sleep(Duration::from_millis(400));
                while let Ok(extra) = receiver.try_recv() {
                    if matches!(extra, WatchEvent::Shot { .. }) {
                        shots += 1;
                    }
                }
                break;
            }
        }
        assert!(saw_shot, "no shot event arrived");
        assert_eq!(shots, 1, "burst should debounce to one shot event");
        watcher.close();
    }

    #[test]
    fn reports_an_unwatchable_root_and_is_unsupported() {
        let (sender, receiver) = mpsc::channel();
        let missing = "/definitely/not/a/real/root-astroshot".to_string();
        let watcher = watch_roots(
            std::slice::from_ref(&missing),
            |_| {},
            WatchOptions {
                on_error: Some(Arc::new(move |root, _error| {
                    let _ = sender.send(root.to_string());
                })),
                ..WatchOptions::default()
            },
        );
        assert_eq!(receiver.try_recv().expect("on_error called"), missing);
        assert!(!watcher.supported());
    }
}
