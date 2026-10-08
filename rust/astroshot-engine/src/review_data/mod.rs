//! Port of the engine-side half of `packages/astroshot-review/src/data`: the
//! `.astroshot/` contract readers and writers (manifests, `review.json`,
//! user stories, hashing, the tree scan). The tray's live store, watcher and
//! index cache are in the `astroshot-review` crate.
pub mod friction;
pub mod hash_cache;
pub mod manifest;
pub mod model;
pub mod paths;
pub mod review_store;
pub mod scan;
