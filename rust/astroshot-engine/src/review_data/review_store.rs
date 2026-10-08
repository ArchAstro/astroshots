//! Port of `packages/astroshot-review/src/data/review-store.ts`.
//!
//! `review.json` — the human side of the on-disk contract.
//!
//! Reads mirror the macOS app exactly: version gate, run-id gate, then hash
//! scoping. Writes mirror it too: sorted keys, second-precision UTC
//! timestamps, uppercase UUID comment ids, run reset on a run-id change, and
//! an atomic temp-file rename.
//!
//! Output bytes match `JSON.stringify(sortKeys(doc), null, 2) + "\n"`: keys
//! sorted by UTF-16 code unit (JS default sort), two-space indent, trailing
//! newline. Every written value is a string, so no number formatting arises.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use super::model::{ReviewComment, ReviewSnapshot, ReviewState};

pub const REVIEW_FILE: &str = "review.json";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReviewEntry {
    pub decision: Option<String>,
    pub reviewed_at: Option<String>,
    pub image_sha256: Option<String>,
    pub comments: Option<Vec<ReviewComment>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewDocument {
    pub version: u32,
    pub run_id: Option<String>,
    pub updated_at: Option<String>,
    pub reviews: BTreeMap<String, StoredEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredComment {
    pub id: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredEntry {
    pub decision: Option<String>,
    pub reviewed_at: Option<String>,
    pub image_sha256: Option<String>,
    pub comments: Option<Vec<StoredComment>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unsupported review.json version {0}")]
pub struct UnsupportedReviewVersion(pub String);

pub fn empty_document() -> ReviewDocument {
    ReviewDocument {
        version: 1,
        run_id: None,
        updated_at: None,
        reviews: BTreeMap::new(),
    }
}

/// JS `String(number)` for a finite double.
fn js_number_string(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let abs = value.abs();
    if value.fract() == 0.0 && abs < 1e21 {
        return format!("{value:.0}");
    }
    if !(1e-6..1e21).contains(&abs) {
        let text = format!("{value:e}");
        return match text.split_once('e') {
            Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
            _ => text,
        };
    }
    format!("{value}")
}

/// JS `String(value)` for a parsed JSON value.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => js_number_string(number.as_f64().unwrap_or(f64::NAN)),
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// `String(value ?? "")`.
fn js_string_or_empty(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(other) => js_string(other),
    }
}

/// JS `String.prototype.trim` whitespace: Rust's `White_Space` plus U+FEFF,
/// minus U+0085 (which JS does not trim).
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{FEFF}')
}

/// Missing file -> empty document. Malformed JSON -> `None` (treated as
/// unreadable). Unsupported version -> `Err`.
pub async fn read_review_document(
    directory: &str,
) -> Result<Option<ReviewDocument>, UnsupportedReviewVersion> {
    let bytes = match tokio::fs::read(Path::new(directory).join(REVIEW_FILE)).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(empty_document()));
        }
        Err(_) => return Ok(None),
    };
    parse_review_document(&String::from_utf8_lossy(&bytes))
}

pub fn parse_review_document(
    raw: &str,
) -> Result<Option<ReviewDocument>, UnsupportedReviewVersion> {
    let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(raw) else {
        return Ok(None);
    };
    match parsed.get("version") {
        Some(Value::Number(number)) if number.as_f64() == Some(1.0) => {}
        other => {
            return Err(UnsupportedReviewVersion(
                other.map_or_else(|| "undefined".to_string(), js_string),
            ));
        }
    }
    let mut reviews = BTreeMap::new();
    if let Some(Value::Object(entries)) = parsed.get("reviews") {
        for (file_name, entry) in entries {
            let Value::Object(entry) = entry else {
                continue;
            };
            let comments = match entry.get("comments") {
                Some(Value::Array(items)) => Some(
                    items
                        .iter()
                        .filter_map(|item| item.as_object())
                        .map(|comment| StoredComment {
                            id: js_string_or_empty(comment.get("id")),
                            body: js_string_or_empty(comment.get("body")),
                            created_at: js_string_or_empty(comment.get("created_at")),
                        })
                        .collect(),
                ),
                _ => None,
            };
            let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
            reviews.insert(
                file_name.clone(),
                StoredEntry {
                    decision: text("decision"),
                    reviewed_at: text("reviewed_at"),
                    image_sha256: text("image_sha256"),
                    comments,
                },
            );
        }
    }
    let text = |key: &str| parsed.get(key).and_then(Value::as_str).map(str::to_string);
    Ok(Some(ReviewDocument {
        version: 1,
        run_id: text("run_id"),
        updated_at: text("updated_at"),
        reviews,
    }))
}

pub async fn sha256_file(file_path: &str) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(file_path).await?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hex(&hash.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The entry that applies to `file_name`, or `None` when the run gate rejects it.
pub fn scoped_entry<'a>(
    document: &'a ReviewDocument,
    file_name: &str,
    expected_run_id: Option<&str>,
) -> Option<&'a StoredEntry> {
    if let Some(expected) = expected_run_id
        && document.run_id.as_deref() != Some(expected)
    {
        return None;
    }
    document.reviews.get(file_name)
}

/// Whether validating this entry needs the image's current hash.
pub fn entry_needs_hash(entry: Option<&StoredEntry>) -> bool {
    entry
        .and_then(|entry| entry.image_sha256.as_deref())
        .is_some_and(|sha| !sha.is_empty())
}

/// The seen/stale truth table from the app's `ReviewSnapshot`: hash mismatch
/// hides the decision but keeps comments and flags staleness.
pub fn snapshot_from_entry(
    entry: Option<&StoredEntry>,
    current_sha256: Option<&str>,
) -> ReviewSnapshot {
    let stored_sha = entry
        .and_then(|entry| entry.image_sha256.as_deref())
        .filter(|sha| !sha.is_empty());
    let hash_matches = stored_sha.is_some() && stored_sha == current_sha256;
    let decision = entry.and_then(|entry| entry.decision.clone());
    let effective = if hash_matches {
        decision.as_deref()
    } else {
        None
    };
    ReviewSnapshot {
        state: if matches!(effective, Some("seen" | "approved")) {
            ReviewState::Seen
        } else {
            ReviewState::Pending
        },
        is_stale: decision.is_some() && !hash_matches,
        decision,
        hash_matches,
        comments: entry
            .and_then(|entry| entry.comments.as_ref())
            .map(|comments| {
                comments
                    .iter()
                    .map(|comment| ReviewComment {
                        id: comment.id.clone(),
                        body: comment.body.clone(),
                        created_at: comment.created_at.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        reviewed_at: entry.and_then(|entry| entry.reviewed_at.clone()),
    }
}

/// `date.toISOString()` with the milliseconds dropped (`...:00Z`).
pub fn now_iso(date: Option<DateTime<Utc>>) -> String {
    date.unwrap_or_else(Utc::now)
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn random_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom")
        && file.read_exact(&mut bytes).is_ok()
    {
        return bytes;
    }
    // Fallback: std's per-instance random SipHash keys plus the clock.
    use std::hash::{BuildHasher, Hasher};
    for chunk in bytes.chunks_mut(8) {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes
}

/// `crypto.randomUUID()`: a lowercase RFC 4122 version 4 UUID.
pub fn random_uuid() -> String {
    let mut bytes = random_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let digits = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &digits[0..8],
        &digits[8..12],
        &digits[12..16],
        &digits[16..20],
        &digits[20..32]
    )
}

pub fn new_comment_id() -> String {
    random_uuid().to_uppercase()
}

/// Rebuilds `value` with object keys in JS default sort order (UTF-16 code
/// units), recursively.
fn sort_keys(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            let mut sorted = Map::new();
            for (key, inner) in entries {
                sorted.insert(key, sort_keys(inner));
            }
            Value::Object(sorted)
        }
        other => other,
    }
}

fn document_value(document: &ReviewDocument) -> Value {
    let mut reviews = Map::new();
    for (file_name, entry) in &document.reviews {
        let mut stored = Map::new();
        if let Some(decision) = &entry.decision {
            stored.insert("decision".into(), Value::String(decision.clone()));
        }
        if let Some(reviewed_at) = &entry.reviewed_at {
            stored.insert("reviewed_at".into(), Value::String(reviewed_at.clone()));
        }
        if let Some(sha) = &entry.image_sha256 {
            stored.insert("image_sha256".into(), Value::String(sha.clone()));
        }
        if let Some(comments) = &entry.comments {
            let items = comments
                .iter()
                .map(|comment| {
                    let mut item = Map::new();
                    item.insert("id".into(), Value::String(comment.id.clone()));
                    item.insert("body".into(), Value::String(comment.body.clone()));
                    item.insert(
                        "created_at".into(),
                        Value::String(comment.created_at.clone()),
                    );
                    Value::Object(item)
                })
                .collect();
            stored.insert("comments".into(), Value::Array(items));
        }
        reviews.insert(file_name.clone(), Value::Object(stored));
    }
    let mut root = Map::new();
    root.insert("version".into(), Value::from(document.version));
    if let Some(run_id) = &document.run_id {
        root.insert("run_id".into(), Value::String(run_id.clone()));
    }
    if let Some(updated_at) = &document.updated_at {
        root.insert("updated_at".into(), Value::String(updated_at.clone()));
    }
    root.insert("reviews".into(), Value::Object(reviews));
    Value::Object(root)
}

pub fn serialize_review_document(document: &ReviewDocument) -> String {
    let value = sort_keys(document_value(document));
    // serde_json's pretty printer matches JSON.stringify(_, null, 2), including
    // `{}` / `[]` for empty containers.
    let mut text = serde_json::to_string_pretty(&value).expect("review document serializes");
    text.push('\n');
    text
}

pub async fn write_review_document(directory: &str, document: &ReviewDocument) -> Result<()> {
    let target = Path::new(directory).join(REVIEW_FILE);
    let temp = Path::new(directory).join(format!(".review.tmp.{}", random_uuid()));
    {
        use tokio::io::AsyncWriteExt;
        // `wx`: fail if the temp file already exists.
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await?;
        file.write_all(serialize_review_document(document).as_bytes())
            .await?;
        file.flush().await?;
    }
    if let Err(error) = tokio::fs::rename(&temp, &target).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error.into());
    }
    Ok(())
}

/// A run-id change starts a fresh review map, exactly like the app.
pub fn reset_reviews_if_needed(document: &mut ReviewDocument, run_id: Option<&str>) {
    if let Some(run_id) = run_id
        && document.run_id.as_deref() != Some(run_id)
    {
        document.run_id = Some(run_id.to_string());
        document.reviews = BTreeMap::new();
    }
}

#[derive(Debug, Clone)]
pub struct ReviewWriteRequest {
    /// Directory holding review.json (feature dir, or friction run dir).
    pub directory: String,
    /// Key inside `reviews` (image file name, or `log.jsonl`).
    pub file_name: String,
    pub run_id: Option<String>,
    /// Absolute path of the bytes to hash when marking seen.
    pub target_path: String,
}

#[derive(Debug, Clone, PartialEq)]
struct FileStamp {
    mtime: Option<SystemTime>,
    size: u64,
}

async fn stamp_of(file_path: &Path) -> Option<FileStamp> {
    let stat = tokio::fs::metadata(file_path).await.ok()?;
    Some(FileStamp {
        mtime: stat.modified().ok(),
        size: stat.len(),
    })
}

async fn load_for_write(directory: &str) -> Result<(ReviewDocument, Option<FileStamp>)> {
    let stamp = stamp_of(&Path::new(directory).join(REVIEW_FILE)).await;
    match read_review_document(directory).await? {
        Some(document) => Ok((document, stamp)),
        None => Err(anyhow!(
            "review.json in {directory} is not valid JSON; refusing to overwrite it"
        )),
    }
}

/// Read -> mutate -> write, re-reading when another writer (the macOS app, a
/// second tray) changed the file in between so neither side's update is lost.
async fn mutate_review_document<T>(
    directory: &str,
    mut mutate: impl FnMut(&mut ReviewDocument) -> T,
) -> Result<T> {
    let target = Path::new(directory).join(REVIEW_FILE);
    for _attempt in 0..5 {
        let (mut document, stamp) = load_for_write(directory).await?;
        let result = mutate(&mut document);
        if stamp != stamp_of(&target).await {
            continue;
        }
        write_review_document(directory, &document).await?;
        return Ok(result);
    }
    Err(anyhow!(
        "review.json in {directory} kept changing underneath this write; try again"
    ))
}

fn append_comment(entry: &mut StoredEntry, body: &str, now: &str) -> Result<ReviewComment> {
    let trimmed = js_trim(body);
    if trimmed.is_empty() {
        return Err(anyhow!("Feedback cannot be empty"));
    }
    let comment = StoredComment {
        id: new_comment_id(),
        body: trimmed.to_string(),
        created_at: now.to_string(),
    };
    entry
        .comments
        .get_or_insert_with(Vec::new)
        .push(comment.clone());
    Ok(ReviewComment {
        id: comment.id,
        body: comment.body,
        created_at: comment.created_at,
    })
}

#[derive(Debug, Clone, Default)]
pub struct MarkSeenOptions {
    pub comment: Option<String>,
    pub now: Option<DateTime<Utc>>,
}

pub async fn mark_seen(
    request: &ReviewWriteRequest,
    options: &MarkSeenOptions,
) -> Result<ReviewSnapshot> {
    let sha = sha256_file(&request.target_path).await?;
    mutate_review_document(&request.directory, |document| {
        reset_reviews_if_needed(document, request.run_id.as_deref());
        let now = now_iso(options.now);
        let mut entry = document
            .reviews
            .get(&request.file_name)
            .cloned()
            .unwrap_or_default();
        if let Some(comment) = options
            .comment
            .as_deref()
            .filter(|comment| !js_trim(comment).is_empty())
        {
            append_comment(&mut entry, comment, &now)?;
        }
        entry.decision = Some("seen".to_string());
        entry.reviewed_at = Some(now.clone());
        entry.image_sha256 = Some(sha.clone());
        document
            .reviews
            .insert(request.file_name.clone(), entry.clone());
        document.updated_at = Some(now);
        Ok::<_, anyhow::Error>(snapshot_from_entry(Some(&entry), Some(&sha)))
    })
    .await?
}

#[derive(Debug, Clone, Default)]
pub struct AddCommentOptions {
    pub current_sha256: Option<String>,
    pub now: Option<DateTime<Utc>>,
}

pub async fn add_comment(
    request: &ReviewWriteRequest,
    body: &str,
    options: &AddCommentOptions,
) -> Result<ReviewSnapshot> {
    if js_trim(body).is_empty() {
        return Err(anyhow!("Feedback cannot be empty"));
    }
    let entry = mutate_review_document(&request.directory, |document| {
        reset_reviews_if_needed(document, request.run_id.as_deref());
        let now = now_iso(options.now);
        let mut stored = document
            .reviews
            .get(&request.file_name)
            .cloned()
            .unwrap_or_default();
        append_comment(&mut stored, body, &now)?;
        document
            .reviews
            .insert(request.file_name.clone(), stored.clone());
        document.updated_at = Some(now);
        Ok::<_, anyhow::Error>(stored)
    })
    .await??;
    let sha = if entry
        .image_sha256
        .as_deref()
        .is_some_and(|sha| !sha.is_empty())
    {
        Some(match &options.current_sha256 {
            Some(sha) => sha.clone(),
            None => sha256_file(&request.target_path).await?,
        })
    } else {
        None
    };
    Ok(snapshot_from_entry(Some(&entry), sha.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha() -> String {
        "a".repeat(64)
    }

    fn comment(id: &str, body: &str, created_at: &str) -> StoredComment {
        StoredComment {
            id: id.into(),
            body: body.into(),
            created_at: created_at.into(),
        }
    }

    fn request(
        directory: &Path,
        file_name: &str,
        run_id: Option<&str>,
        target: &Path,
    ) -> ReviewWriteRequest {
        ReviewWriteRequest {
            directory: directory.to_str().unwrap().into(),
            file_name: file_name.into(),
            run_id: run_id.map(str::to_string),
            target_path: target.to_str().unwrap().into(),
        }
    }

    // review snapshot truth table

    #[test]
    fn is_seen_only_when_the_hash_matches() {
        let seen = snapshot_from_entry(
            Some(&StoredEntry {
                decision: Some("seen".into()),
                image_sha256: Some(sha()),
                comments: Some(vec![]),
                ..StoredEntry::default()
            }),
            Some(&sha()),
        );
        assert_eq!(seen.state, ReviewState::Seen);
        assert!(!seen.is_stale);
        let stale = snapshot_from_entry(
            Some(&StoredEntry {
                decision: Some("seen".into()),
                image_sha256: Some(sha()),
                comments: Some(vec![comment("1", "hi", "t")]),
                ..StoredEntry::default()
            }),
            Some(&"b".repeat(64)),
        );
        assert_eq!(stale.state, ReviewState::Pending);
        assert!(stale.is_stale);
        let bodies: Vec<&str> = stale.comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, ["hi"]);
    }

    #[test]
    fn accepts_legacy_approved_and_rejects_changes_requested() {
        let entry = |decision: &str| StoredEntry {
            decision: Some(decision.into()),
            image_sha256: Some(sha()),
            ..StoredEntry::default()
        };
        assert_eq!(
            snapshot_from_entry(Some(&entry("approved")), Some(&sha())).state,
            ReviewState::Seen
        );
        assert_eq!(
            snapshot_from_entry(Some(&entry("changes_requested")), Some(&sha())).state,
            ReviewState::Pending
        );
    }

    #[test]
    fn treats_comment_only_entries_as_pending_and_not_stale() {
        let snapshot = snapshot_from_entry(
            Some(&StoredEntry {
                comments: Some(vec![comment("1", "x", "t")]),
                ..StoredEntry::default()
            }),
            None,
        );
        assert_eq!(snapshot.state, ReviewState::Pending);
        assert!(!snapshot.is_stale);
        assert_eq!(snapshot.comments.len(), 1);
    }

    #[test]
    fn gates_on_run_id() {
        let raw = serde_json::json!({
            "version": 1,
            "run_id": "run-1",
            "reviews": {"a.png": {"decision": "seen", "image_sha256": sha()}}
        })
        .to_string();
        let document = parse_review_document(&raw).unwrap().unwrap();
        assert!(scoped_entry(&document, "a.png", Some("run-1")).is_some());
        assert!(scoped_entry(&document, "a.png", Some("run-2")).is_none());
        assert!(scoped_entry(&document, "a.png", None).is_some());
    }

    #[test]
    fn rejects_unsupported_versions_and_malformed_json_differently() {
        let error = parse_review_document(r#"{"version":2,"reviews":{}}"#).unwrap_err();
        assert_eq!(error.to_string(), "Unsupported review.json version 2");
        assert_eq!(parse_review_document("{ nope"), Ok(None));
        assert_eq!(
            parse_review_document("{}").unwrap_err().to_string(),
            "Unsupported review.json version undefined"
        );
        assert_eq!(parse_review_document("[1]"), Ok(None));
    }

    // review writes

    #[tokio::test]
    async fn marks_seen_with_the_apps_exact_document_shape() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("0001-a.png");
        std::fs::write(&image, b"png-bytes").unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-05T17:42:00.123Z")
            .unwrap()
            .with_timezone(&Utc);
        mark_seen(
            &request(dir.path(), "0001-a.png", Some("run-7"), &image),
            &MarkSeenOptions {
                comment: Some("  Looks clipped  ".into()),
                now: Some(now),
            },
        )
        .await
        .unwrap();
        let raw = std::fs::read_to_string(dir.path().join("review.json")).unwrap();
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        let keys: Vec<&String> = parsed.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["reviews", "run_id", "updated_at", "version"]);
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["run_id"], "run-7");
        assert_eq!(parsed["updated_at"], "2026-09-05T17:42:00Z");
        let entry = &parsed["reviews"]["0001-a.png"];
        assert_eq!(entry["decision"], "seen");
        assert_eq!(entry["reviewed_at"], "2026-09-05T17:42:00Z");
        assert_eq!(entry["image_sha256"], sha256_bytes(b"png-bytes"));
        assert_eq!(entry["comments"].as_array().unwrap().len(), 1);
        assert_eq!(entry["comments"][0]["body"], "Looks clipped");
        let id = entry["comments"][0]["id"].as_str().unwrap();
        let shape: Vec<usize> = id.split('-').map(str::len).collect();
        assert_eq!(shape, [8, 4, 4, 4, 12]);
        assert!(
            id.chars()
                .all(|c| c == '-' || c.is_ascii_digit() || ('A'..='F').contains(&c))
        );
        assert!(raw.ends_with('\n'));
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".review.tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[tokio::test]
    async fn adds_comments_without_inventing_a_decision_and_appends_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("0002-b.png");
        std::fs::write(&image, "x").unwrap();
        let req = request(dir.path(), "0002-b.png", None, &image);
        add_comment(&req, "first", &AddCommentOptions::default())
            .await
            .unwrap();
        add_comment(&req, "second", &AddCommentOptions::default())
            .await
            .unwrap();
        let document = read_review_document(dir.path().to_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        let entry = &document.reviews["0002-b.png"];
        assert_eq!(entry.decision, None);
        assert_eq!(entry.image_sha256, None);
        let bodies: Vec<&str> = entry
            .comments
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(bodies, ["first", "second"]);
    }

    #[tokio::test]
    async fn wipes_prior_reviews_when_the_run_id_changes() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("0001-a.png");
        std::fs::write(&image, "x").unwrap();
        mark_seen(
            &request(dir.path(), "0001-a.png", Some("run-1"), &image),
            &MarkSeenOptions::default(),
        )
        .await
        .unwrap();
        add_comment(
            &request(dir.path(), "0001-a.png", Some("run-2"), &image),
            "new run",
            &AddCommentOptions::default(),
        )
        .await
        .unwrap();
        let document = read_review_document(dir.path().to_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(document.run_id.as_deref(), Some("run-2"));
        assert_eq!(document.reviews["0001-a.png"].decision, None);
        assert_eq!(
            document.reviews["0001-a.png"]
                .comments
                .as_ref()
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn refuses_to_overwrite_a_review_json_it_cannot_parse() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("review.json"), "{ broken").unwrap();
        let image = dir.path().join("0001-a.png");
        std::fs::write(&image, "x").unwrap();
        let error = mark_seen(
            &request(dir.path(), "0001-a.png", None, &image),
            &MarkSeenOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("not valid JSON"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("review.json")).unwrap(),
            "{ broken"
        );
    }

    #[test]
    fn serializes_with_sorted_keys_and_two_space_indentation() {
        let document = ReviewDocument {
            version: 1,
            run_id: Some("r".into()),
            updated_at: None,
            reviews: BTreeMap::from([
                (
                    "b.png".to_string(),
                    StoredEntry {
                        decision: Some("seen".into()),
                        ..StoredEntry::default()
                    },
                ),
                ("a.png".to_string(), StoredEntry::default()),
            ]),
        };
        let text = serialize_review_document(&document);
        assert!(text.find("\"a.png\"").unwrap() < text.find("\"b.png\"").unwrap());
        assert!(text.starts_with("{\n  \"reviews\""));
    }

    // Beyond the TS cases.

    /// Expected text produced by the TS `serializeReviewDocument` (node) for
    /// the same document: sorted by UTF-16 code unit (so the astral key sorts
    /// before U+FF5E), `\u0001` escaped, U+2028 raw, empty containers `{}`
    /// and `[]`, two-space indent, trailing newline.
    #[test]
    fn serialized_bytes_match_the_ts_output_exactly() {
        let document = ReviewDocument {
            version: 1,
            run_id: Some("run-7".into()),
            updated_at: Some("2026-09-05T17:42:00Z".into()),
            reviews: BTreeMap::from([
                (
                    "\u{FF5E}.png".to_string(),
                    StoredEntry {
                        decision: Some("seen".into()),
                        reviewed_at: Some("2026-09-05T17:42:00Z".into()),
                        image_sha256: Some("ab".repeat(32)),
                        comments: Some(vec![comment(
                            "A",
                            "say \"hi\"\n\ttab \u{1} \u{e9} \u{1F600} / </x>\u{2028}",
                            "t",
                        )]),
                    },
                ),
                (
                    "\u{1F600}.png".to_string(),
                    StoredEntry {
                        comments: Some(vec![]),
                        ..StoredEntry::default()
                    },
                ),
                ("b.png".to_string(), StoredEntry::default()),
                (
                    "a.png".to_string(),
                    StoredEntry {
                        decision: Some("seen".into()),
                        ..StoredEntry::default()
                    },
                ),
            ]),
        };
        let expected = concat!(
            "{\n",
            "  \"reviews\": {\n",
            "    \"a.png\": {\n",
            "      \"decision\": \"seen\"\n",
            "    },\n",
            "    \"b.png\": {},\n",
            "    \"\u{1F600}.png\": {\n",
            "      \"comments\": []\n",
            "    },\n",
            "    \"\u{FF5E}.png\": {\n",
            "      \"comments\": [\n",
            "        {\n",
            "          \"body\": \"say \\\"hi\\\"\\n\\ttab \\u0001 \u{e9} \u{1F600} / </x>\u{2028}\",\n",
            "          \"created_at\": \"t\",\n",
            "          \"id\": \"A\"\n",
            "        }\n",
            "      ],\n",
            "      \"decision\": \"seen\",\n",
            "      \"image_sha256\": \"abababababababababababababababababababababababababababababababab\",\n",
            "      \"reviewed_at\": \"2026-09-05T17:42:00Z\"\n",
            "    }\n",
            "  },\n",
            "  \"run_id\": \"run-7\",\n",
            "  \"updated_at\": \"2026-09-05T17:42:00Z\",\n",
            "  \"version\": 1\n",
            "}\n",
        );
        assert_eq!(serialize_review_document(&document), expected);
        assert_eq!(
            serialize_review_document(&empty_document()),
            "{\n  \"reviews\": {},\n  \"version\": 1\n}\n"
        );
    }

    #[test]
    fn parses_loosely_typed_comments_like_string_coercion() {
        let raw = r#"{"version":1,"reviews":{"a.png":{"comments":[{"id":5,"body":null,"created_at":true},7,{}],"decision":3}, "b.png": 1}}"#;
        let document = parse_review_document(raw).unwrap().unwrap();
        assert_eq!(document.reviews.len(), 1);
        let entry = &document.reviews["a.png"];
        assert_eq!(entry.decision, None);
        assert_eq!(
            entry.comments,
            Some(vec![comment("5", "", "true"), comment("", "", "")])
        );
    }

    #[test]
    fn js_trim_follows_ecmascript_whitespace() {
        assert_eq!(js_trim("\u{FEFF} a \u{a0}\u{2028}"), "a");
        assert_eq!(js_trim("\u{85}a\u{85}"), "\u{85}a\u{85}");
    }

    #[test]
    fn now_iso_drops_milliseconds() {
        let date = DateTime::parse_from_rfc3339("2026-01-02T03:04:05.999Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(now_iso(Some(date)), "2026-01-02T03:04:05Z");
    }

    #[test]
    fn random_uuid_is_version_4_and_unique() {
        let a = random_uuid();
        assert_ne!(a, random_uuid());
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
    }

    #[tokio::test]
    async fn missing_file_reads_as_an_empty_document() {
        let dir = tempfile::tempdir().unwrap();
        let document = read_review_document(dir.path().to_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(document, empty_document());
    }

    #[tokio::test]
    async fn rejects_blank_feedback() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("a.png");
        std::fs::write(&image, "x").unwrap();
        let error = add_comment(
            &request(dir.path(), "a.png", None, &image),
            " \n ",
            &AddCommentOptions::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "Feedback cannot be empty");
        assert!(!dir.path().join("review.json").exists());
    }
}
