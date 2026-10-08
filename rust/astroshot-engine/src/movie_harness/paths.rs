//! Path helpers for `.astroshot/<feature>` layout.
//!
//! Port of `packages/movie-harness/src/paths.ts`.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{SecondsFormat, Utc};

/// Error text is part of the CLI contract (reaches stderr).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathsError {
    #[error("{label} must be kebab-case [a-z0-9-]+, got {got}")]
    NotKebabCase { label: String, got: String },
}

/// Lexical `path.resolve`: join onto the cwd when relative, then collapse
/// `.` and `..` without touching the filesystem.
pub fn resolve_lexically(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
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

/// JS `String#trim` whitespace (differs from `str::trim` on U+0085 / U+FEFF).
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// `root` if non-empty, else the git toplevel of the cwd, else the cwd.
pub fn resolve_root(root: Option<&str>) -> String {
    if let Some(root) = root.filter(|root| !root.is_empty()) {
        return resolve_lexically(Path::new(root))
            .to_string_lossy()
            .into_owned();
    }
    // execFileSync leaves stderr attached to the parent's.
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim_matches(is_js_whitespace)
            .to_string(),
        _ => std::env::current_dir()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

fn is_kebab_case(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

pub fn assert_kebab_case(name: &str, label: &str) -> Result<(), PathsError> {
    if !is_kebab_case(name) {
        return Err(PathsError::NotKebabCase {
            label: label.to_string(),
            got: json_string(name),
        });
    }
    Ok(())
}

pub fn assert_slug(slug: &str) -> Result<(), PathsError> {
    assert_kebab_case(slug, "slug")
}

pub fn feature_dir(root: &str, feature: &str) -> String {
    join_js(&[root, ".astroshot", feature])
}

pub fn movie_state_dir(root: &str, feature: &str) -> String {
    join_js(&[&feature_dir(root, feature), ".movie"])
}

/// `path.join`: concatenate with `/` and normalize (empty segments skipped).
fn join_js(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        return ".".to_string();
    }
    let absolute = joined.starts_with('/');
    let trailing = joined.ends_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if stack.last().is_some_and(|last| *last != "..") {
                    stack.pop();
                } else if !absolute {
                    stack.push("..");
                }
            }
            other => stack.push(other),
        }
    }
    let mut out = stack.join("/");
    if absolute {
        out.insert(0, '/');
    } else if out.is_empty() {
        out.push('.');
    }
    if trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

pub fn default_run_id(feature: &str) -> String {
    // toISOString with `-`/`:` removed and the milliseconds dropped.
    let stamp = Utc::now()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
        .replace(['-', ':'], "");
    format!("{feature}-{stamp}-{}", std::process::id())
}

pub fn humanize(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn ensure_dir(dir: &str) -> io::Result<()> {
    fs::create_dir_all(dir)
}

/// Next `NNNN` sequence number for files named `NNNN-*` in the directory.
pub fn next_sequence(feature_directory: &str) -> io::Result<String> {
    let mut max: u64 = 0;
    if Path::new(feature_directory).exists() {
        for entry in fs::read_dir(feature_directory)? {
            let name = entry?.file_name();
            let Some(name) = name.to_str() else { continue };
            let bytes = name.as_bytes();
            if bytes.len() >= 5 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-' {
                max = max.max(name[..4].parse::<u64>().unwrap_or(0));
            }
        }
    }
    Ok(format!("{:04}", max + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_case_validation_matches_ts_messages() {
        assert!(assert_kebab_case("my-feature2", "feature").is_ok());
        assert_eq!(
            assert_kebab_case("Bad_Name", "feature")
                .unwrap_err()
                .to_string(),
            "feature must be kebab-case [a-z0-9-]+, got \"Bad_Name\""
        );
        assert!(assert_kebab_case("-x", "feature").is_err());
        assert!(assert_kebab_case("", "feature").is_err());
        assert!(assert_kebab_case("a\n", "feature").is_err());
        assert_eq!(
            assert_slug("No Good").unwrap_err().to_string(),
            "slug must be kebab-case [a-z0-9-]+, got \"No Good\""
        );
    }

    #[test]
    fn feature_and_state_dirs() {
        assert_eq!(feature_dir("/r", "f"), "/r/.astroshot/f");
        assert_eq!(movie_state_dir("/r", "f"), "/r/.astroshot/f/.movie");
    }

    #[test]
    fn humanize_title_cases_words() {
        assert_eq!(humanize("hello-big_world"), "Hello Big World");
        assert_eq!(humanize("--a__b--"), "A B");
        assert_eq!(humanize(""), "");
    }

    #[test]
    fn default_run_id_shape() {
        let id = default_run_id("feat");
        let re = regex::Regex::new(r"^feat-\d{8}T\d{6}Z-\d+$").unwrap();
        assert!(re.is_match(&id), "{id}");
        assert!(id.ends_with(&std::process::id().to_string()));
    }

    #[test]
    fn next_sequence_counts_from_highest_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        assert_eq!(next_sequence(path).unwrap(), "0001");
        fs::write(dir.path().join("0003-a.png"), "").unwrap();
        fs::write(dir.path().join("0001-b.png"), "").unwrap();
        fs::write(dir.path().join("12345-no.png"), "").unwrap();
        fs::write(dir.path().join("notes.md"), "").unwrap();
        assert_eq!(next_sequence(path).unwrap(), "0004");
        assert_eq!(
            next_sequence(dir.path().join("missing").to_str().unwrap()).unwrap(),
            "0001"
        );
    }

    #[test]
    fn ensure_dir_creates_nested() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        ensure_dir(nested.to_str().unwrap()).unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn resolve_root_resolves_explicit_root_lexically() {
        assert_eq!(resolve_root(Some("/a/b/../c/./d")), "/a/c/d");
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            resolve_root(Some("x/../y")),
            cwd.join("y").to_string_lossy()
        );
        assert!(!resolve_root(None).is_empty());
    }
}
