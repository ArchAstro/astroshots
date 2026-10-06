//! Port of `packages/astroshot-review/src/data/hash-cache.ts`.
//!
//! Only the `HashRecord` type is declared here so `index_cache` can persist
//! it; the rest of the module is ported separately.

use serde::{Deserialize, Serialize};

use crate::movie_harness::types::js_number;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashRecord {
    #[serde(with = "js_number")]
    pub mtime_ms: f64,
    #[serde(with = "js_number")]
    pub size: f64,
    pub sha256: String,
}
