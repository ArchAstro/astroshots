//! Port of `packages/tui-shot/src/batch-paths.ts`.

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

use anyhow::{Result, bail};

use super::types::BatchEntry;

/// Lexical `path.posix.normalize`.
fn posix_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut joined = parts.join("/");
    if joined.is_empty() && !absolute {
        joined = ".".to_string();
    }
    if !joined.is_empty() && trailing && joined != "." {
        joined.push('/');
    } else if joined == "." && trailing {
        joined = "./".to_string();
    }
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// `path.win32.isAbsolute`.
fn win32_is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    match bytes.first() {
        Some(b'/' | b'\\') => true,
        _ => {
            bytes.len() > 2
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'/' | b'\\')
        }
    }
}

/// Lexical POSIX `path.resolve(base, rel)`.
fn resolve(base: &str, rel: &str) -> String {
    let joined = if rel.starts_with('/') {
        rel.to_string()
    } else if base.starts_with('/') {
        format!("{base}/{rel}")
    } else {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".to_string());
        format!("{cwd}/{base}/{rel}")
    };
    let normalized = posix_normalize(&joined);
    if normalized.len() > 1 {
        normalized.trim_end_matches('/').to_string()
    } else {
        normalized
    }
}

fn collision_key(file_path: &str) -> String {
    // Reject case-only collisions on every platform so a manifest behaves the
    // same on case-sensitive and case-insensitive filesystems.
    resolve("/", file_path)
        .nfc()
        .collect::<String>()
        .to_lowercase()
}

fn safe_relative_output(output: &str) -> Result<String> {
    let portable = output.replace('\\', "/");
    let normalized = posix_normalize(&portable);
    if normalized.starts_with('/')
        || win32_is_absolute(output)
        || normalized == ".."
        || normalized.starts_with("../")
    {
        bail!("Batch output must be a safe relative path when --out-dir is used: {output}");
    }
    Ok(normalized)
}

/// Resolve batch destinations once, rejecting traversal and overwrite risks.
pub fn resolve_batch_output_paths(
    entries: &[BatchEntry],
    manifest_dir: &str,
    out_dir: Option<&str>,
) -> Result<Vec<String>> {
    let mut destinations = Vec::with_capacity(entries.len());
    for entry in entries {
        destinations.push(match out_dir {
            // TS treats an empty `outDir` string as unset (`outDir ? ... : ...`).
            Some(out_dir) if !out_dir.is_empty() => {
                resolve(out_dir, &safe_relative_output(&entry.out)?)
            }
            _ => resolve(manifest_dir, &entry.out),
        });
    }
    let mut seen: HashMap<String, &str> = HashMap::new();
    for destination in &destinations {
        let key = collision_key(destination);
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
        }
    }

    #[test]
    fn preserves_safe_nested_output_paths_under_out_dir() {
        let resolved = resolve_batch_output_paths(
            &[
                entry("one.tsx", "first/screen.png"),
                entry("two.tsx", "second/screen.png"),
            ],
            "/project/shots",
            Some("/artifacts"),
        )
        .unwrap();
        assert_eq!(
            resolved,
            vec![
                "/artifacts/first/screen.png",
                "/artifacts/second/screen.png"
            ]
        );
    }

    #[test]
    fn rejects_unsafe_out_dir_paths() {
        for output in [
            "../escape.png",
            "nested/../../escape.png",
            "/absolute.png",
            "C:\\absolute.png",
            "..\\escape.png",
        ] {
            let err = resolve_batch_output_paths(
                &[entry("one.tsx", output)],
                "/project/shots",
                Some("/artifacts"),
            )
            .unwrap_err();
            assert!(
                err.to_string().contains("safe relative path"),
                "{output}: {err}"
            );
        }
    }

    #[test]
    fn rejects_normalized_and_case_only_destination_collisions() {
        let err = resolve_batch_output_paths(
            &[
                entry("one.tsx", "flow/screen.png"),
                entry("two.tsx", "FLOW/../flow/SCREEN.png"),
            ],
            "/project/shots",
            Some("/artifacts"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("same destination"));
    }

    #[test]
    fn rejects_composed_and_decomposed_unicode_duplicates() {
        // "café" with a precomposed é versus e + combining acute.
        let err = resolve_batch_output_paths(
            &[
                entry("one.tsx", "caf\u{e9}.png"),
                entry("two.tsx", "cafe\u{301}.png"),
            ],
            "/artifacts",
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("same destination"));
    }
}
