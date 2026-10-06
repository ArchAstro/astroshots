//! Port of `packages/astroshot/bin/mac-preferences.mjs`.
//!
//! Read-only access to the Astroshots macOS app's real preferences.
//!
//! Two traps make the obvious implementations wrong:
//!
//! 1. Whole-domain plist to JSON conversion can never work: the domain holds
//!    AppKit's `NSOSPLastRootDirectory` CFData bookmark, which JSON cannot
//!    represent, so `plutil -convert json` aborts. Extract one key at a time.
//! 2. `~/Library/Preferences/<domain>.plist` is a lazily flushed cache of
//!    cfprefsd state. Go through cfprefsd (`defaults export`) first and treat
//!    the file as a fallback only.
//!
//! Nothing here writes: `astroshot doctor` must not mutate app state.
//!
//! Divergences from the TS: platform strings follow `std::env::consts::OS`
//! (`"macos"`, not `"darwin"`); spawn-error text is the Rust `io::Error`
//! message rather than Node's; watch-root input is `&[String]` (the TS
//! skipped non-strings, which the type now rules out).

use std::fs;
use std::io::Write;
use std::path::{MAIN_SEPARATOR, MAIN_SEPARATOR_STR, Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

pub const ASTROSHOTS_DOMAIN: &str = "ai.archastro.Astroshots";

// Resolve Apple's tools by absolute path: a minimal or empty PATH must never
// be mistaken for "this user has no watch roots".
const DEFAULTS_BIN: &str = "/usr/bin/defaults";
const PLUTIL_BIN: &str = "/usr/bin/plutil";

const MACOS: &str = "macos";

fn current_platform() -> String {
    std::env::consts::OS.to_string()
}

fn default_home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Matches `MISSING_KEY_PATTERN` (ASCII case-insensitive).
fn is_missing_key_message(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "no value at that key path",
        "invalid key path",
        "invalid object in plist",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsAvailability {
    pub available: bool,
    pub missing: Vec<String>,
}

/// Whether the tools this module shells out to are actually present.
/// `platform` defaults to the host (`std::env::consts::OS`).
pub fn preference_tools_available(platform: Option<&str>) -> ToolsAvailability {
    let platform = platform.map_or_else(current_platform, str::to_string);
    if platform != MACOS {
        return ToolsAvailability {
            available: false,
            missing: Vec::new(),
        };
    }
    let missing: Vec<String> = [DEFAULTS_BIN, PLUTIL_BIN]
        .iter()
        .filter(|binary| !Path::new(binary).exists())
        .map(|binary| (*binary).to_string())
        .collect();
    ToolsAvailability {
        available: missing.is_empty(),
        missing,
    }
}

#[derive(Debug, Clone, Default)]
pub struct PreferenceOptions {
    pub home: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreferenceDomain {
    pub available: bool,
    /// `"cfprefsd"` or `"plist-file"`.
    pub source: Option<String>,
    pub reason: Option<String>,
    pub plist: Vec<u8>,
    pub plist_path: Option<String>,
    pub stale_risk: bool,
    pub error: Option<String>,
}

fn unavailable_domain(reason: &str) -> PreferenceDomain {
    PreferenceDomain {
        reason: Some(reason.to_string()),
        ..Default::default()
    }
}

/// Snapshot the preference domain as an XML plist.
///
/// `defaults export` goes through cfprefsd, so it observes what the app itself
/// sees. The on-disk plist is only used when cfprefsd is unavailable.
pub fn read_preference_domain(domain: &str, options: &PreferenceOptions) -> PreferenceDomain {
    let platform = options.platform.clone().unwrap_or_else(current_platform);
    if platform != MACOS {
        return unavailable_domain("not-macos");
    }
    let home = options.home.clone().unwrap_or_else(default_home);

    let exported = Command::new(DEFAULTS_BIN)
        .args(["export", domain, "-"])
        .stdin(Stdio::null())
        .output();
    if let Ok(output) = &exported
        && output.status.code() == Some(0)
        && !output.stdout.is_empty()
    {
        return PreferenceDomain {
            available: true,
            source: Some("cfprefsd".into()),
            plist: output.stdout.clone(),
            ..Default::default()
        };
    }

    let plist_path = Path::new(&home)
        .join("Library")
        .join("Preferences")
        .join(format!("{domain}.plist"));
    match fs::read(&plist_path) {
        Ok(bytes) => PreferenceDomain {
            available: true,
            source: Some("plist-file".into()),
            plist: bytes,
            plist_path: Some(plist_path.to_string_lossy().into_owned()),
            stale_risk: true,
            ..Default::default()
        },
        Err(_) => match exported {
            // `defaults` failing to launch is a tool failure, not evidence
            // that the domain is absent.
            Err(error) => PreferenceDomain {
                reason: Some("tool-unavailable".into()),
                error: Some(error.to_string()),
                ..Default::default()
            },
            Ok(output) => {
                let missing = output.status.code() == Some(1) && output.stdout.is_empty();
                unavailable_domain(if missing {
                    "domain-not-found"
                } else {
                    "unreadable"
                })
            }
        },
    }
}

/// Outcome of extracting one key. Three deliberately distinct states:
/// present, genuinely absent (`!present && !failed`), and tool failure
/// (`failed`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedKey {
    pub present: bool,
    pub raw: Option<String>,
    pub failed: bool,
    pub error: Option<String>,
}

/// Extract a single preference key from a plist snapshot with
/// `plutil -extract <key> <format> -o - -`. `format` is `"raw"` or `"json"`.
///
/// A missing key exits non-zero with the same "Invalid object" wording as a
/// real failure, so absence is matched explicitly. Everything else (plutil
/// missing, spawn error, unrecognized non-zero exit) is a tool failure.
pub fn extract_preference_key(plist: &[u8], key: &str, format: &str) -> ExtractedKey {
    let failure = |error: String| ExtractedKey {
        failed: true,
        error: Some(error),
        ..Default::default()
    };
    let mut child = match Command::new(PLUTIL_BIN)
        .args(["-extract", key, format, "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return failure(error.to_string()),
    };
    // Feed stdin from a thread so a large plist cannot deadlock against the
    // child filling its stdout pipe.
    let stdin = child.stdin.take();
    let input = plist.to_vec();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            // A broken pipe means plutil exited early; its status reports why.
            let _ = stdin.write_all(&input);
        }
    });
    let output = child.wait_with_output();
    let _ = writer.join();
    let output = match output {
        Ok(output) => output,
        Err(error) => return failure(error.to_string()),
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() != Some(0) {
        if is_missing_key_message(&stderr) {
            return ExtractedKey::default();
        }
        let trimmed = stderr.trim();
        return failure(if trimmed.is_empty() {
            match output.status.code() {
                Some(code) => format!("plutil exited {code}"),
                None => "plutil exited null".to_string(),
            }
        } else {
            trimmed.to_string()
        });
    }
    ExtractedKey {
        present: true,
        raw: Some(String::from_utf8_lossy(&output.stdout).into_owned()),
        ..Default::default()
    }
}

#[derive(Debug, Default)]
struct Extracted<T> {
    present: bool,
    failed: bool,
    error: Option<String>,
    value: Option<T>,
}

impl<T> Extracted<T> {
    fn absent() -> Self {
        Self {
            present: false,
            failed: false,
            error: None,
            value: None,
        }
    }
    fn found(value: T) -> Self {
        Self {
            present: true,
            failed: false,
            error: None,
            value: Some(value),
        }
    }
}

fn carry_failure<T>(extracted: &ExtractedKey) -> Extracted<T> {
    Extracted {
        present: false,
        failed: extracted.failed,
        error: extracted.error.clone(),
        value: None,
    }
}

fn extract_string_array(plist: &[u8], key: &str) -> Extracted<Vec<String>> {
    let extracted = extract_preference_key(plist, key, "json");
    if !extracted.present {
        return carry_failure(&extracted);
    }
    let raw = extracted.raw.unwrap_or_default();
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(serde_json::Value::Array(entries)) => Extracted::found(
            entries
                .into_iter()
                .filter_map(|entry| match entry {
                    serde_json::Value::String(s) => Some(s),
                    _ => None,
                })
                .collect(),
        ),
        Ok(_) => Extracted::absent(),
        // The key exists but did not decode: unreadable, not absent.
        Err(error) => Extracted {
            failed: true,
            error: Some(error.to_string()),
            ..Extracted::absent()
        },
    }
}

fn extract_string(plist: &[u8], key: &str) -> Extracted<String> {
    let extracted = extract_preference_key(plist, key, "raw");
    if !extracted.present {
        return carry_failure(&extracted);
    }
    let raw = extracted.raw.unwrap_or_default();
    // `replace(/\n$/, "")`: drop one trailing newline.
    let value = raw.strip_suffix('\n').unwrap_or(&raw).to_string();
    Extracted::found(value)
}

fn extract_boolean(plist: &[u8], key: &str) -> Extracted<bool> {
    let extracted = extract_string(plist, key);
    if !extracted.present {
        return Extracted {
            failed: extracted.failed,
            ..Extracted::absent()
        };
    }
    let value = extracted.value.unwrap_or_default();
    let truthy = matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes");
    Extracted::found(truthy)
}

/// Mirror of `Preferences.normalizeWatchRootPaths` in the Swift app: expand
/// tildes, standardize, resolve symlinks, drop duplicates, and drop roots
/// already covered recursively by an earlier root. `home` defaults to the
/// user's home directory.
pub fn normalize_watch_root_paths(paths: &[String], home: Option<&str>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for raw_path in paths {
        if raw_path.is_empty() {
            continue;
        }
        let candidate = normalize_path(raw_path, home);
        if result.iter().any(|root| path_is_inside(root, &candidate)) {
            continue;
        }
        result.retain(|root| !path_is_inside(&candidate, root));
        result.push(candidate);
    }
    result
}

/// Lexical `path.normalize` for an absolute or relative string path.
fn normalize_lexical(path: &str) -> String {
    let absolute = path.starts_with(MAIN_SEPARATOR);
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(MAIN_SEPARATOR) {
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
    let joined = parts.join(MAIN_SEPARATOR_STR);
    if absolute {
        format!("{MAIN_SEPARATOR}{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Lexical `path.resolve(p)`.
fn resolve_absolute(path: &str) -> String {
    if path.starts_with(MAIN_SEPARATOR) {
        normalize_lexical(path)
    } else {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| MAIN_SEPARATOR_STR.to_string());
        normalize_lexical(&format!("{cwd}{MAIN_SEPARATOR}{path}"))
    }
}

fn realpath(path: &str) -> Option<String> {
    fs::canonicalize(PathBuf::from(path))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Expand `~`, make absolute, and resolve symlinks as far as the path exists.
pub fn normalize_path(raw_path: &str, home: Option<&str>) -> String {
    let home = home.map_or_else(default_home, str::to_string);
    let expanded = if raw_path == "~" {
        home
    } else if let Some(rest) = raw_path.strip_prefix("~/") {
        normalize_lexical(&format!("{home}{MAIN_SEPARATOR}{rest}"))
    } else {
        raw_path.to_string()
    };
    let absolute = resolve_absolute(&expanded);
    if let Some(real) = realpath(&absolute) {
        return real;
    }
    // Resolve the deepest existing ancestor so a missing leaf still normalizes.
    let parts: Vec<&str> = absolute.split(MAIN_SEPARATOR).collect();
    let mut depth = parts.len().saturating_sub(1);
    while depth > 1 {
        let ancestor = parts[..depth].join(MAIN_SEPARATOR_STR);
        if let Some(real) = realpath(&ancestor) {
            let tail = parts[depth..].join(MAIN_SEPARATOR_STR);
            return normalize_lexical(&format!("{real}{MAIN_SEPARATOR}{tail}"));
        }
        depth -= 1;
    }
    absolute
}

/// Containment on path-component boundaries, so `/Users/x/proj-two` is not
/// treated as living inside `/Users/x/proj`.
pub fn path_is_inside(root: &str, candidate: &str) -> bool {
    if root == candidate {
        return true;
    }
    if root.ends_with(MAIN_SEPARATOR) {
        candidate.starts_with(root)
    } else {
        candidate.starts_with(&format!("{root}{MAIN_SEPARATOR}"))
    }
}

/// The app's live watch configuration. Field names serialize camelCase and
/// absent optionals are omitted, matching the TS object shapes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchConfiguration {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_risk: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plist_path: Option<String>,
    pub roots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_roots: Option<Vec<String>>,
    pub used_legacy_key: bool,
    pub has_completed_first_run_setup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_configured_watch_roots: Option<bool>,
}

pub type ReadDomainFn<'a> = &'a dyn Fn(&str, &PreferenceOptions) -> PreferenceDomain;

#[derive(Default)]
pub struct ReadWatchConfigurationOptions<'a> {
    pub domain: Option<String>,
    pub home: Option<String>,
    pub platform: Option<String>,
    /// Seam for tests: supply a plist snapshot (including a corrupt one)
    /// without depending on the host's `/usr/bin`.
    pub read_domain: Option<ReadDomainFn<'a>>,
}

/// `watchRoots` (string array) is authoritative; the legacy singular
/// `watchRoot` is still honored when `watchRoots` was never written so
/// upgrades keep the folder the user already chose.
pub fn read_watch_configuration(options: &ReadWatchConfigurationOptions) -> WatchConfiguration {
    let domain = options.domain.as_deref().unwrap_or(ASTROSHOTS_DOMAIN);
    let home = options.home.clone().unwrap_or_else(default_home);
    let platform = options.platform.clone().unwrap_or_else(current_platform);
    let preference_options = PreferenceOptions {
        home: Some(home.clone()),
        platform: Some(platform),
    };
    let snapshot = match options.read_domain {
        Some(read) => read(domain, &preference_options),
        None => read_preference_domain(domain, &preference_options),
    };
    if !snapshot.available {
        return WatchConfiguration {
            reason: snapshot.reason,
            ..Default::default()
        };
    }

    let modern = extract_string_array(&snapshot.plist, "watchRoots");
    let legacy = if modern.present {
        Extracted::absent()
    } else {
        extract_string(&snapshot.plist, "watchRoot")
    };
    let first_run = extract_boolean(&snapshot.plist, "hasCompletedFirstRunSetup");

    // If the extraction tool could not answer, we know nothing about this
    // user's setup; surface the unreadable state, never "no watch roots".
    let failure = if modern.failed {
        Some(modern.error.clone())
    } else if legacy.failed {
        Some(legacy.error.clone())
    } else if first_run.failed {
        Some(first_run.error.clone())
    } else {
        None
    };
    if let Some(error) = failure {
        return WatchConfiguration {
            reason: Some("tool-unavailable".into()),
            error,
            source: snapshot.source,
            ..Default::default()
        };
    }

    let stored: Vec<String> = if modern.present {
        modern.value.clone().unwrap_or_default()
    } else if legacy.present && legacy.value.as_deref().is_some_and(|v| !v.is_empty()) {
        vec![legacy.value.clone().unwrap_or_default()]
    } else {
        Vec::new()
    };

    WatchConfiguration {
        available: true,
        source: snapshot.source,
        stale_risk: Some(snapshot.stale_risk),
        plist_path: snapshot.plist_path,
        roots: normalize_watch_root_paths(&stored, Some(&home)),
        stored_roots: Some(stored),
        used_legacy_key: !modern.present && legacy.present,
        has_completed_first_run_setup: first_run.present && first_run.value.unwrap_or(false),
        has_configured_watch_roots: Some(modern.present || legacy.present),
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageState {
    SetupIncomplete,
    OutsideRoots,
    InsideRoot,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchCoverage {
    pub state: CoverageState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub roots: Vec<String>,
}

/// Classify a project directory against the app's watch roots. Each outcome
/// needs a different remediation, so they stay distinct:
/// - `SetupIncomplete`: first-run folder setup never finished
/// - `OutsideRoots`: setup finished, but this project is not covered
/// - `InsideRoot`: covered by `matched_root`
/// - `Unknown`: the configuration could not be read at all; never claim
///   anything about the user's setup here
/// - `Unsupported`: not macOS
pub fn evaluate_watch_coverage(
    project_path: &str,
    configuration: &WatchConfiguration,
    home: Option<&str>,
) -> WatchCoverage {
    if !configuration.available {
        return WatchCoverage {
            state: if configuration.reason.as_deref() == Some("not-macos") {
                CoverageState::Unsupported
            } else {
                CoverageState::Unknown
            },
            reason: configuration.reason.clone(),
            error: configuration.error.clone(),
            matched_root: None,
            project_path: None,
            roots: Vec::new(),
        };
    }
    let normalized_project = normalize_path(project_path, home);
    let matched_root = configuration
        .roots
        .iter()
        .find(|root| path_is_inside(root, &normalized_project))
        .cloned();
    let (state, matched_root) = if matched_root.is_some() {
        (CoverageState::InsideRoot, matched_root)
    } else if !configuration.has_completed_first_run_setup || configuration.roots.is_empty() {
        (CoverageState::SetupIncomplete, None)
    } else {
        (CoverageState::OutsideRoots, None)
    };
    WatchCoverage {
        state,
        reason: None,
        error: None,
        matched_root,
        project_path: Some(normalized_project),
        roots: configuration.roots.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/tester";

    fn on_macos() -> bool {
        cfg!(target_os = "macos")
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>watchRoots</key>
  <array><string>/Users/tester/watched</string></array>
  <key>watchRoot</key>
  <string>/Users/tester/watched</string>
  <key>hasCompletedFirstRunSetup</key>
  <true/>
  <key>NSOSPLastRootDirectory</key>
  <data>YWJjZA==</data>
</dict>
</plist>
"#;

    #[test]
    fn watch_root_normalization_matches_the_apps_rules() {
        assert_eq!(
            normalize_watch_root_paths(
                &strings(&["~/proj", "/Users/tester/proj", "/Users/tester/proj/inner"]),
                Some(HOME)
            ),
            strings(&["/Users/tester/proj"]),
            "duplicates and covered children collapse into the parent root"
        );
        assert_eq!(
            normalize_watch_root_paths(
                &strings(&["/Users/tester/proj/inner", "/Users/tester/proj"]),
                Some(HOME)
            ),
            strings(&["/Users/tester/proj"]),
            "a later parent replaces an already-covered child"
        );
        assert!(normalize_watch_root_paths(&strings(&[""]), Some(HOME)).is_empty());

        // Component-boundary matching: proj-two is not inside proj.
        assert!(path_is_inside(
            "/Users/tester/proj",
            "/Users/tester/proj/app"
        ));
        assert!(path_is_inside("/Users/tester/proj", "/Users/tester/proj"));
        assert!(!path_is_inside(
            "/Users/tester/proj",
            "/Users/tester/proj-two"
        ));
        assert!(path_is_inside("/", "/Users/tester"));
    }

    #[test]
    fn normalize_path_expands_tilde_and_resolves_missing_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(dir.path()).unwrap();
        let home = home.to_string_lossy().into_owned();
        assert_eq!(normalize_path("~", Some(&home)), home);
        // Missing leaf under an existing ancestor still resolves the ancestor.
        assert_eq!(
            normalize_path("~/nope/deeper/../x", Some(&home)),
            format!("{home}/nope/x")
        );
        // Symlinked ancestors are resolved even when the leaf is missing.
        #[cfg(unix)]
        {
            let real = dir.path().join("real");
            fs::create_dir(&real).unwrap();
            std::os::unix::fs::symlink(&real, dir.path().join("link")).unwrap();
            let expected = format!("{}/missing", fs::canonicalize(&real).unwrap().display());
            assert_eq!(
                normalize_path(&format!("{home}/link/missing"), Some(&home)),
                expected
            );
        }
        // Fully nonexistent absolute path falls back to the lexical form.
        assert_eq!(
            normalize_path("/no-such-root-xyz/a/./b", Some(&home)),
            "/no-such-root-xyz/a/b"
        );
    }

    fn config(available: bool, reason: Option<&str>) -> WatchConfiguration {
        WatchConfiguration {
            available,
            reason: reason.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn watch_coverage_keeps_setup_incomplete_distinct_from_outside_roots() {
        let unavailable = config(false, Some("domain-not-found"));
        assert_eq!(
            evaluate_watch_coverage("/Users/tester/proj", &unavailable, Some(HOME)).state,
            CoverageState::Unknown
        );
        assert_eq!(
            evaluate_watch_coverage(
                "/Users/tester/proj",
                &config(false, Some("not-macos")),
                Some(HOME)
            )
            .state,
            CoverageState::Unsupported
        );

        let never_set_up = config(true, None);
        assert_eq!(
            evaluate_watch_coverage("/Users/tester/proj", &never_set_up, Some(HOME)).state,
            CoverageState::SetupIncomplete
        );

        let configured = WatchConfiguration {
            available: true,
            roots: strings(&["/Users/tester/watched"]),
            has_completed_first_run_setup: true,
            ..Default::default()
        };
        assert_eq!(
            evaluate_watch_coverage("/Users/tester/elsewhere", &configured, Some(HOME)).state,
            CoverageState::OutsideRoots
        );
        let inside = evaluate_watch_coverage("/Users/tester/watched/app", &configured, Some(HOME));
        assert_eq!(inside.state, CoverageState::InsideRoot);
        assert_eq!(
            inside.matched_root.as_deref(),
            Some("/Users/tester/watched")
        );
        assert_eq!(
            evaluate_watch_coverage("/Users/tester/watched-two", &configured, Some(HOME)).state,
            CoverageState::OutsideRoots,
            "sibling paths sharing a prefix must not read as watched"
        );
    }

    #[test]
    fn coverage_carries_reason_and_error_when_unavailable() {
        let mut unavailable = config(false, Some("tool-unavailable"));
        unavailable.error = Some("spawn failed".into());
        let coverage = evaluate_watch_coverage("/x", &unavailable, Some(HOME));
        assert_eq!(coverage.state, CoverageState::Unknown);
        assert_eq!(coverage.reason.as_deref(), Some("tool-unavailable"));
        assert_eq!(coverage.error.as_deref(), Some("spawn failed"));
        assert!(coverage.roots.is_empty());
    }

    #[test]
    fn a_missing_preference_key_is_absence_not_a_failure() {
        if !on_macos() {
            return;
        }
        let roots = extract_preference_key(PLIST.as_bytes(), "watchRoots", "json");
        assert!(roots.present);
        let parsed: Vec<String> = serde_json::from_str(roots.raw.as_deref().unwrap()).unwrap();
        assert_eq!(parsed, strings(&["/Users/tester/watched"]));

        // Scalars need raw, and CFData in the domain must not break per-key reads.
        assert_eq!(
            extract_preference_key(PLIST.as_bytes(), "watchRoot", "raw")
                .raw
                .unwrap()
                .trim(),
            "/Users/tester/watched"
        );
        assert_eq!(
            extract_preference_key(PLIST.as_bytes(), "hasCompletedFirstRunSetup", "raw")
                .raw
                .unwrap()
                .trim(),
            "true"
        );

        let absent = extract_preference_key(PLIST.as_bytes(), "watchRootsNope", "json");
        assert!(!absent.present);
        assert!(!absent.failed);
        assert_eq!(absent.error, None, "absence must not surface as an error");
    }

    #[test]
    fn an_unreadable_plist_is_a_tool_failure_not_an_absent_key() {
        if !on_macos() {
            return;
        }
        let corrupt = extract_preference_key(b"this is not a plist", "watchRoots", "json");
        assert!(!corrupt.present);
        assert!(corrupt.failed, "a parse failure must be flagged as failed");
        assert!(corrupt.error.is_some());

        let empty = extract_preference_key(b"", "watchRoots", "json");
        assert!(!empty.present);
        assert!(empty.failed);
    }

    #[test]
    fn the_macos_preference_tools_are_resolved_by_absolute_path() {
        let tools = preference_tools_available(Some("linux"));
        assert!(!tools.available);
        assert!(tools.missing.is_empty());
        if !on_macos() {
            return;
        }
        let tools = preference_tools_available(Some("macos"));
        assert!(
            tools.available,
            "/usr/bin/defaults and /usr/bin/plutil must exist"
        );
        assert!(tools.missing.is_empty());
    }

    #[test]
    fn read_preference_domain_reports_not_macos() {
        let result = read_preference_domain(
            ASTROSHOTS_DOMAIN,
            &PreferenceOptions {
                home: Some(HOME.into()),
                platform: Some("linux".into()),
            },
        );
        assert!(!result.available);
        assert_eq!(result.reason.as_deref(), Some("not-macos"));
        assert_eq!(result.source, None);
    }

    #[test]
    fn read_preference_domain_falls_back_to_the_plist_file() {
        if !on_macos() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let prefs = home.path().join("Library/Preferences");
        fs::create_dir_all(&prefs).unwrap();
        let domain = "ai.archastro.AstroshotsPortTestDomainThatDoesNotExist";
        fs::write(prefs.join(format!("{domain}.plist")), PLIST).unwrap();
        let options = PreferenceOptions {
            home: Some(home.path().to_string_lossy().into_owned()),
            platform: Some("macos".into()),
        };
        let snapshot = read_preference_domain(domain, &options);
        assert!(snapshot.available);
        // Newer macOS answers `defaults export` with an empty dict even for an
        // unknown domain, so cfprefsd may legitimately win over the file.
        match snapshot.source.as_deref() {
            Some("plist-file") => {
                assert!(snapshot.stale_risk);
                assert_eq!(snapshot.plist, PLIST.as_bytes());
            }
            other => assert_eq!(other, Some("cfprefsd")),
        }

        let missing = read_preference_domain(
            "ai.archastro.AstroshotsPortTestMissing",
            &PreferenceOptions {
                home: Some(home.path().join("none").to_string_lossy().into_owned()),
                platform: Some("macos".into()),
            },
        );
        if missing.available {
            assert_eq!(missing.source.as_deref(), Some("cfprefsd"));
        } else {
            assert!(matches!(
                missing.reason.as_deref(),
                Some("domain-not-found" | "unreadable")
            ));
        }
    }

    #[test]
    fn unavailable_domains_become_unavailable_configurations() {
        let read = |_: &str, _: &PreferenceOptions| unavailable_domain("not-macos");
        let configuration = read_watch_configuration(&ReadWatchConfigurationOptions {
            home: Some(HOME.into()),
            platform: Some("linux".into()),
            read_domain: Some(&read),
            ..Default::default()
        });
        assert!(!configuration.available);
        assert_eq!(configuration.reason.as_deref(), Some("not-macos"));
        assert_eq!(configuration.source, None);
        assert!(configuration.roots.is_empty());
        assert!(!configuration.has_completed_first_run_setup);
        assert!(!configuration.used_legacy_key);
    }

    fn snapshot_of(plist: &str) -> impl Fn(&str, &PreferenceOptions) -> PreferenceDomain {
        let plist = plist.as_bytes().to_vec();
        move |_, _| PreferenceDomain {
            available: true,
            source: Some("plist-file".into()),
            plist: plist.clone(),
            plist_path: Some("/Users/tester/Library/Preferences/x.plist".into()),
            stale_risk: true,
            ..Default::default()
        }
    }

    #[test]
    fn an_unreadable_snapshot_never_becomes_an_empty_watch_configuration() {
        if !on_macos() {
            return;
        }
        // Exercises the real extraction path: a snapshot plutil cannot parse
        // must surface as unreadable, not as "zero watch roots".
        let read = snapshot_of("this is not a plist");
        let configuration = read_watch_configuration(&ReadWatchConfigurationOptions {
            home: Some(HOME.into()),
            platform: Some("macos".into()),
            read_domain: Some(&read),
            ..Default::default()
        });
        assert!(
            !configuration.available,
            "an unparseable snapshot must not report a readable, empty configuration"
        );
        assert_eq!(configuration.reason.as_deref(), Some("tool-unavailable"));
        assert!(configuration.roots.is_empty());
        assert!(configuration.error.is_some());

        let coverage = evaluate_watch_coverage("/Users/tester/proj", &configuration, Some(HOME));
        assert_eq!(
            coverage.state,
            CoverageState::Unknown,
            "unreadable configuration must classify as unknown, never setup-incomplete"
        );
    }

    #[test]
    fn watch_roots_are_authoritative_and_the_legacy_key_is_a_fallback() {
        if !on_macos() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(home.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let watched = format!("{home}/watched");
        fs::create_dir(&watched).unwrap();
        let modern = PLIST.replace("/Users/tester/watched", &watched);
        let read = snapshot_of(&modern);
        let configuration = read_watch_configuration(&ReadWatchConfigurationOptions {
            home: Some(home.clone()),
            platform: Some("macos".into()),
            read_domain: Some(&read),
            ..Default::default()
        });
        assert!(configuration.available);
        assert_eq!(configuration.roots, vec![watched.clone()]);
        assert!(!configuration.used_legacy_key);
        assert!(configuration.has_completed_first_run_setup);
        assert_eq!(configuration.has_configured_watch_roots, Some(true));
        assert_eq!(configuration.stale_risk, Some(true));
        assert_eq!(configuration.source.as_deref(), Some("plist-file"));

        let legacy_only = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>watchRoot</key><string>WATCHED</string>
</dict></plist>
"#
        .replace("WATCHED", &watched);
        let read = snapshot_of(&legacy_only);
        let configuration = read_watch_configuration(&ReadWatchConfigurationOptions {
            home: Some(home.clone()),
            platform: Some("macos".into()),
            read_domain: Some(&read),
            ..Default::default()
        });
        assert!(configuration.available);
        assert!(configuration.used_legacy_key);
        assert_eq!(configuration.roots, vec![watched]);
        assert!(!configuration.has_completed_first_run_setup);

        let empty = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict></dict></plist>
"#;
        let read = snapshot_of(empty);
        let configuration = read_watch_configuration(&ReadWatchConfigurationOptions {
            home: Some(home),
            platform: Some("macos".into()),
            read_domain: Some(&read),
            ..Default::default()
        });
        assert!(configuration.available);
        assert!(configuration.roots.is_empty());
        assert_eq!(configuration.has_configured_watch_roots, Some(false));
        assert!(!configuration.used_legacy_key);
    }

    #[test]
    fn missing_key_messages_match_case_insensitively() {
        assert!(is_missing_key_message(
            "Could not extract value: No value at that key path"
        ));
        assert!(is_missing_key_message("INVALID KEY PATH"));
        assert!(is_missing_key_message(
            "Invalid object in plist for JSON format"
        ));
        assert!(!is_missing_key_message("something else failed"));
    }
}
