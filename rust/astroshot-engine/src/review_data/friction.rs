//! Port of `packages/astroshot-review/src/data/friction.ts`.
//!
//! User stories (formerly friction logs):
//! `.astroshot/stories/<slug>/{prompt.md,meta.json,runs/<run>/log.jsonl}`, plus the
//! legacy `.astroshot/friction-logs/<slug>/` tree with the same layout.
//! Mirrors the macOS loader, including field aliases and silent skipping of
//! malformed lines and missing screenshots. This module only reads.
//!
//! Divergences from the TS:
//! - The async loaders return `io::Result`; a failing `readFile` of an
//!   existing `log.jsonl` (for example a directory with that name) rejects in
//!   TS and is an `Err` here.
//! - `Date.parse` of `meta.updated_at` handles the ISO-8601 forms (`Z`/offset
//!   date-times, offset-less date-times as local time, date-only, `YYYY-MM`,
//!   `YYYY`), not V8's legacy free-form formats.
//! - `JSON.parse` is serde_json: lone surrogate escapes and nesting beyond 128
//!   levels fail to parse here.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeDelta, TimeZone, Timelike, Utc};
use serde_json::{Map, Value};

use super::hash_cache::HashCache;
use super::model::{FrictionLog, FrictionRun, FrictionStep, ReviewSnapshot};
use super::paths::{basename, friction_logs_dir, humanize, join, stories_dir, worktree_short};
use super::review_store::{
    entry_needs_hash, js_trim, read_review_document, scoped_entry, snapshot_from_entry,
};

pub const PROMPT_FILE: &str = "prompt.md";
pub const META_FILE: &str = "meta.json";
pub const RUNS_DIR: &str = "runs";
pub const LOG_FILE: &str = "log.jsonl";

/// The `{ worktreePath, worktree }` argument of `loadFrictionLog(s)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrictionContext {
    pub worktree_path: String,
    pub worktree: String,
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|entry| js_trim(entry).to_string())
            .filter(|entry| !entry.is_empty())
            .collect(),
        Some(Value::String(text)) if !js_trim(text).is_empty() => {
            vec![js_trim(text).to_string()]
        }
        _ => Vec::new(),
    }
}

fn first_string(record: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| record.get(*key).and_then(Value::as_str))
        .map(str::to_string)
}

/// `record[a] ?? record[b] ?? ...`: the first key whose value is not null.
fn first_present<'a>(record: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| record.get(*key).filter(|value| !value.is_null()))
}

async fn exists(file_path: &str) -> bool {
    tokio::fs::metadata(file_path).await.is_ok()
}

async fn is_directory_entry(parent: &str, entry: &tokio::fs::DirEntry) -> bool {
    let Ok(file_type) = entry.file_type().await else {
        return false;
    };
    if file_type.is_dir() {
        return true;
    }
    if !file_type.is_symlink() {
        return false;
    }
    match tokio::fs::metadata(join(parent, &entry.file_name().to_string_lossy())).await {
        Ok(metadata) => metadata.is_dir(),
        Err(_) => false,
    }
}

async fn mtime_of(file_path: &str) -> Option<f64> {
    let modified = tokio::fs::metadata(file_path).await.ok()?.modified().ok()?;
    Some(match modified.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64() * 1000.0,
        Err(before) => -before.duration().as_secs_f64() * 1000.0,
    })
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_millis() as f64)
}

/// `path.resolve(dir, name)` for a POSIX path.
fn resolve(dir: &str, name: &str) -> String {
    let base = if dir.starts_with('/') {
        dir.to_string()
    } else {
        let cwd = std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".to_string());
        join(&cwd, dir)
    };
    let joined = if name.starts_with('/') {
        join("/", name)
    } else {
        join(&base, name)
    };
    if joined.len() > 1 {
        joined.trim_end_matches('/').to_string()
    } else {
        joined
    }
}

/// `String(number)` for a JS number.
fn js_number_string(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs();
    if (1e-6..1e21).contains(&magnitude) {
        return format!("{value}");
    }
    let text = format!("{value:e}");
    match text.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => text,
    }
}

/// Parse JSONL text into ordered steps; screenshots are resolved against `run_dir`.
pub async fn parse_jsonl(text: &str, run_dir: &str) -> Vec<FrictionStep> {
    let mut steps: Vec<FrictionStep> = Vec::new();
    let mut index: u64 = 0;
    for raw_line in text.split('\n') {
        let line = js_trim(raw_line.strip_suffix('\r').unwrap_or(raw_line));
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        index += 1;
        let step = match parsed.get("step") {
            Some(Value::Number(number)) => number.as_f64().unwrap_or(index as f64),
            _ => index as f64,
        };
        let step_id = first_string(&parsed, &["id"])
            .unwrap_or_else(|| format!("step-{}", js_number_string(step)));
        let mut screenshots: Vec<String> = Vec::new();
        for name in string_list(first_present(&parsed, &["screenshots", "screenshot"])) {
            let by_basename = join(run_dir, basename(&name));
            let as_written = resolve(run_dir, &name);
            if exists(&by_basename).await {
                screenshots.push(by_basename);
            } else if exists(&as_written).await {
                screenshots.push(as_written);
            }
        }
        steps.push(FrictionStep {
            id: format!("{}-{step_id}", js_number_string(step)),
            step,
            title: first_string(&parsed, &["title"]).unwrap_or_else(|| humanize(&step_id)),
            description: first_string(&parsed, &["description"]).unwrap_or_default(),
            transcript: first_string(&parsed, &["transcript", "narration", "voiceover"])
                .unwrap_or_default(),
            step_id,
            screenshots,
            good: string_list(first_present(&parsed, &["good", "looks_good"])),
            improve: string_list(first_present(
                &parsed,
                &["improve", "can_improve", "improvements"],
            )),
            url: first_string(&parsed, &["url"]),
            captured_at: first_string(&parsed, &["captured_at"]),
        });
    }
    steps.sort_by(|a, b| a.step.total_cmp(&b.step));
    steps
}

async fn run_review(run_dir: &str, run_id: &str, hashes: &HashCache) -> Option<ReviewSnapshot> {
    let log_path = join(run_dir, LOG_FILE);
    let document = match read_review_document(run_dir).await {
        Ok(Some(document)) => document,
        Ok(None) | Err(_) => return None,
    };
    let entry = scoped_entry(&document, LOG_FILE, Some(run_id));
    let sha = if entry_needs_hash(entry) {
        hashes.hash(&log_path, None).await.ok()
    } else {
        None
    };
    Some(snapshot_from_entry(entry, sha.as_deref()))
}

fn is_image_name(name: &str) -> bool {
    // /\.(png|jpe?g|webp|gif)$/i
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp", ".gif"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

pub async fn load_run(
    run_dir: &str,
    run_id: &str,
    hashes: &HashCache,
) -> std::io::Result<Option<FrictionRun>> {
    let log_path = join(run_dir, LOG_FILE);
    let has_log = exists(&log_path).await;
    let mut steps: Vec<FrictionStep> = Vec::new();
    if has_log {
        let bytes = tokio::fs::read(&log_path).await?;
        steps = parse_jsonl(&String::from_utf8_lossy(&bytes), run_dir).await;
    } else {
        let Ok(mut reader) = tokio::fs::read_dir(run_dir).await else {
            return Ok(None);
        };
        let mut has_image = false;
        while let Ok(Some(entry)) = reader.next_entry().await {
            if is_image_name(&entry.file_name().to_string_lossy()) {
                has_image = true;
                break;
            }
        }
        if !has_image {
            return Ok(None);
        }
    }
    let captured_at = match mtime_of(run_dir).await {
        Some(value) => value,
        None => mtime_of(&log_path).await.unwrap_or_else(now_ms),
    };
    let review = if has_log {
        run_review(run_dir, run_id, hashes).await
    } else {
        None
    };
    Ok(Some(FrictionRun {
        run_id: run_id.to_string(),
        directory: run_dir.to_string(),
        log_path: has_log.then_some(log_path),
        captured_at,
        status: None,
        steps,
        review,
    }))
}

/// Visible subdirectory names of `dir`, in `fs.readdir` (byte-sorted) order.
async fn directory_names(dir: &str) -> Vec<String> {
    let Ok(mut reader) = tokio::fs::read_dir(dir).await else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = reader.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !is_directory_entry(dir, &entry).await {
            continue;
        }
        names.push(name);
    }
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    names
}

pub async fn load_runs(log_dir: &str, hashes: &HashCache) -> std::io::Result<Vec<FrictionRun>> {
    let runs_dir = join(log_dir, RUNS_DIR);
    let mut runs: Vec<FrictionRun> = Vec::new();
    for name in directory_names(&runs_dir).await {
        if let Some(run) = load_run(&join(&runs_dir, &name), &name, hashes).await? {
            runs.push(run);
        }
    }
    if runs.is_empty()
        && exists(&join(log_dir, LOG_FILE)).await
        && let Some(flat) = load_run(log_dir, "latest", hashes).await?
    {
        runs.push(flat);
    }
    runs.sort_by(|a, b| b.captured_at.total_cmp(&a.captured_at));
    Ok(runs)
}

/// `Date.parse` for the ISO-8601 forms.
fn parse_date_ms(text: &str) -> Option<f64> {
    let text = js_trim(text);
    if let Ok(date) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(date.timestamp_millis() as f64);
    }
    let utc = |date: NaiveDate| {
        Some(
            Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?)
                .timestamp_millis() as f64,
        )
    };
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return utc(date);
    }
    if text.len() == 7
        && let Ok(date) = NaiveDate::parse_from_str(&format!("{text}-01"), "%Y-%m-%d")
    {
        return utc(date);
    }
    if text.len() == 4
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && let Ok(date) = NaiveDate::parse_from_str(&format!("{text}-01-01"), "%Y-%m-%d")
    {
        return utc(date);
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|date| date.timestamp_millis() as f64);
        }
    }
    None
}

pub async fn load_friction_log(
    log_dir: &str,
    context: &FrictionContext,
    hashes: &HashCache,
) -> std::io::Result<Option<FrictionLog>> {
    let slug = basename(log_dir).to_string();
    let prompt_path = join(log_dir, PROMPT_FILE);
    let meta_path = join(log_dir, META_FILE);
    let has_prompt = exists(&prompt_path).await;
    let runs = load_runs(log_dir, hashes).await?;
    if !has_prompt && runs.is_empty() {
        return Ok(None);
    }

    let meta: Map<String, Value> = match tokio::fs::read(&meta_path).await {
        Ok(bytes) => match serde_json::from_str::<Value>(&String::from_utf8_lossy(&bytes)) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        },
        Err(_) => Map::new(),
    };
    let meta_title = first_string(&meta, &["title"]).map(|title| js_trim(&title).to_string());
    let mut candidates: Vec<f64> = vec![];
    candidates.extend(mtime_of(log_dir).await);
    if has_prompt {
        candidates.extend(mtime_of(&prompt_path).await);
    }
    candidates.extend(mtime_of(&meta_path).await);
    candidates.extend(runs.first().map(|run| run.captured_at));
    if let Some(Value::String(updated_at)) = meta.get("updated_at") {
        candidates.extend(parse_date_ms(updated_at));
    }
    candidates.retain(|value| value.is_finite());
    let updated_at = candidates
        .iter()
        .copied()
        .reduce(f64::max)
        .unwrap_or_else(now_ms);

    Ok(Some(FrictionLog {
        id: format!("{}::{slug}", context.worktree_path),
        title: match meta_title {
            Some(title) if !title.is_empty() => title,
            _ => humanize(&slug),
        },
        slug,
        directory: log_dir.to_string(),
        worktree_path: context.worktree_path.clone(),
        worktree: context.worktree.clone(),
        worktree_short: worktree_short(&context.worktree),
        description: first_string(&meta, &["description"]).unwrap_or_default(),
        status: first_string(&meta, &["status"])
            .or_else(|| runs.first().and_then(|run| run.status.clone())),
        updated_at,
        prompt_path: has_prompt.then_some(prompt_path),
        runs,
    }))
}

pub async fn load_friction_logs(
    friction_dir: &str,
    context: &FrictionContext,
    hashes: &HashCache,
) -> std::io::Result<Vec<FrictionLog>> {
    let mut logs: Vec<FrictionLog> = Vec::new();
    for name in directory_names(friction_dir).await {
        if let Some(log) = load_friction_log(&join(friction_dir, &name), context, hashes).await? {
            logs.push(log);
        }
    }
    logs.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
    Ok(logs)
}

/// Stories under `.astroshot/stories/` and the legacy `.astroshot/friction-logs/`.
/// A slug that loads from `stories/` is listed once, from `stories/`; the
/// legacy copy is ignored. Newest first.
pub async fn load_user_stories(
    astroshot_dir: &str,
    context: &FrictionContext,
    hashes: &HashCache,
) -> std::io::Result<Vec<FrictionLog>> {
    let mut logs = load_friction_logs(&stories_dir(astroshot_dir), context, hashes).await?;
    let legacy = load_friction_logs(&friction_logs_dir(astroshot_dir), context, hashes).await?;
    let legacy_only: Vec<FrictionLog> = legacy
        .into_iter()
        .filter(|old| !logs_contain_slug(&logs, &old.slug))
        .collect();
    logs.extend(legacy_only);
    logs.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
    Ok(logs)
}

fn logs_contain_slug(logs: &[FrictionLog], slug: &str) -> bool {
    logs.iter().any(|log| log.slug == slug)
}

/// `MMM d · HH:mm` in local time for `yyyyMMddTHHmmssZ(-N)` run ids.
pub fn run_display_title(run_id: &str) -> String {
    if let Some(fields) = parse_run_id(run_id) {
        let [year, month, day, hour, minute, second] = fields;
        // Date.UTC maps years 0..=99 to 1900..=1999 and carries overflowing fields.
        let year = if year <= 99 { year + 1900 } else { year };
        let months = year * 12 + (month - 1);
        if let Some(first) = NaiveDate::from_ymd_opt(
            months.div_euclid(12) as i32,
            (months.rem_euclid(12) + 1) as u32,
            1,
        ) {
            let naive = first.and_hms_opt(0, 0, 0).expect("midnight")
                + TimeDelta::days(day - 1)
                + TimeDelta::seconds(hour * 3600 + minute * 60 + second);
            let date = Local.from_utc_datetime(&naive);
            return format!(
                "{} {} · {:02}:{:02}",
                date.format("%b"),
                date.day(),
                date.hour(),
                date.minute()
            );
        }
    }
    let units: Vec<u16> = run_id.encode_utf16().collect();
    if units.len() > 16 {
        String::from_utf16_lossy(&units[units.len() - 14..])
    } else {
        run_id.to_string()
    }
}

/// `/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z(?:-\d+)?$/`
fn parse_run_id(run_id: &str) -> Option<[i64; 6]> {
    let bytes = run_id.as_bytes();
    if bytes.len() < 16 || bytes[8] != b'T' || bytes[15] != b'Z' {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = &bytes[range];
        part.iter().all(u8::is_ascii_digit).then(|| {
            part.iter()
                .fold(0, |acc, byte| acc * 10 + i64::from(byte - b'0'))
        })
    };
    let rest = &bytes[16..];
    let suffix_ok = rest.is_empty()
        || (rest[0] == b'-' && rest.len() > 1 && rest[1..].iter().all(u8::is_ascii_digit));
    if !suffix_ok {
        return None;
    }
    Some([
        digits(0..4)?,
        digits(4..6)?,
        digits(6..8)?,
        digits(9..11)?,
        digits(11..13)?,
        digits(13..15)?,
    ])
}

pub fn step_count_label(count: usize) -> String {
    if count == 1 {
        "1 step".to_string()
    } else {
        format!("{count} steps")
    }
}

pub fn friction_status_label(status: Option<&str>) -> Option<String> {
    match status.unwrap_or("").to_lowercase().as_str() {
        "draft" => Some("Draft".to_string()),
        "ready" => Some("Ready".to_string()),
        "running" => Some("Running".to_string()),
        "complete" | "completed" | "done" => Some("Complete".to_string()),
        "failed" | "fail" | "error" => Some("Failed".to_string()),
        _ => status.filter(|status| !status.is_empty()).map(humanize),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("friction-")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("0001-a.png"), "x").unwrap();
        dir
    }

    #[tokio::test]
    async fn honors_aliases_skips_comments_and_bad_lines_drops_missing_screenshots() {
        let dir = fixture();
        let dir_str = dir.path().to_str().unwrap();
        let text = [
            "# header".to_string(),
            String::new(),
            "not json".to_string(),
            serde_json::json!({ "title": "Auto", "screenshots": ["missing.png", "nested/0001-a.png"] }).to_string(),
            serde_json::json!({ "step": 2, "id": "two", "narration": "spoken", "screenshot": "0001-a.png", "looks_good": ["ok"], "improvements": [" fix ", ""] }).to_string(),
        ]
        .join("\n");
        let steps = parse_jsonl(&text, dir_str).await;
        assert_eq!(
            steps.iter().map(|step| step.step).collect::<Vec<_>>(),
            vec![1.0, 2.0]
        );
        let expected = vec![join(dir_str, "0001-a.png")];
        let two = &steps[1];
        assert_eq!(two.transcript, "spoken");
        assert_eq!(two.screenshots, expected);
        assert_eq!(two.good, vec!["ok"]);
        assert_eq!(two.improve, vec!["fix"]);
        let auto = &steps[0];
        assert_eq!(auto.step_id, "step-1");
        assert_eq!(auto.title, "Auto");
        assert_eq!(auto.screenshots, expected);
    }

    #[test]
    fn formats_run_titles_and_step_counts() {
        let long = Regex::new(r"^Aug 11 · \d{2}:\d{2}$").unwrap();
        assert!(long.is_match(&run_display_title("20260811T153000Z")));
        assert!(run_display_title("20260811T153000Z-2").starts_with("Aug 11 · "));
        assert_eq!(run_display_title("short"), "short");
        assert_eq!(
            run_display_title("a-very-long-run-identifier"),
            "run-identifier"
        );
        assert_eq!(step_count_label(1), "1 step");
        assert_eq!(step_count_label(3), "3 steps");
    }

    async fn write_story(astroshot: &std::path::Path, tree: &str, slug: &str, title: &str) {
        let story = astroshot.join(tree).join(slug);
        let run = story.join("runs/20260811T153000Z");
        tokio::fs::create_dir_all(&run).await.unwrap();
        tokio::fs::write(run.join("log.jsonl"), "{\"step\":1,\"id\":\"a\"}\n")
            .await
            .unwrap();
        tokio::fs::write(story.join("meta.json"), format!(r#"{{"title":"{title}"}}"#))
            .await
            .unwrap();
    }

    async fn load_stories(astroshot: &std::path::Path) -> Vec<FrictionLog> {
        let context = FrictionContext {
            worktree_path: "/w/wt7".into(),
            worktree: "wt7".into(),
        };
        load_user_stories(astroshot.to_str().unwrap(), &context, &HashCache::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn lists_a_story_that_exists_only_under_stories() {
        let dir = tempfile::tempdir().unwrap();
        write_story(dir.path(), "stories", "signup", "New").await;
        let logs = load_stories(dir.path()).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].title, "New");
        assert!(logs[0].directory.ends_with("stories/signup"));
        assert!(logs[0].runs[0].log_path.is_some());
    }

    #[tokio::test]
    async fn lists_a_story_that_exists_only_under_legacy_friction_logs() {
        let dir = tempfile::tempdir().unwrap();
        write_story(dir.path(), "friction-logs", "signup", "Old").await;
        let logs = load_stories(dir.path()).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].title, "Old");
        assert!(logs[0].directory.ends_with("friction-logs/signup"));
    }

    #[tokio::test]
    async fn takes_a_slug_present_in_both_trees_from_stories() {
        let dir = tempfile::tempdir().unwrap();
        write_story(dir.path(), "friction-logs", "signup", "Old").await;
        write_story(dir.path(), "friction-logs", "legacy-only", "Legacy only").await;
        write_story(dir.path(), "stories", "signup", "New").await;
        let logs = load_stories(dir.path()).await;
        let mut titles: Vec<&str> = logs.iter().map(|log| log.title.as_str()).collect();
        titles.sort_unstable();
        assert_eq!(titles, ["Legacy only", "New"]);
        let signup = logs.iter().find(|log| log.slug == "signup").unwrap();
        assert!(signup.directory.ends_with("stories/signup"));
    }

    #[tokio::test]
    async fn an_empty_stories_directory_does_not_hide_the_legacy_story() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("stories/signup")).unwrap();
        write_story(dir.path(), "friction-logs", "signup", "Old").await;
        let logs = load_stories(dir.path()).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].title, "Old");
    }

    // Beyond the TS tests: the loaders and label helpers.
    #[tokio::test]
    async fn loads_logs_runs_and_labels() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("friction-logs/my-flow");
        let run = log.join("runs/20260811T153000Z");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("log.jsonl"), "{\"step\":1.5,\"id\":\"a\"}\n").unwrap();
        std::fs::write(log.join("meta.json"), r#"{"status":"done","title":" "}"#).unwrap();
        let context = FrictionContext {
            worktree_path: "/w/wt7".into(),
            worktree: "wt7".into(),
        };
        let hashes = HashCache::default();
        let logs = load_friction_logs(
            dir.path().join("friction-logs").to_str().unwrap(),
            &context,
            &hashes,
        )
        .await
        .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].id, "/w/wt7::my-flow");
        assert_eq!(logs[0].title, "My Flow");
        assert_eq!(logs[0].runs[0].steps[0].id, "1.5-a");
        assert!(logs[0].runs[0].review.is_some());
        assert_eq!(
            friction_status_label(Some("done")).as_deref(),
            Some("Complete")
        );
        assert_eq!(
            friction_status_label(Some("on-hold")).as_deref(),
            Some("On Hold")
        );
        assert_eq!(friction_status_label(None), None);
        assert_eq!(js_number_string(1e21), "1e+21");
        assert_eq!(js_number_string(1e-7), "1e-7");
    }
}
