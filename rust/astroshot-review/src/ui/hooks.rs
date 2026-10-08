//! Port of `packages/astroshot-review/src/ui/hooks.ts`.
//!
//! React hooks become small state structs the app state owns. An effect that
//! re-rendered on a subscription or timer becomes a [`Wake`] callback: the
//! hook calls it from wherever the change happened, and the event loop (the
//! `tokio::select!` in the app) turns it into a redraw message.
//!
//! - `useStoreState` -> [`StoreStateHook`] (subscribes to the store; `state()`
//!   is `useSyncExternalStore`'s snapshot).
//! - `useTerminalSize` -> [`TerminalSizeHook`]; the event loop calls
//!   `on_resize` for each crossterm `Event::Resize`.
//! - `useClock` -> [`ClockHook`]; a tokio interval task refreshes `now()` and
//!   wakes the loop.
//!
//! Dropping a hook is its effect cleanup.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

use crate::data::store::{StoreState, Subscription};
use crate::ui::context::AppServices;

/// Called when hook state changed and the screen needs a redraw.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// `useStoreState()`.
pub struct StoreStateHook {
    services: AppServices,
    changed: Arc<AtomicBool>,
    _subscription: Subscription,
}

impl StoreStateHook {
    pub fn new(services: &AppServices, wake: Wake) -> Self {
        let changed = Arc::new(AtomicBool::new(false));
        let flag = changed.clone();
        let subscription = services.store.subscribe(Arc::new(move || {
            flag.store(true, Ordering::SeqCst);
            wake();
        }));
        Self {
            services: services.clone(),
            changed,
            _subscription: subscription,
        }
    }

    /// The current store snapshot.
    pub fn state(&self) -> Arc<StoreState> {
        self.services.store.get_state()
    }

    /// True once per publish burst since the last call.
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
}

impl TerminalSize {
    /// `stdout.columns || 80`, `stdout.rows || 24`: zero or unknown falls back.
    pub fn from_reported(columns: Option<u16>, rows: Option<u16>) -> Self {
        Self {
            columns: columns.filter(|c| *c != 0).unwrap_or(80),
            rows: rows.filter(|r| *r != 0).unwrap_or(24),
        }
    }

    /// The real terminal's size (80x24 when it cannot be read).
    pub fn current() -> Self {
        match crossterm::terminal::size() {
            Ok((columns, rows)) => Self::from_reported(Some(columns), Some(rows)),
            Err(_) => Self::from_reported(None, None),
        }
    }
}

/// `useTerminalSize()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSizeHook {
    size: TerminalSize,
}

impl TerminalSizeHook {
    pub fn new(size: TerminalSize) -> Self {
        Self { size }
    }

    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Handle `Event::Resize(columns, rows)`; returns whether the size changed.
    pub fn on_resize(&mut self, columns: u16, rows: u16) -> bool {
        let next = TerminalSize::from_reported(Some(columns), Some(rows));
        let changed = next != self.size;
        self.size = next;
        changed
    }
}

pub type NowFn = Arc<dyn Fn() -> f64 + Send + Sync>;

fn system_now_ms() -> f64 {
    crate::ui::theme::now_ms()
}

/// `useClock(intervalMs)`: re-render every `interval` so relative times stay fresh.
/// Must be created inside a tokio runtime.
pub struct ClockHook {
    now: Arc<Mutex<f64>>,
    task: JoinHandle<()>,
}

impl ClockHook {
    pub fn new(interval: Duration, wake: Wake) -> Self {
        Self::with_now(interval, wake, Arc::new(system_now_ms))
    }

    pub fn with_now(interval: Duration, wake: Wake, now_fn: NowFn) -> Self {
        let now = Arc::new(Mutex::new(now_fn()));
        let shared = now.clone();
        // interval_at: the first tick is one period away, like setInterval.
        let mut ticker = interval_at(Instant::now() + interval, interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let task = tokio::spawn(async move {
            loop {
                ticker.tick().await;
                *shared.lock().unwrap_or_else(|e| e.into_inner()) = now_fn();
                wake();
            }
        });
        Self { now, task }
    }

    pub fn now(&self) -> f64 {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for ClockHook {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::probe::GraphicsProtocol;
    use crate::ui::context::test_support::{FakeService, services};
    use std::sync::atomic::AtomicUsize;

    fn counter_wake() -> (Wake, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        (
            Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
            count,
        )
    }

    #[tokio::test]
    async fn store_state_hook_tracks_published_snapshots_and_stops_after_drop() {
        let services = services(GraphicsProtocol::None, FakeService::new(false));
        let (wake, count) = counter_wake();
        let hook = StoreStateHook::new(&services, wake);
        assert_eq!(hook.state().revision, 0);
        assert!(!hook.take_changed());

        services.store.publish_with(|s| s.scanning = true);
        assert!(hook.take_changed());
        assert!(!hook.take_changed());
        assert_eq!((hook.state().revision, hook.state().scanning), (1, true));
        assert_eq!(count.load(Ordering::SeqCst), 1);

        drop(hook);
        services.store.publish_with(|s| s.scanning = false);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn terminal_size_falls_back_to_80_by_24_when_unreported_or_zero() {
        assert_eq!(
            TerminalSize::from_reported(None, None),
            TerminalSize {
                columns: 80,
                rows: 24
            }
        );
        assert_eq!(
            TerminalSize::from_reported(Some(0), Some(0)),
            TerminalSize {
                columns: 80,
                rows: 24
            }
        );
        assert_eq!(
            TerminalSize::from_reported(Some(120), Some(40)),
            TerminalSize {
                columns: 120,
                rows: 40
            }
        );
    }

    #[test]
    fn terminal_size_hook_updates_on_resize_and_reports_changes() {
        let mut hook = TerminalSizeHook::new(TerminalSize::from_reported(Some(100), Some(30)));
        assert!(!hook.on_resize(100, 30));
        assert!(hook.on_resize(90, 20));
        assert_eq!(
            hook.size(),
            TerminalSize {
                columns: 90,
                rows: 20
            }
        );
        assert!(hook.on_resize(0, 0));
        assert_eq!(
            hook.size(),
            TerminalSize {
                columns: 80,
                rows: 24
            }
        );
    }

    // tokio's `test-util` (paused clock) is not enabled for this crate, so these use short real intervals.
    #[tokio::test]
    async fn clock_hook_refreshes_now_each_interval_and_wakes_the_loop() {
        let ticks = Arc::new(Mutex::new(1000.0));
        let source = ticks.clone();
        let now_fn: NowFn = Arc::new(move || *source.lock().unwrap());
        let (wake, count) = counter_wake();
        let clock = ClockHook::with_now(Duration::from_millis(60), wake, now_fn);
        assert_eq!(clock.now(), 1000.0);

        // The first tick is a full period away (setInterval), not immediate.
        *ticks.lock().unwrap() = 31_000.0;
        assert_eq!((clock.now(), count.load(Ordering::SeqCst)), (1000.0, 0));
        for _ in 0..200 {
            if count.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(count.load(Ordering::SeqCst) >= 1);
        assert_eq!(clock.now(), 31_000.0);
    }

    #[tokio::test]
    async fn clock_hook_stops_ticking_once_dropped() {
        let (wake, count) = counter_wake();
        let clock = ClockHook::new(Duration::from_millis(20), wake);
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(clock);
        // Let an in-flight tick land, then confirm the count is frozen.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let frozen = count.load(Ordering::SeqCst);
        assert!(frozen >= 1);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(count.load(Ordering::SeqCst), frozen);
    }
}
