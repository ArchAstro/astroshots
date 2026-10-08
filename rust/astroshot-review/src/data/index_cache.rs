//! Port of `packages/astroshot-review/src/data/index-cache.ts`.
//!
//! Durable index so the tray opens instantly: known `.astroshot` directories,
//! the newest-first arrival order, and memoized image hashes.
//!
//! Divergence: `hashes` is a `BTreeMap`, so `index.json` lists hash records in
//! path order instead of TS insertion order. Readers (TS and Rust) are
//! order-insensitive, and nothing outside the review tray reads this cache.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use astroshot_engine::movie_harness::types::Version1;
use astroshot_engine::review_data::hash_cache::HashRecord;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDocument {
    pub version: Version1,
    pub roots: Vec<String>,
    pub astroshot_dirs: Vec<String>,
    pub arrival_order: Vec<String>,
    pub hashes: BTreeMap<String, HashRecord>,
    pub updated_at: String,
    /// When the last deep walk of every root finished (ISO), if ever.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_scan_at: Option<String>,
}

/// `indexCachePath(env)`: `env` looks a variable up (an unset or empty value
/// is falsy, as in JS).
pub fn index_cache_path(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(dir) = get("ASTROSHOT_REVIEW_CACHE_DIR") {
        return Path::new(&dir).join("index.json");
    }
    let base = get("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache")
    });
    base.join("astroshot-review").join("index.json")
}

/// `indexCachePath()` with the default `process.env`.
pub fn default_index_cache_path() -> PathBuf {
    index_cache_path(&|name| std::env::var(name).ok())
}

/// JS default `Array.prototype.sort` order (UTF-16 code units), on a copy.
fn normalized_roots(roots: &[String]) -> Vec<String> {
    let mut sorted = roots.to_vec();
    sorted.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    sorted
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

pub async fn load_index(roots: &[String], file_path: &Path) -> Option<IndexDocument> {
    let bytes = tokio::fs::read(file_path).await.ok()?;
    let Value::Object(parsed) = serde_json::from_slice::<Value>(&bytes).ok()? else {
        return None;
    };
    match parsed.get("version") {
        Some(Value::Number(version)) if version.as_f64() == Some(1.0) => {}
        _ => return None,
    }
    let Some(Value::Array(stored_roots)) = parsed.get("roots") else {
        return None;
    };
    // Non-string roots can never equal the string roots passed in.
    let stored_roots: Vec<String> = stored_roots
        .iter()
        .map(|root| root.as_str().map(str::to_string))
        .collect::<Option<_>>()?;
    if normalized_roots(&stored_roots) != normalized_roots(roots) {
        return None;
    }
    let hashes = match parsed.get("hashes") {
        Some(Value::Object(entries)) => entries
            .iter()
            .filter_map(|(path, record)| {
                HashRecord::deserialize(record)
                    .ok()
                    .map(|record| (path.clone(), record))
            })
            .collect(),
        _ => BTreeMap::new(),
    };
    Some(IndexDocument {
        version: Version1,
        roots: stored_roots,
        astroshot_dirs: string_list(parsed.get("astroshotDirs")),
        arrival_order: string_list(parsed.get("arrivalOrder")),
        hashes,
        updated_at: parsed
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        full_scan_at: parsed
            .get("fullScanAt")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub async fn save_index(document: &IndexDocument, file_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut temp = file_path.as_os_str().to_owned();
    temp.push(format!(".{}.tmp", std::process::id()));
    let temp = PathBuf::from(temp);
    let text = serde_json::to_string(document).map_err(std::io::Error::other)?;
    tokio::fs::write(&temp, text).await?;
    tokio::fs::rename(&temp, file_path).await
}

/// One shot's identity and capture time, as `reconcileArrivalOrder` reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrivalShot {
    pub path: String,
    pub captured_at: f64,
}

/// Keep known paths in their prior relative order; new paths go first, sorted
/// newest capture first. Paths that vanished are dropped.
pub fn reconcile_arrival_order(previous: &[String], shots: &[ArrivalShot]) -> Vec<String> {
    let present: HashSet<&str> = shots.iter().map(|shot| shot.path.as_str()).collect();
    let kept: Vec<String> = previous
        .iter()
        .filter(|entry| present.contains(entry.as_str()))
        .cloned()
        .collect();
    let known: HashSet<&str> = kept.iter().map(String::as_str).collect();
    let mut fresh: Vec<&ArrivalShot> = shots
        .iter()
        .filter(|shot| !known.contains(shot.path.as_str()))
        .collect();
    fresh.sort_by(|a, b| {
        b.captured_at
            .partial_cmp(&a.captured_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fresh
        .into_iter()
        .map(|shot| shot.path.clone())
        .chain(kept.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(path: &str, captured_at: f64) -> ArrivalShot {
        ArrivalShot {
            path: path.into(),
            captured_at,
        }
    }

    #[test]
    fn keeps_known_order_prepends_new_paths_newest_first_drops_vanished() {
        let previous: Vec<String> = ["/b", "/a", "/gone"].map(String::from).to_vec();
        let shots = [
            shot("/a", 1.0),
            shot("/b", 2.0),
            shot("/new-old", 5.0),
            shot("/new-new", 9.0),
        ];
        assert_eq!(
            reconcile_arrival_order(&previous, &shots),
            ["/new-new", "/new-old", "/b", "/a"]
        );
    }

    #[test]
    fn cache_path_honors_env_in_ts_order() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(
            index_cache_path(&env(&[
                ("ASTROSHOT_REVIEW_CACHE_DIR", "/c"),
                ("XDG_CACHE_HOME", "/x")
            ])),
            PathBuf::from("/c/index.json")
        );
        assert_eq!(
            index_cache_path(&env(&[
                ("ASTROSHOT_REVIEW_CACHE_DIR", ""),
                ("XDG_CACHE_HOME", "/x")
            ])),
            PathBuf::from("/x/astroshot-review/index.json")
        );
    }

    fn document(roots: &[&str]) -> IndexDocument {
        IndexDocument {
            version: Version1,
            roots: roots.iter().map(|root| root.to_string()).collect(),
            astroshot_dirs: vec!["/w/.astroshot".into()],
            arrival_order: vec!["/w/.astroshot/f/a.png".into()],
            hashes: BTreeMap::from([(
                "/w/.astroshot/f/a.png".to_string(),
                HashRecord {
                    mtime_ms: 1757094120000.0,
                    size: 12.0,
                    sha256: "ab".repeat(32),
                },
            )]),
            updated_at: "2026-09-05T17:42:00Z".into(),
            full_scan_at: None,
        }
    }

    #[tokio::test]
    async fn saves_compact_json_in_ts_key_order_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("index.json");
        let saved = document(&["/b", "/a"]);
        save_index(&saved, &file).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!(
                concat!(
                    r#"{{"version":1,"roots":["/b","/a"],"astroshotDirs":["/w/.astroshot"],"#,
                    r#""arrivalOrder":["/w/.astroshot/f/a.png"],"hashes":{{"/w/.astroshot/f/a.png":"#,
                    r#"{{"mtimeMs":1757094120000,"size":12,"sha256":"{}"}}}},"updatedAt":"2026-09-05T17:42:00Z"}}"#
                ),
                "ab".repeat(32)
            )
        );
        // Same root set in a different order still loads.
        let loaded = load_index(&["/a".to_string(), "/b".to_string()], &file)
            .await
            .unwrap();
        assert_eq!(loaded, saved);
        assert!(load_index(&["/a".to_string()], &file).await.is_none());
        assert!(
            load_index(
                &["/a".to_string(), "/b".to_string()],
                &dir.path().join("none")
            )
            .await
            .is_none()
        );
    }
}
