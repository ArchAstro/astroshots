//! Port of `packages/react-shot/src/batch-paths.ts`.
//!
//! Divergence: the TS key is `.normalize("NFC").toLowerCase()`. No Unicode
//! normalization crate is available, so only the lowercase step runs; paths
//! that differ solely by NFC/NFD composition are not detected as collisions.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};

use super::types::BatchEntry;

/// Lexical `path.resolve(base, rel)` for POSIX paths.
pub(crate) fn resolve(base: &Path, rel: &str) -> PathBuf {
    let joined = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        let base = if base.is_absolute() {
            base.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("/"))
                .join(base)
        };
        base.join(rel)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn collision_key(file_path: &Path) -> String {
    // Keep manifests portable across case-sensitive and case-insensitive hosts.
    let mut existing_ancestor = resolve(Path::new("/"), &file_path.to_string_lossy());
    let mut missing_segments: Vec<std::ffi::OsString> = Vec::new();
    while !existing_ancestor.exists() {
        let Some(parent) = existing_ancestor.parent().map(Path::to_path_buf) else {
            break;
        };
        if let Some(name) = existing_ancestor.file_name() {
            missing_segments.insert(0, name.to_os_string());
        }
        existing_ancestor = parent;
    }
    let mut canonical = std::fs::canonicalize(&existing_ancestor).unwrap_or(existing_ancestor);
    for segment in missing_segments {
        canonical.push(segment);
    }
    canonical.to_string_lossy().to_lowercase()
}

/// Resolve every destination before capture and reject accidental overwrites.
pub fn resolve_batch_output_paths(
    entries: &[BatchEntry],
    manifest_directory: &str,
) -> Result<Vec<String>> {
    let destinations: Vec<String> = entries
        .iter()
        .map(|entry| {
            resolve(Path::new(manifest_directory), &entry.out)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let mut seen: HashMap<String, &str> = HashMap::new();
    for destination in &destinations {
        let key = collision_key(Path::new(destination));
        if let Some(previous) = seen.get(&key) {
            bail!("Batch outputs resolve to the same destination: {previous} and {destination}");
        }
        seen.insert(key, destination);
    }
    Ok(destinations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(fixture: &str, out: &str) -> BatchEntry {
        BatchEntry {
            fixture: fixture.to_string(),
            out: out.to_string(),
            root: None,
            config: None,
            width: None,
            height: None,
        }
    }

    #[test]
    fn resolves_distinct_manifest_outputs() {
        let resolved = resolve_batch_output_paths(
            &[
                entry("one.tsx", "first/screen.png"),
                entry("two.tsx", "second/screen.png"),
            ],
            "/shots",
        )
        .unwrap();
        assert_eq!(
            resolved,
            vec!["/shots/first/screen.png", "/shots/second/screen.png"]
        );
    }

    #[test]
    fn rejects_normalized_duplicate_destinations() {
        let err = resolve_batch_output_paths(
            &[
                entry("one.tsx", "result/screen.png"),
                entry("two.tsx", "result/nested/../screen.png"),
            ],
            "/shots",
        )
        .unwrap_err();
        assert!(err.to_string().contains("same destination"));
    }

    #[test]
    fn rejects_case_only_destination_collisions_on_every_platform() {
        let err = resolve_batch_output_paths(
            &[
                entry("one.tsx", "result/screen.png"),
                entry("two.tsx", "RESULT/SCREEN.PNG"),
            ],
            "/shots",
        )
        .unwrap_err();
        assert!(err.to_string().contains("same destination"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_destinations_aliased_through_a_symlinked_directory() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        let alias = root.path().join("alias");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let err = resolve_batch_output_paths(
            &[
                entry("one.tsx", "real/screen.png"),
                entry("two.tsx", "alias/screen.png"),
            ],
            &root.path().to_string_lossy(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("same destination"));
    }
}
