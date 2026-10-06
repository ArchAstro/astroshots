//! Port of `packages/astroshot-review/src/data/store.ts`: types first.
//!
//! Declares `StoreState` and the observable surface of `ReviewStore`
//! (`get_state`, `subscribe`, `publish_with`) so `ui::context` and `ui::hooks`
//! compile against it. The scanning, indexing and watching logic is ported
//! with `store.ts` itself and extends this file.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::astroshot_review::data::model::{FrictionLog, Shot};

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

/// `type Listener = () => void`.
pub type Listener = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    state: Arc<StoreState>,
    listeners: BTreeMap<u64, Listener>,
    next_listener: u64,
}

pub struct ReviewStore {
    inner: Mutex<Inner>,
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

impl ReviewStore {
    pub fn new(roots: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                state: Arc::new(StoreState::initial(roots)),
                listeners: BTreeMap::new(),
                next_listener: 0,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
}
