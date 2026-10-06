//! Port of `packages/astroshot-review/src/data/hash-cache.ts`.
//!
//! Memoizes file hashes by (path, mtime, size) so rescans stay cheap.
//!
//! Divergences: records live in a `BTreeMap` (matching `index_cache`), so
//! `to_json` is key-sorted rather than insertion-ordered; the `fs.Stats`
//! argument becomes [`FileStat`]; failures are `std::io::Error`s (waiters of a
//! shared in-flight hash get an error of the same kind and message).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use futures::FutureExt;
use futures::future::Shared;
use serde::{Deserialize, Serialize};

use super::review_store::sha256_file;
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

/// The slice of `fs.Stats` the cache keys on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileStat {
    pub mtime_ms: f64,
    pub size: f64,
}

impl FileStat {
    pub fn of(file_path: impl AsRef<Path>) -> io::Result<FileStat> {
        let metadata = std::fs::metadata(file_path)?;
        let mtime_ms = match metadata.modified()?.duration_since(UNIX_EPOCH) {
            Ok(elapsed) => elapsed.as_secs_f64() * 1000.0,
            Err(before) => -before.duration().as_secs_f64() * 1000.0,
        };
        Ok(FileStat {
            mtime_ms,
            size: metadata.len() as f64,
        })
    }
}

type SharedHash = Shared<Pin<Box<dyn Future<Output = Result<String, Arc<io::Error>>> + Send>>>;

#[derive(Default)]
struct Inner {
    records: BTreeMap<String, HashRecord>,
    in_flight: HashMap<String, SharedHash>,
}

fn lock(inner: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Cheap to clone; clones share the same records.
#[derive(Clone, Default)]
pub struct HashCache {
    inner: Arc<Mutex<Inner>>,
}

impl HashCache {
    pub fn new(initial: BTreeMap<String, HashRecord>) -> HashCache {
        HashCache {
            inner: Arc::new(Mutex::new(Inner {
                records: initial,
                in_flight: HashMap::new(),
            })),
        }
    }

    pub async fn hash(&self, file_path: &str, stat: Option<FileStat>) -> io::Result<String> {
        let info = match stat {
            Some(info) => info,
            None => {
                let path = file_path.to_string();
                tokio::task::spawn_blocking(move || FileStat::of(path))
                    .await
                    .map_err(io::Error::other)??
            }
        };
        let shared = {
            let mut inner = lock(&self.inner);
            if let Some(cached) = inner.records.get(file_path)
                && cached.mtime_ms == info.mtime_ms
                && cached.size == info.size
            {
                return Ok(cached.sha256.clone());
            }
            if let Some(pending) = inner.in_flight.get(file_path) {
                pending.clone()
            } else {
                let inner_arc = Arc::clone(&self.inner);
                let path = file_path.to_string();
                let job: Pin<Box<dyn Future<Output = Result<String, Arc<io::Error>>> + Send>> =
                    Box::pin(async move {
                        let result = sha256_file(&path).await;
                        let mut inner = lock(&inner_arc);
                        inner.in_flight.remove(&path);
                        match result {
                            Ok(sha256) => {
                                inner.records.insert(
                                    path,
                                    HashRecord {
                                        mtime_ms: info.mtime_ms,
                                        size: info.size,
                                        sha256: sha256.clone(),
                                    },
                                );
                                Ok(sha256)
                            }
                            Err(error) => Err(Arc::new(error)),
                        }
                    });
                let shared = job.shared();
                inner
                    .in_flight
                    .insert(file_path.to_string(), shared.clone());
                shared
            }
        };
        shared
            .await
            .map_err(|error| io::Error::new(error.kind(), error.to_string()))
    }

    /// Adds records for paths that have none; existing records win.
    pub fn seed(&self, records: BTreeMap<String, HashRecord>) {
        let mut inner = lock(&self.inner);
        for (file_path, record) in records {
            inner.records.entry(file_path).or_insert(record);
        }
    }

    /// Drop records for files that are no longer part of the stream.
    pub fn retain(&self, keep: &HashSet<String>, also_keep: impl Fn(&str) -> bool) {
        lock(&self.inner)
            .records
            .retain(|file_path, _| keep.contains(file_path) || also_keep(file_path));
    }

    pub fn to_json(&self) -> BTreeMap<String, HashRecord> {
        lock(&self.inner).records.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn path_str(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn hashes_a_file_with_sha256_and_records_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = path_str(&dir.path().join("a.png"));
        std::fs::write(&file, b"hello").unwrap();
        let cache = HashCache::default();
        let digest = cache.hash(&file, None).await.unwrap();
        assert_eq!(
            digest,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(digest, sha(b"hello"));
        let json = cache.to_json();
        assert_eq!(json[&file].sha256, digest);
        assert_eq!(json[&file].size, 5.0);
    }

    #[tokio::test]
    async fn reuses_the_record_while_mtime_and_size_match() {
        let dir = tempfile::tempdir().unwrap();
        let file = path_str(&dir.path().join("a.png"));
        std::fs::write(&file, b"hello").unwrap();
        let stat = FileStat::of(&file).unwrap();
        let cache = HashCache::new(BTreeMap::from([(
            file.clone(),
            HashRecord {
                mtime_ms: stat.mtime_ms,
                size: stat.size,
                sha256: "seeded".into(),
            },
        )]));
        // The stored digest is trusted: the file is not re-read.
        assert_eq!(cache.hash(&file, None).await.unwrap(), "seeded");
        assert_eq!(cache.hash(&file, Some(stat)).await.unwrap(), "seeded");
    }

    #[tokio::test]
    async fn rehashes_when_size_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file = path_str(&dir.path().join("a.png"));
        std::fs::write(&file, b"hello").unwrap();
        let cache = HashCache::default();
        let first = cache.hash(&file, None).await.unwrap();
        std::fs::write(&file, b"hello world").unwrap();
        let second = cache.hash(&file, None).await.unwrap();
        assert_ne!(first, second);
        assert_eq!(second, sha(b"hello world"));
        assert_eq!(cache.to_json()[&file].size, 11.0);
    }

    #[tokio::test]
    async fn rehashes_when_mtime_changes_but_size_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let file = path_str(&dir.path().join("a.png"));
        std::fs::write(&file, b"aaaa").unwrap();
        let cache = HashCache::default();
        let stat = FileStat::of(&file).unwrap();
        let first = cache.hash(&file, Some(stat)).await.unwrap();
        std::fs::write(&file, b"bbbb").unwrap();
        let newer = FileStat {
            mtime_ms: stat.mtime_ms + 5000.0,
            size: stat.size,
        };
        let second = cache.hash(&file, Some(newer)).await.unwrap();
        assert_ne!(first, second);
        assert_eq!(second, sha(b"bbbb"));
        assert_eq!(cache.to_json()[&file].mtime_ms, newer.mtime_ms);
    }

    #[tokio::test]
    async fn concurrent_hashes_of_one_file_share_a_single_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = path_str(&dir.path().join("big.bin"));
        let bytes = vec![7u8; 2 * 1024 * 1024];
        std::fs::write(&file, &bytes).unwrap();
        let cache = HashCache::default();
        let (a, b, c) = tokio::join!(
            cache.hash(&file, None),
            cache.hash(&file, None),
            cache.hash(&file, None)
        );
        let expected = sha(&bytes);
        assert_eq!(a.unwrap(), expected);
        assert_eq!(b.unwrap(), expected);
        assert_eq!(c.unwrap(), expected);
        assert!(lock(&cache.inner).in_flight.is_empty());
        assert_eq!(cache.to_json().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_hash_is_not_cached_and_clears_in_flight() {
        let dir = tempfile::tempdir().unwrap();
        let file = path_str(&dir.path().join("a.png"));
        let cache = HashCache::default();
        let stat = FileStat {
            mtime_ms: 1.0,
            size: 1.0,
        };
        let error = cache.hash(&file, Some(stat)).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(lock(&cache.inner).in_flight.is_empty());
        assert!(cache.to_json().is_empty());
        // A later attempt after the file appears succeeds.
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(cache.hash(&file, Some(stat)).await.unwrap(), sha(b"x"));
    }

    #[tokio::test]
    async fn stat_is_taken_from_disk_when_missing_and_errors_propagate() {
        let dir = tempfile::tempdir().unwrap();
        let file = path_str(&dir.path().join("missing"));
        let error = HashCache::default().hash(&file, None).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    fn record(sha256: &str) -> HashRecord {
        HashRecord {
            mtime_ms: 1.0,
            size: 2.0,
            sha256: sha256.into(),
        }
    }

    #[test]
    fn seed_only_fills_missing_paths() {
        let cache = HashCache::new(BTreeMap::from([("/a".to_string(), record("old"))]));
        cache.seed(BTreeMap::from([
            ("/a".to_string(), record("new")),
            ("/b".to_string(), record("b")),
        ]));
        let json = cache.to_json();
        assert_eq!(json["/a"].sha256, "old");
        assert_eq!(json["/b"].sha256, "b");
    }

    #[test]
    fn retain_drops_records_outside_the_stream_unless_also_kept() {
        let cache = HashCache::new(BTreeMap::from([
            ("/keep".to_string(), record("1")),
            ("/also".to_string(), record("2")),
            ("/drop".to_string(), record("3")),
        ]));
        let keep = HashSet::from(["/keep".to_string()]);
        cache.retain(&keep, |path| path == "/also");
        let json = cache.to_json();
        assert_eq!(json.keys().collect::<Vec<_>>(), vec!["/also", "/keep"]);
    }
}
