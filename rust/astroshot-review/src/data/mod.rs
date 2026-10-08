//! Port of the tray-side half of `packages/astroshot-review/src/data`: the
//! live store, the filesystem watcher and the on-disk index. The readers and
//! writers of the `.astroshot/` contract are `astroshot_engine::review_data`.
pub mod index_cache;
pub mod store;
pub mod watcher;
