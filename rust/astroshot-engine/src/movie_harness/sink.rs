//! Publish a movie into `.astroshot/<feature>/` and maintain `manifest.json`.
//!
//! Port of `packages/movie-harness/src/sink.ts`. The macOS app reads these
//! files, so bytes match `JSON.stringify(manifest, null, 2) + "\n"`: key order
//! is preserved (serde_json `preserve_order`), whole numbers have no `.0`, and
//! optional fields are omitted when absent. The manifest is edited as a JSON
//! map so keys of an existing manifest keep their position and unknown keys
//! survive, as with the TS object spread.
//!
//! [`sink_still`] and [`finalize_current_run`] are Rust-only additions (no TS
//! counterpart): they follow `skills/astroshots-review/scripts/astroshot-capture`
//! (`--source`, and `--status <s> --finalize` without `--run-id`). Every
//! function here that picks a sequence number or rewrites `manifest.json`
//! holds the helper's `.capture.lock` (see [`super::capture_lock`]).

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};

use super::capture_lock::{CaptureLock, DEFAULT_LOCK_TIMEOUT};
use super::encode::{extname_lower, js_round};
use super::paths::default_run_id;
use super::paths::{
    assert_kebab_case, assert_slug, assert_still_slug, ensure_dir, feature_dir, humanize,
    next_sequence,
};
use super::types::{ManifestStatus, SinkMovieRequest, SinkMovieResult};

fn path_join(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

fn status_str(status: ManifestStatus) -> String {
    match serde_json::to_value(status) {
        Ok(Value::String(text)) => text,
        _ => String::new(),
    }
}

/// A JS number as JSON: integers without `.0`, non-finite as `null`.
fn js_number_value(value: f64) -> Value {
    if !value.is_finite() {
        Value::Null
    } else if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

fn read_manifest(manifest_path: &str) -> Result<Option<Map<String, Value>>> {
    if !Path::new(manifest_path).exists() {
        return Ok(None);
    }
    let Value::Object(mut raw) =
        serde_json::from_str::<Value>(&fs::read_to_string(manifest_path)?)?
    else {
        return Ok(None);
    };
    if !raw.get("shots").is_some_and(Value::is_array) {
        raw.insert("shots".into(), json!([]));
    }
    Ok(Some(raw))
}

fn write_atomic(file_path: &str, contents: &str) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = Path::new(file_path);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_dir(&dir.to_string_lossy())?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.tmp.{}.{}.{}",
        std::process::id(),
        Utc::now().timestamp_millis(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// `JSON.stringify(value, null, 2)` plus the trailing newline.
fn manifest_text(manifest: &Map<String, Value>) -> String {
    let mut text = serde_json::to_string_pretty(manifest).expect("manifest serializes");
    text.push('\n');
    text
}

/// Directories under `.astroshot/` that hold user stories; never shot features.
const RESERVED_FEATURES: [&str; 2] = ["stories", "friction-logs"];

/// A new manifest's leading keys, in file order. Callers add `description`
/// (movies) and `shots`.
fn new_manifest(feature: &str, run_id: &str, status: &str) -> Map<String, Value> {
    let mut manifest = Map::new();
    manifest.insert("version".into(), json!(1));
    manifest.insert("feature".into(), json!(feature));
    manifest.insert("run_id".into(), json!(run_id));
    manifest.insert("status".into(), json!(status));
    manifest
}

/// Append `shot` to the manifest's `shots` array.
fn push_shot(manifest: &mut Map<String, Value>, shot: Map<String, Value>) {
    let mut shots = match manifest.get("shots") {
        Some(Value::Array(shots)) => shots.clone(),
        _ => Vec::new(),
    };
    shots.push(Value::Object(shot));
    manifest.insert("shots".into(), Value::Array(shots));
}

/// Whether `existing` is a running manifest of the run `run_id`.
fn continues_running_run(existing: &Option<Map<String, Value>>, run_id: &str) -> bool {
    existing.as_ref().is_some_and(|existing| {
        existing.get("run_id").and_then(Value::as_str) == Some(run_id)
            && existing.get("status").and_then(Value::as_str) == Some("running")
    })
}

/// Publish poster + video into `.astroshot/<feature>/` and append a movie shot
/// to `manifest.json`. Poster basename is the review key (matches stills).
///
/// Holds the capture lock for up to [`DEFAULT_LOCK_TIMEOUT`]; see
/// [`sink_movie_with_lock_timeout`].
pub fn sink_movie(request: &SinkMovieRequest) -> Result<SinkMovieResult> {
    sink_movie_with_lock_timeout(request, None)
}

/// [`sink_movie`] with an explicit wait for the capture lock (`None` is
/// [`DEFAULT_LOCK_TIMEOUT`]).
pub fn sink_movie_with_lock_timeout(
    request: &SinkMovieRequest,
    lock_timeout: Option<Duration>,
) -> Result<SinkMovieResult> {
    assert_kebab_case(&request.feature, "feature")?;
    assert_slug(&request.slug)?;

    let dir = feature_dir(&request.root, &request.feature);
    ensure_dir(&dir)?;
    let _lock = CaptureLock::acquire(&dir, lock_timeout.unwrap_or(DEFAULT_LOCK_TIMEOUT))?;
    let sequence = next_sequence(&dir)?;
    let poster_name = format!("{sequence}-{}.png", request.slug);
    let ext = extname_lower(&request.video_path);
    let video_ext = if ext.is_empty() { ".webm" } else { &ext };
    let video_name = format!("{sequence}-{}{video_ext}", request.slug);
    let poster_dest = path_join(&dir, &poster_name);
    let video_dest = path_join(&dir, &video_name);

    if !Path::new(&request.poster_path).exists() {
        bail!("poster not found: {}", request.poster_path);
    }
    if !Path::new(&request.video_path).exists() {
        bail!("video not found: {}", request.video_path);
    }

    fs::copy(&request.poster_path, &poster_dest)?;
    fs::copy(&request.video_path, &video_dest)?;

    let manifest_path = path_join(&dir, "manifest.json");
    let existing = read_manifest(&manifest_path)?;
    let continue_run = continues_running_run(&existing, &request.run_id);

    let mut shot = Map::new();
    shot.insert("id".into(), json!(sequence));
    shot.insert("file".into(), json!(poster_name));
    shot.insert("slug".into(), json!(request.slug));
    shot.insert(
        "title".into(),
        json!(
            request
                .title
                .clone()
                .unwrap_or_else(|| humanize(&request.slug))
        ),
    );
    if let Some(description) = &request.description {
        shot.insert("description".into(), json!(description));
    }
    shot.insert(
        "captured_at".into(),
        json!(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    );
    if let Some(size) = request.size {
        shot.insert(
            "viewport".into(),
            json!(format!("{}x{}", size.width, size.height)),
        );
    }
    shot.insert("kind".into(), json!("movie"));
    shot.insert("video".into(), json!(video_name));
    shot.insert(
        "duration_ms".into(),
        js_number_value(js_round(request.duration_ms)),
    );
    shot.insert(
        "source".into(),
        serde_json::to_value(request.source).expect("source serializes"),
    );
    let chapters: Vec<Value> = request
        .chapters
        .iter()
        .map(|chapter| {
            let mut entry = Map::new();
            entry.insert("slug".into(), json!(chapter.slug));
            entry.insert("t_ms".into(), js_number_value(js_round(chapter.t_ms)));
            if let Some(note) = &chapter.note {
                entry.insert("note".into(), json!(note));
            }
            Value::Object(entry)
        })
        .collect();
    shot.insert("chapters".into(), Value::Array(chapters));

    let manifest = match existing.filter(|_| continue_run) {
        Some(mut manifest) => {
            if let Some(status) = request.status {
                manifest.insert("status".into(), json!(status_str(status)));
            }
            if let Some(description) = &request.description {
                manifest.insert("description".into(), json!(description));
            }
            push_shot(&mut manifest, shot);
            manifest
        }
        None => {
            let mut manifest = new_manifest(
                &request.feature,
                &request.run_id,
                &request.status.map_or("running".to_string(), status_str),
            );
            if let Some(description) = &request.description {
                manifest.insert("description".into(), json!(description));
            }
            manifest.insert("shots".into(), json!([shot]));
            manifest
        }
    };

    write_atomic(&manifest_path, &manifest_text(&manifest))?;

    Ok(SinkMovieResult {
        sequence,
        poster_dest,
        video_dest,
        manifest_path,
        feature_dir: dir,
    })
}

// -------------------------------------------------------------------- stills

/// Request for [`sink_still`]: publish an existing image as a still shot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkStillRequest {
    /// Repo or worktree root; the shot lands in `<root>/.astroshot/<feature>/`.
    pub root: String,
    /// Kebab-case feature directory name. `stories` and `friction-logs` are
    /// reserved for user stories and rejected.
    pub feature: String,
    /// Slug of the file name (`NNNN-<slug>.<ext>`), `^[a-z0-9][a-z0-9_-]*$`.
    /// This is the bash helper's rule; movies still reject underscores.
    pub slug: String,
    /// The image to publish: png, jpg, jpeg, webp or gif.
    pub image_path: String,
    /// Explicit run identity. A run id equal to the manifest's continues that
    /// run, even after it was finished; any other id starts a new run. When
    /// absent, a running manifest is continued and anything else starts a new
    /// run with a generated id.
    pub run_id: Option<String>,
    /// Human title; defaults to the humanized slug.
    pub title: Option<String>,
    /// What the frame proves; written as `""` when absent.
    pub description: Option<String>,
    /// Route or UI context; omitted when absent or empty.
    pub url: Option<String>,
    /// For example `1280x1100`; omitted when absent or empty.
    pub viewport: Option<String>,
    /// Manifest status after this capture; defaults to `running`.
    pub status: Option<ManifestStatus>,
    /// How long to wait for the feature's capture lock; `None` is
    /// [`DEFAULT_LOCK_TIMEOUT`] (the helper's 120 s).
    pub lock_timeout: Option<Duration>,
}

impl SinkStillRequest {
    pub fn new(
        root: impl Into<String>,
        feature: impl Into<String>,
        slug: impl Into<String>,
        image_path: impl Into<String>,
    ) -> Self {
        Self {
            root: root.into(),
            feature: feature.into(),
            slug: slug.into(),
            image_path: image_path.into(),
            run_id: None,
            title: None,
            description: None,
            url: None,
            viewport: None,
            status: None,
            lock_timeout: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkStillResult {
    pub sequence: String,
    /// The published image, `<feature_dir>/<sequence>-<slug>.<ext>`.
    pub image_dest: String,
    pub manifest_path: String,
    pub feature_dir: String,
    /// The run the shot belongs to (generated when the request had none).
    pub run_id: String,
}

const STILL_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// The structural checks of `validate_image_source` in `astroshot-capture`:
/// signature, mandatory header chunk and terminator per format.
fn is_structurally_valid_image(bytes: &[u8], extension: &str) -> bool {
    let size = bytes.len();
    match extension {
        "png" => {
            size >= 45
                && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
                && bytes[8..12] == [0, 0, 0, 0x0d]
                && &bytes[12..16] == b"IHDR"
                && bytes[size - 12..]
                    == [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]
        }
        "jpg" | "jpeg" => {
            size >= 20 && bytes[..3] == [0xff, 0xd8, 0xff] && bytes[size - 2..] == [0xff, 0xd9]
        }
        "gif" => {
            size >= 14
                && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a")
                && bytes[size - 1] == 0x3b
        }
        "webp" => {
            size >= 20
                && &bytes[..4] == b"RIFF"
                && &bytes[8..12] == b"WEBP"
                && [&b"VP8 "[..], b"VP8L", b"VP8X"].contains(&&bytes[12..16])
                && u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize + 8 == size
        }
        _ => false,
    }
}

/// Whether the `png` crate decodes every row of the image and reaches the end
/// of the file: a truncated or corrupt IDAT stream, a bad chunk CRC or a bad
/// zlib checksum fails here although the signature and `IEND` bytes are fine.
fn png_decodes(bytes: &[u8]) -> bool {
    let Ok(mut reader) = png::Decoder::new(std::io::Cursor::new(bytes)).read_info() else {
        return false;
    };
    loop {
        match reader.next_row() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => return false,
        }
    }
    reader.finish().is_ok()
}

/// What `sink_still` requires of the source image: the helper's structural
/// checks, plus a full decode for PNG. The helper additionally decodes any
/// format with `sips`, `magick` or `identify` when one is installed; here only
/// PNG is decoded, because the `image` crate is built with its `png` feature
/// only. Truncated JPEG, GIF and WebP files that keep their header and
/// terminator bytes pass.
fn is_valid_image(bytes: &[u8], extension: &str) -> bool {
    is_structurally_valid_image(bytes, extension) && (extension != "png" || png_decodes(bytes))
}

/// The helper's `next_index`: one more than the highest `N` over files named
/// `N-*.<png|jpg|jpeg|webp|gif>` (`N` all digits, any length, extension in
/// lower case), 9999 being the last usable number.
fn next_still_sequence(dir: &str, feature: &str) -> Result<String> {
    let mut max: u64 = 0;
    if Path::new(dir).exists() {
        for entry in fs::read_dir(dir)? {
            let name = entry?.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(suffix) = STILL_EXTENSIONS
                .iter()
                .map(|extension| format!(".{extension}"))
                .find(|suffix| name.ends_with(suffix.as_str()))
            else {
                continue;
            };
            let Some(dash) = name.find('-') else { continue };
            // The glob `[0-9]*-*.ext` needs a digit first and the dash before the suffix.
            if dash == 0 || dash >= name.len() - suffix.len() {
                continue;
            }
            let number = &name[..dash];
            if !number.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            max = max.max(number.parse::<u64>().unwrap_or(u64::MAX));
        }
    }
    if max >= 9999 {
        bail!("capture sequence limit 9999 reached for feature {feature}; start a new feature");
    }
    Ok(format!("{:04}", max + 1))
}

/// The manifest as a JSON object, `None` when the file does not exist. Unlike
/// [`read_manifest`] this adds nothing and rejects a file that is not an
/// object, as `jq` would fail on it in the bash helper.
fn read_manifest_object(manifest_path: &str) -> Result<Option<Map<String, Value>>> {
    if !Path::new(manifest_path).exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(manifest_path)
        .map_err(|error| anyhow!("cannot read {manifest_path}: {error}"))?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(manifest)) => Ok(Some(manifest)),
        Ok(_) => bail!("{manifest_path} is not a JSON object"),
        Err(error) => bail!("{manifest_path} is not valid JSON: {error}"),
    }
}

/// A run id of the manifest, when it has a non-empty string one.
fn manifest_run_id(manifest: Option<&Map<String, Value>>) -> Option<String> {
    manifest
        .and_then(|manifest| manifest.get("run_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Reject names the user-story trees own.
fn assert_not_reserved(feature: &str) -> Result<()> {
    if RESERVED_FEATURES.contains(&feature) {
        bail!("feature \"{feature}\" is reserved for user stories; pick another feature name");
    }
    Ok(())
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Publish an existing image into `.astroshot/<feature>/` as `NNNN-<slug>.<ext>`
/// and append a still shot to `manifest.json`. Follows
/// `skills/astroshots-review/scripts/astroshot-capture --source`:
///
/// - the capture lock (`.capture.lock`, shared with the helper) is held from
///   the sequence pick to the manifest rewrite;
/// - the sequence is the helper's (highest image number plus one);
/// - the image is validated before any file or directory is created, then
///   published by rename, so the manifest never names a half-written file;
/// - an existing manifest is edited in place: `status`, `run_id` and `shots`
///   are set and every other key (`description`, unknown keys) keeps its value
///   and position. A new run resets `shots`.
///
/// One difference: a generated run id that equals the manifest's current one
/// (two runs in the same second of one process) gets a `-2`, `-3`... suffix,
/// which the helper cannot hit because each run has its own pid.
pub fn sink_still(request: &SinkStillRequest) -> Result<SinkStillResult> {
    assert_kebab_case(&request.feature, "feature")?;
    assert_not_reserved(&request.feature)?;
    assert_still_slug(&request.slug)?;
    if request.run_id.as_deref() == Some("") {
        bail!("run_id must not be empty");
    }

    let source = Path::new(&request.image_path);
    if !source.is_file() {
        bail!("image not found: {}", request.image_path);
    }
    let extension = extname_lower(&request.image_path)
        .trim_start_matches('.')
        .to_string();
    if extension.is_empty() {
        bail!(
            "image must have a supported extension: png|jpg|jpeg|webp|gif ({})",
            request.image_path
        );
    }
    if !STILL_EXTENSIONS.contains(&extension.as_str()) {
        bail!(
            "unsupported image extension .{extension}; expected png|jpg|jpeg|webp|gif ({})",
            request.image_path
        );
    }
    // Read once: the bytes that were validated are the bytes that are published.
    let image = fs::read(source)?;
    if !is_valid_image(&image, &extension) {
        bail!("not a valid .{extension} image: {}", request.image_path);
    }

    let dir = feature_dir(&request.root, &request.feature);
    ensure_dir(&dir)?;
    let _lock = CaptureLock::acquire(&dir, request.lock_timeout.unwrap_or(DEFAULT_LOCK_TIMEOUT))?;

    let sequence = next_still_sequence(&dir, &request.feature)?;
    let file_name = format!("{sequence}-{}.{extension}", request.slug);
    let image_dest = path_join(&dir, &file_name);
    let manifest_path = path_join(&dir, "manifest.json");
    let existing = read_manifest_object(&manifest_path)?;

    let existing_run = manifest_run_id(existing.as_ref());
    let (run_id, new_run) = match &request.run_id {
        Some(explicit) => (
            explicit.clone(),
            existing_run.as_deref() != Some(explicit.as_str()),
        ),
        None => {
            let running = existing
                .as_ref()
                .and_then(|manifest| manifest.get("status"))
                .and_then(Value::as_str)
                == Some("running");
            match existing_run {
                Some(id) if running => (id, false),
                other => {
                    let base = default_run_id(&request.feature);
                    let mut id = base.clone();
                    let mut attempt = 1;
                    while Some(&id) == other.as_ref() {
                        attempt += 1;
                        id = format!("{base}-{attempt}");
                    }
                    (id, true)
                }
            }
        }
    };
    let status = status_str(request.status.unwrap_or(ManifestStatus::Running));

    let mut shot = Map::new();
    shot.insert("id".into(), json!(sequence));
    shot.insert("file".into(), json!(file_name));
    shot.insert("slug".into(), json!(request.slug));
    let title = request.title.clone().filter(|title| !title.is_empty());
    shot.insert(
        "title".into(),
        json!(title.unwrap_or_else(|| humanize(&request.slug))),
    );
    shot.insert(
        "description".into(),
        json!(request.description.clone().unwrap_or_default()),
    );
    shot.insert(
        "captured_at".into(),
        json!(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)),
    );
    for (key, value) in [("url", &request.url), ("viewport", &request.viewport)] {
        if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
            shot.insert(key.into(), json!(value));
        }
    }

    let mut manifest = existing.unwrap_or_else(|| new_manifest(&request.feature, &run_id, &status));
    // The same edits, in the same order, as the helper's jq program.
    manifest.insert("status".into(), json!(status));
    manifest.insert("run_id".into(), json!(run_id));
    let mut shots = match manifest.get("shots") {
        _ if new_run => Vec::new(),
        None | Some(Value::Null | Value::Bool(false)) => Vec::new(),
        Some(Value::Array(shots)) => shots.clone(),
        Some(_) => bail!("{manifest_path}: \"shots\" is not an array"),
    };
    shots.push(Value::Object(shot));
    manifest.insert("shots".into(), Value::Array(shots));

    // The manifest is the commit record: publish the image first, and take it
    // back if the manifest cannot be written.
    let temp = path_join(
        &dir,
        &format!(
            ".capture.tmp.{}.{}.{extension}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::SeqCst)
        ),
    );
    let published = fs::write(&temp, &image).and_then(|()| fs::rename(&temp, &image_dest));
    if let Err(error) = published {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    if let Err(error) = write_atomic(&manifest_path, &manifest_text(&manifest)) {
        let _ = fs::remove_file(&image_dest);
        return Err(error);
    }

    Ok(SinkStillResult {
        sequence,
        image_dest,
        manifest_path,
        feature_dir: dir,
        run_id,
    })
}

/// Set the status of the run `run_id`, which must be the manifest's run (a
/// missing manifest is created). Holds the capture lock for up to
/// [`DEFAULT_LOCK_TIMEOUT`]; see [`finalize_manifest_with_lock_timeout`].
pub fn finalize_manifest(
    root: &str,
    feature: &str,
    run_id: &str,
    status: ManifestStatus,
) -> Result<()> {
    finalize_manifest_with_lock_timeout(root, feature, run_id, status, None)
}

/// [`finalize_manifest`] with an explicit wait for the capture lock (`None` is
/// [`DEFAULT_LOCK_TIMEOUT`]).
pub fn finalize_manifest_with_lock_timeout(
    root: &str,
    feature: &str,
    run_id: &str,
    status: ManifestStatus,
    lock_timeout: Option<Duration>,
) -> Result<()> {
    assert_kebab_case(feature, "feature")?;
    let dir = feature_dir(root, feature);
    ensure_dir(&dir)?;
    let _lock = CaptureLock::acquire(&dir, lock_timeout.unwrap_or(DEFAULT_LOCK_TIMEOUT))?;
    let manifest_path = path_join(&dir, "manifest.json");
    let status = status_str(status);
    let Some(mut existing) = read_manifest(&manifest_path)? else {
        let mut manifest = new_manifest(feature, run_id, &status);
        manifest.insert("shots".into(), json!([]));
        return write_atomic(&manifest_path, &manifest_text(&manifest));
    };
    if existing.get("run_id").and_then(Value::as_str) != Some(run_id) {
        let found = existing
            .get("run_id")
            .map_or_else(|| "undefined".to_string(), Value::to_string);
        bail!(
            "manifest run_id {found} does not match {}",
            Value::String(run_id.to_string())
        );
    }
    existing.insert("status".into(), json!(status));
    write_atomic(&manifest_path, &manifest_text(&existing))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeCurrentRunResult {
    /// The run that was finalized: the manifest's `run_id`, or the generated
    /// one when the manifest did not exist. `None` when an existing manifest
    /// has no `run_id` (its status is still set; nothing else is added).
    pub run_id: Option<String>,
    pub manifest_path: String,
    /// Whether the manifest did not exist and was created (empty `shots`).
    pub created: bool,
}

/// `astroshot-capture --status <s> --finalize` without `--run-id`: set the
/// status of the manifest's current run and add no shot. Like the helper, it
/// creates `{version, feature, run_id, status, shots: []}` with a generated
/// run id when there is no manifest, edits an existing one in place (only
/// `status` changes, keys keep their order), and holds the capture lock.
/// `lock_timeout` `None` is [`DEFAULT_LOCK_TIMEOUT`]. Names under the
/// user-story trees (`stories`, `friction-logs`) are rejected, as for
/// [`sink_still`]; the helper itself has no such check.
pub fn finalize_current_run(
    root: &str,
    feature: &str,
    status: ManifestStatus,
    lock_timeout: Option<Duration>,
) -> Result<FinalizeCurrentRunResult> {
    assert_kebab_case(feature, "feature")?;
    assert_not_reserved(feature)?;
    let dir = feature_dir(root, feature);
    ensure_dir(&dir)?;
    let _lock = CaptureLock::acquire(&dir, lock_timeout.unwrap_or(DEFAULT_LOCK_TIMEOUT))?;
    let manifest_path = path_join(&dir, "manifest.json");
    let status = status_str(status);
    match read_manifest_object(&manifest_path)? {
        None => {
            let run_id = default_run_id(feature);
            let mut manifest = new_manifest(feature, &run_id, &status);
            manifest.insert("shots".into(), json!([]));
            write_atomic(&manifest_path, &manifest_text(&manifest))?;
            Ok(FinalizeCurrentRunResult {
                run_id: Some(run_id),
                manifest_path,
                created: true,
            })
        }
        Some(mut manifest) => {
            let run_id = manifest_run_id(Some(&manifest));
            manifest.insert("status".into(), json!(status));
            write_atomic(&manifest_path, &manifest_text(&manifest))?;
            Ok(FinalizeCurrentRunResult {
                run_id,
                manifest_path,
                created: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movie_harness::types::{MovieChapter, MovieSourceKind, Size};

    struct Fixture {
        dir: tempfile::TempDir,
        root: String,
        poster: String,
        video: String,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let poster = dir.path().join("poster.png");
        let video = dir.path().join("clip.MP4");
        fs::write(&poster, b"POSTER").unwrap();
        fs::write(&video, b"VIDEO").unwrap();
        Fixture {
            root: dir.path().join("repo").to_string_lossy().into_owned(),
            poster: poster.to_string_lossy().into_owned(),
            video: video.to_string_lossy().into_owned(),
            dir,
        }
    }

    fn request(fx: &Fixture, slug: &str, run_id: &str) -> SinkMovieRequest {
        SinkMovieRequest {
            root: fx.root.clone(),
            feature: "login".into(),
            slug: slug.into(),
            run_id: run_id.into(),
            title: None,
            description: None,
            status: None,
            source: MovieSourceKind::Frames,
            poster_path: fx.poster.clone(),
            video_path: fx.video.clone(),
            duration_ms: 1500.4,
            chapters: vec![],
            size: None,
        }
    }

    /// Replace the wall-clock `captured_at` so bytes can be compared exactly.
    fn normalized(manifest_path: &str) -> String {
        let text = fs::read_to_string(manifest_path).unwrap();
        let re = regex::Regex::new(r#""captured_at": "\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z""#)
            .unwrap();
        re.replace_all(&text, r#""captured_at": "T""#).into_owned()
    }

    #[test]
    fn sink_movie_writes_files_and_exact_manifest_bytes() {
        let fx = fixture();
        let mut req = request(&fx, "hello-world", "run-1");
        req.description = Some("Desc".into());
        req.size = Some(Size {
            width: 640,
            height: 360,
        });
        req.chapters = vec![
            MovieChapter {
                slug: "start".into(),
                t_ms: 0.0,
                note: None,
            },
            MovieChapter {
                slug: "end".into(),
                t_ms: 1499.5,
                note: Some("done".into()),
            },
        ];
        let result = sink_movie(&req).unwrap();

        let dir = format!("{}/.astroshot/login", fx.root);
        assert_eq!(result.sequence, "0001");
        assert_eq!(result.feature_dir, dir);
        assert_eq!(result.poster_dest, format!("{dir}/0001-hello-world.png"));
        assert_eq!(result.video_dest, format!("{dir}/0001-hello-world.mp4"));
        assert_eq!(result.manifest_path, format!("{dir}/manifest.json"));
        assert_eq!(fs::read(&result.poster_dest).unwrap(), b"POSTER");
        assert_eq!(fs::read(&result.video_dest).unwrap(), b"VIDEO");
        let expected = r#"{
  "version": 1,
  "feature": "login",
  "run_id": "run-1",
  "status": "running",
  "description": "Desc",
  "shots": [
    {
      "id": "0001",
      "file": "0001-hello-world.png",
      "slug": "hello-world",
      "title": "Hello World",
      "description": "Desc",
      "captured_at": "T",
      "viewport": "640x360",
      "kind": "movie",
      "video": "0001-hello-world.mp4",
      "duration_ms": 1500,
      "source": "frames",
      "chapters": [
        {
          "slug": "start",
          "t_ms": 0
        },
        {
          "slug": "end",
          "t_ms": 1500,
          "note": "done"
        }
      ]
    }
  ]
}
"#;
        assert_eq!(normalized(&result.manifest_path), expected);
        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn sink_movie_minimal_manifest_omits_optional_keys() {
        let fx = fixture();
        let mut req = request(&fx, "a", "r");
        req.title = Some("Custom".into());
        req.status = Some(ManifestStatus::Pass);
        req.source = MovieSourceKind::DesktopWindow;
        let result = sink_movie(&req).unwrap();
        let expected = r#"{
  "version": 1,
  "feature": "login",
  "run_id": "r",
  "status": "pass",
  "shots": [
    {
      "id": "0001",
      "file": "0001-a.png",
      "slug": "a",
      "title": "Custom",
      "captured_at": "T",
      "kind": "movie",
      "video": "0001-a.mp4",
      "duration_ms": 1500,
      "source": "desktop.window",
      "chapters": []
    }
  ]
}
"#;
        assert_eq!(normalized(&result.manifest_path), expected);
    }

    #[test]
    fn sink_movie_appends_to_running_manifest_of_same_run() {
        let fx = fixture();
        sink_movie(&request(&fx, "one", "run-1")).unwrap();
        let mut second = request(&fx, "two", "run-1");
        second.description = Some("Later".into());
        second.status = Some(ManifestStatus::Fail);
        let result = sink_movie(&second).unwrap();
        assert_eq!(result.sequence, "0002");
        let text = normalized(&result.manifest_path);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["shots"].as_array().unwrap().len(), 2);
        assert_eq!(value["shots"][1]["id"], "0002");
        assert_eq!(value["status"], "fail");
        assert_eq!(value["run_id"], "run-1");
        // Existing key order is kept; a description added later lands after
        // `shots`, as with the TS object spread.
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "version",
                "feature",
                "run_id",
                "status",
                "shots",
                "description"
            ]
        );
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn sink_movie_starts_a_new_manifest_for_a_new_run_or_finished_status() {
        let fx = fixture();
        let mut first = request(&fx, "one", "run-1");
        first.status = Some(ManifestStatus::Pass);
        sink_movie(&first).unwrap();
        // Same run id but status is not running: replaced, not appended.
        let result = sink_movie(&request(&fx, "two", "run-1")).unwrap();
        let value: Value = serde_json::from_str(&normalized(&result.manifest_path)).unwrap();
        assert_eq!(value["shots"].as_array().unwrap().len(), 1);
        assert_eq!(value["shots"][0]["id"], "0002");
        // Different run id replaces too.
        let result = sink_movie(&request(&fx, "three", "run-2")).unwrap();
        let value: Value = serde_json::from_str(&normalized(&result.manifest_path)).unwrap();
        assert_eq!(value["run_id"], "run-2");
        assert_eq!(value["shots"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn sink_movie_validates_inputs() {
        let fx = fixture();
        let mut bad = request(&fx, "Bad Slug", "r");
        assert_eq!(
            sink_movie(&bad).unwrap_err().to_string(),
            "slug must be kebab-case [a-z0-9-]+, got \"Bad Slug\""
        );
        bad = request(&fx, "ok", "r");
        bad.feature = "Nope".into();
        assert!(
            sink_movie(&bad)
                .unwrap_err()
                .to_string()
                .starts_with("feature must be kebab-case")
        );
        let mut missing = request(&fx, "ok", "r");
        missing.poster_path = format!("{}/none.png", fx.dir.path().display());
        assert_eq!(
            sink_movie(&missing).unwrap_err().to_string(),
            format!("poster not found: {}", missing.poster_path)
        );
        let mut missing = request(&fx, "ok", "r");
        missing.video_path = format!("{}/none.webm", fx.dir.path().display());
        assert_eq!(
            sink_movie(&missing).unwrap_err().to_string(),
            format!("video not found: {}", missing.video_path)
        );
    }

    #[test]
    fn sink_movie_defaults_extension_to_webm() {
        let fx = fixture();
        let bare = fx.dir.path().join("noext");
        fs::write(&bare, b"V").unwrap();
        let mut req = request(&fx, "x", "r");
        req.video_path = bare.to_string_lossy().into_owned();
        let result = sink_movie(&req).unwrap();
        assert!(result.video_dest.ends_with("0001-x.webm"));
    }

    #[test]
    fn finalize_manifest_creates_when_missing() {
        let fx = fixture();
        finalize_manifest(&fx.root, "login", "run-9", ManifestStatus::Idle).unwrap();
        let path = format!("{}/.astroshot/login/manifest.json", fx.root);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "{\n  \"version\": 1,\n  \"feature\": \"login\",\n  \"run_id\": \"run-9\",\n  \"status\": \"idle\",\n  \"shots\": []\n}\n"
        );
    }

    #[test]
    fn finalize_manifest_updates_status_in_place_and_checks_run_id() {
        let fx = fixture();
        let result = sink_movie(&request(&fx, "one", "run-1")).unwrap();
        finalize_manifest(&fx.root, "login", "run-1", ManifestStatus::Pass).unwrap();
        let value: Value = serde_json::from_str(&normalized(&result.manifest_path)).unwrap();
        assert_eq!(value["status"], "pass");
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version", "feature", "run_id", "status", "shots"]);
        assert_eq!(
            finalize_manifest(&fx.root, "login", "other", ManifestStatus::Fail)
                .unwrap_err()
                .to_string(),
            "manifest run_id \"run-1\" does not match \"other\""
        );
        assert!(
            finalize_manifest(&fx.root, "Bad", "r", ManifestStatus::Fail)
                .unwrap_err()
                .to_string()
                .starts_with("feature must be kebab-case")
        );
    }

    // ---- sink_still -------------------------------------------------------

    fn still_request(fx: &Fixture, slug: &str) -> SinkStillRequest {
        let image = fx.dir.path().join("shot.png");
        fs::write(
            &image,
            crate::movie_harness::png::encode_solid_png(4, 4, [1, 2, 3]),
        )
        .unwrap();
        SinkStillRequest::new(
            fx.root.clone(),
            "login",
            slug,
            image.to_string_lossy().into_owned(),
        )
    }

    fn manifest_value(path: &str) -> Value {
        serde_json::from_str(&normalized(path)).unwrap()
    }

    #[test]
    fn sink_still_creates_the_manifest_with_exact_bytes() {
        let fx = fixture();
        let mut req = still_request(&fx, "sign-in");
        req.run_id = Some("run-1".into());
        req.description = Some("The form".into());
        req.url = Some("/login".into());
        req.viewport = Some("1280x800".into());
        let result = sink_still(&req).unwrap();

        let dir = format!("{}/.astroshot/login", fx.root);
        assert_eq!(result.sequence, "0001");
        assert_eq!(result.image_dest, format!("{dir}/0001-sign-in.png"));
        assert_eq!(result.manifest_path, format!("{dir}/manifest.json"));
        assert_eq!(result.feature_dir, dir);
        assert_eq!(result.run_id, "run-1");
        assert_eq!(
            fs::read(&result.image_dest).unwrap(),
            fs::read(&req.image_path).unwrap()
        );
        let text = fs::read_to_string(&result.manifest_path).unwrap();
        let stamp =
            regex::Regex::new(r#""captured_at": "\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ""#).unwrap();
        assert_eq!(
            stamp.replace_all(&text, r#""captured_at": "T""#),
            r#"{
  "version": 1,
  "feature": "login",
  "run_id": "run-1",
  "status": "running",
  "shots": [
    {
      "id": "0001",
      "file": "0001-sign-in.png",
      "slug": "sign-in",
      "title": "Sign In",
      "description": "The form",
      "captured_at": "T",
      "url": "/login",
      "viewport": "1280x800"
    }
  ]
}
"#
        );
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn sink_still_defaults_description_to_empty_and_omits_url_and_viewport() {
        let fx = fixture();
        let result = sink_still(&still_request(&fx, "one")).unwrap();
        let value = manifest_value(&result.manifest_path);
        let shot = value["shots"][0].as_object().unwrap();
        let keys: Vec<&String> = shot.keys().collect();
        assert_eq!(
            keys,
            ["id", "file", "slug", "title", "description", "captured_at"]
        );
        assert_eq!(shot["description"], "");
        assert_eq!(shot["title"], "One");
        assert!(result.run_id.starts_with("login-"), "{}", result.run_id);
        assert_eq!(value["run_id"], result.run_id.as_str());
    }

    #[test]
    fn sink_still_appends_to_a_running_manifest_and_numbers_after_movies() {
        let fx = fixture();
        let first = sink_still(&still_request(&fx, "one")).unwrap();
        let second = sink_still(&still_request(&fx, "two")).unwrap();
        assert_eq!(second.sequence, "0002");
        assert_eq!(second.run_id, first.run_id);
        let value = manifest_value(&second.manifest_path);
        assert_eq!(value["shots"].as_array().unwrap().len(), 2);
        assert_eq!(value["shots"][1]["file"], "0002-two.png");
        assert_eq!(value["status"], "running");

        // The same sequence counter is shared with movies.
        let movie = sink_movie(&request(&fx, "clip", &first.run_id)).unwrap();
        assert_eq!(movie.sequence, "0003");
        let third = sink_still(&still_request(&fx, "three")).unwrap();
        assert_eq!(third.sequence, "0004");
        let value = manifest_value(&third.manifest_path);
        assert_eq!(value["shots"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn sink_still_starts_a_fresh_manifest_after_a_finished_run() {
        let fx = fixture();
        let mut first = still_request(&fx, "one");
        first.status = Some(ManifestStatus::Pass);
        let first = sink_still(&first).unwrap();
        assert_eq!(manifest_value(&first.manifest_path)["status"], "pass");

        let second = sink_still(&still_request(&fx, "two")).unwrap();
        let value = manifest_value(&second.manifest_path);
        assert_eq!(value["status"], "running");
        let shots = value["shots"].as_array().unwrap();
        assert_eq!(shots.len(), 1);
        // The old image stays on disk as prior-run evidence; numbering goes on.
        assert_eq!(shots[0]["id"], "0002");
        assert!(Path::new(&first.image_dest).exists());
    }

    #[test]
    fn sink_still_with_a_matching_run_id_continues_even_after_finalize() {
        let fx = fixture();
        let mut first = still_request(&fx, "one");
        first.run_id = Some("run-1".into());
        sink_still(&first).unwrap();
        finalize_manifest(&fx.root, "login", "run-1", ManifestStatus::Pass).unwrap();

        let mut again = still_request(&fx, "two");
        again.run_id = Some("run-1".into());
        let result = sink_still(&again).unwrap();
        let value = manifest_value(&result.manifest_path);
        assert_eq!(value["shots"].as_array().unwrap().len(), 2);
        assert_eq!(value["status"], "running");

        // A different explicit id starts a new run.
        let mut other = still_request(&fx, "three");
        other.run_id = Some("run-2".into());
        other.status = Some(ManifestStatus::Fail);
        let result = sink_still(&other).unwrap();
        let value = manifest_value(&result.manifest_path);
        assert_eq!(value["run_id"], "run-2");
        assert_eq!(value["status"], "fail");
        assert_eq!(value["shots"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn sink_still_rejects_bad_images_without_leaving_files_behind() {
        let fx = fixture();
        let dir = format!("{}/.astroshot/login", fx.root);

        let mut truncated = still_request(&fx, "bad");
        let bytes = crate::movie_harness::png::encode_solid_png(4, 4, [1, 2, 3]);
        let broken = fx.dir.path().join("broken.png");
        fs::write(&broken, &bytes[..bytes.len() - 6]).unwrap();
        truncated.image_path = broken.to_string_lossy().into_owned();
        assert_eq!(
            sink_still(&truncated).unwrap_err().to_string(),
            format!("not a valid .png image: {}", truncated.image_path)
        );

        let mut missing = still_request(&fx, "bad");
        missing.image_path = format!("{}/none.png", fx.dir.path().display());
        assert_eq!(
            sink_still(&missing).unwrap_err().to_string(),
            format!("image not found: {}", missing.image_path)
        );

        let text = fx.dir.path().join("notes.txt");
        fs::write(&text, b"hello").unwrap();
        let mut unsupported = still_request(&fx, "bad");
        unsupported.image_path = text.to_string_lossy().into_owned();
        assert!(
            sink_still(&unsupported)
                .unwrap_err()
                .to_string()
                .starts_with("unsupported image extension .txt")
        );

        assert!(!Path::new(&dir).join("manifest.json").exists());
        assert!(
            fs::read_dir(&dir)
                .map(|entries| entries.count() == 0)
                .unwrap_or(true)
        );
    }

    #[test]
    fn sink_still_rejects_reserved_features_and_bad_names() {
        let fx = fixture();
        for reserved in ["stories", "friction-logs"] {
            let mut req = still_request(&fx, "one");
            req.feature = reserved.into();
            let error = sink_still(&req).unwrap_err().to_string();
            assert!(error.contains("reserved for user stories"), "{error}");
        }
        assert!(!Path::new(&format!("{}/.astroshot", fx.root)).exists());

        let mut bad = still_request(&fx, "Bad Slug");
        assert_eq!(
            sink_still(&bad).unwrap_err().to_string(),
            "slug must be [a-z0-9][a-z0-9_-]*, got \"Bad Slug\""
        );
        bad = still_request(&fx, "ok");
        bad.feature = "Nope".into();
        assert!(
            sink_still(&bad)
                .unwrap_err()
                .to_string()
                .starts_with("feature must be kebab-case")
        );
        bad = still_request(&fx, "ok");
        bad.run_id = Some(String::new());
        assert_eq!(
            sink_still(&bad).unwrap_err().to_string(),
            "run_id must not be empty"
        );
    }

    #[test]
    fn sink_still_validates_each_supported_format() {
        let jpg = [&[0xff, 0xd8, 0xff][..], &[0u8; 20], &[0xff, 0xd9]].concat();
        assert!(is_valid_image(&jpg, "jpg") && is_valid_image(&jpg, "jpeg"));
        assert!(!is_valid_image(&jpg[..jpg.len() - 1], "jpg"));
        let gif = [&b"GIF89a"[..], &[0u8; 8], &[0x3b]].concat();
        assert!(is_valid_image(&gif, "gif"));
        assert!(!is_valid_image(&gif[..gif.len() - 1], "gif"));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        webp.extend_from_slice(&[0; 4]);
        let declared = (webp.len() - 8) as u32;
        webp[4..8].copy_from_slice(&declared.to_le_bytes());
        assert!(is_valid_image(&webp, "webp"));
        webp.push(0);
        assert!(!is_valid_image(&webp, "webp"));
        assert!(!is_valid_image(b"anything", "bmp"));
    }

    // ---- capture lock, helper parity (ports of test-astroshot-capture) -----

    use std::time::Instant;

    fn feature_path(fx: &Fixture, feature: &str) -> String {
        format!("{}/.astroshot/{feature}", fx.root)
    }

    fn with_run(mut req: SinkStillRequest, run_id: &str) -> SinkStillRequest {
        req.run_id = Some(run_id.into());
        req
    }

    fn residue(dir: &str) -> Vec<String> {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|name| {
                        name.starts_with(".capture") || name.starts_with(".manifest.tmp")
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Take the lock the way the bash helper does: a directory with our pid.
    fn hold_bash_style_lock(dir: &str, pid: &str) -> std::path::PathBuf {
        let lock = Path::new(dir).join(".capture.lock");
        fs::create_dir_all(&lock).unwrap();
        fs::write(lock.join("pid"), format!("{pid}\n")).unwrap();
        lock
    }

    #[test]
    fn sink_still_accepts_underscore_slugs_and_movies_keep_rejecting_them() {
        let fx = fixture();
        let result = sink_still(&still_request(&fx, "step_1-done")).unwrap();
        assert!(result.image_dest.ends_with("/0001-step_1-done.png"));
        let value = manifest_value(&result.manifest_path);
        assert_eq!(value["shots"][0]["slug"], "step_1-done");
        assert_eq!(value["shots"][0]["title"], "Step 1 Done");

        assert_eq!(
            sink_movie(&request(&fx, "step_1", "r"))
                .unwrap_err()
                .to_string(),
            "slug must be kebab-case [a-z0-9-]+, got \"step_1\""
        );
        for bad in ["_x", "-x", "X", "a.b", ""] {
            assert!(sink_still(&still_request(&fx, bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn sink_still_rejects_unsupported_extensionless_and_corrupt_sources_without_state() {
        let fx = fixture();
        let png = crate::movie_harness::png::encode_solid_png(4, 4, [1, 2, 3]);
        let source = |name: &str, bytes: &[u8]| {
            let path = fx.dir.path().join(name);
            fs::write(&path, bytes).unwrap();
            path.to_string_lossy().into_owned()
        };
        let mut cases = vec![
            ("unsupported-source", source("source.txt", b"plain text\n")),
            ("extensionless-source", source("source", &png)),
            (
                "corrupt-source",
                source("corrupt.png", b"not actually a png\n"),
            ),
        ];
        // Structure intact (signature, IHDR, IEND) but a flipped byte inside
        // the IDAT data: only decoding finds it.
        let mut damaged = png.clone();
        damaged[33 + 8 + 2] ^= 0xff;
        assert!(is_structurally_valid_image(&damaged, "png"));
        cases.push(("damaged-source", source("damaged.png", &damaged)));
        // Cut inside the pixel data, with a valid IEND chunk glued back on.
        let mut truncated = png[..png.len() - 12 - 6].to_vec();
        truncated.extend_from_slice(&png[png.len() - 12..]);
        assert!(is_structurally_valid_image(&truncated, "png"));
        cases.push(("truncated-source", source("truncated.png", &truncated)));

        for (feature, path) in cases {
            let mut req = still_request(&fx, "rejected");
            req.feature = feature.into();
            req.image_path = path.clone();
            assert!(sink_still(&req).is_err(), "{feature}");
            assert!(
                !Path::new(&feature_path(&fx, feature)).exists(),
                "{feature}"
            );
        }
        let mut req = still_request(&fx, "rejected");
        req.image_path = source("source", &png);
        assert!(
            sink_still(&req)
                .unwrap_err()
                .to_string()
                .starts_with("image must have a supported extension")
        );
    }

    #[test]
    fn sink_still_numbers_like_the_helper() {
        let fx = fixture();
        let dir = feature_path(&fx, "login");
        fs::create_dir_all(&dir).unwrap();
        // Counted: any digit-only prefix, lower-case image extension.
        for name in ["0007-a.gif", "12-b.jpeg"] {
            fs::write(format!("{dir}/{name}"), "").unwrap();
        }
        // Not counted by the helper: other extensions, upper case, no dash.
        for name in [
            "0050-notes.txt",
            "0060-x.PNG",
            "0070-x.mp4",
            "0080.png",
            "x-0090.png",
        ] {
            fs::write(format!("{dir}/{name}"), "").unwrap();
        }
        assert_eq!(next_still_sequence(&dir, "login").unwrap(), "0013");
        fs::write(format!("{dir}/9999-existing.png"), "").unwrap();
        assert_eq!(
            next_still_sequence(&dir, "login").unwrap_err().to_string(),
            "capture sequence limit 9999 reached for feature login; start a new feature"
        );
    }

    #[test]
    fn sink_still_at_the_sequence_limit_fails_clearly_and_leaves_nothing() {
        let fx = fixture();
        let dir = feature_path(&fx, "login");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            format!("{dir}/9999-existing.png"),
            crate::movie_harness::png::encode_solid_png(4, 4, [1, 2, 3]),
        )
        .unwrap();
        assert_eq!(
            sink_still(&still_request(&fx, "ten-thousand"))
                .unwrap_err()
                .to_string(),
            "capture sequence limit 9999 reached for feature login; start a new feature"
        );
        assert!(!Path::new(&format!("{dir}/10000-ten-thousand.png")).exists());
        assert!(!Path::new(&format!("{dir}/manifest.json")).exists());
        assert_eq!(residue(&dir), Vec::<String>::new());
    }

    #[test]
    fn sink_still_with_a_broken_manifest_publishes_nothing_and_releases_the_lock() {
        let fx = fixture();
        let dir = feature_path(&fx, "login");
        fs::create_dir_all(&dir).unwrap();
        fs::write(format!("{dir}/manifest.json"), "{invalid json\n").unwrap();
        assert!(sink_still(&still_request(&fx, "rollback")).is_err());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["manifest.json"]);
        assert_eq!(
            fs::read_to_string(format!("{dir}/manifest.json")).unwrap(),
            "{invalid json\n"
        );
    }

    #[test]
    fn sink_still_matching_run_id_continues_and_a_new_one_resets_like_the_helper() {
        let fx = fixture();
        let first = sink_still(&still_request(&fx, "first")).unwrap();
        finalize_current_run(&fx.root, "login", ManifestStatus::Pass, None).unwrap();

        // After finalize a capture without a run id starts another run.
        let second = sink_still(&still_request(&fx, "second")).unwrap();
        assert_ne!(second.run_id, first.run_id);
        let value = manifest_value(&second.manifest_path);
        assert_eq!(value["status"], "running");
        assert_eq!(value["shots"].as_array().unwrap().len(), 1);
        assert_eq!(value["shots"][0]["id"], "0002");
        assert!(Path::new(&first.image_dest).exists());

        // The active explicit id continues the run.
        sink_still(&with_run(still_request(&fx, "third"), &second.run_id)).unwrap();
        let value = manifest_value(&second.manifest_path);
        assert_eq!(value["run_id"], second.run_id.as_str());
        let ids: Vec<&str> = value["shots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|shot| shot["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["0002", "0003"]);

        // Another explicit id resets the shots.
        sink_still(&with_run(
            still_request(&fx, "replacement"),
            "replacement-run",
        ))
        .unwrap();
        let value = manifest_value(&second.manifest_path);
        assert_eq!(value["run_id"], "replacement-run");
        assert_eq!(value["shots"].as_array().unwrap().len(), 1);
        assert_eq!(value["shots"][0]["id"], "0004");

        // Finalize may name the active run, and rejects another.
        finalize_manifest(&fx.root, "login", "replacement-run", ManifestStatus::Pass).unwrap();
        assert!(finalize_manifest(&fx.root, "login", "wrong-run", ManifestStatus::Fail).is_err());
        let value = manifest_value(&second.manifest_path);
        assert_eq!(value["status"], "pass");
        assert_eq!(value["shots"].as_array().unwrap().len(), 1);

        // The matching id resumes the finalized run.
        sink_still(&with_run(
            still_request(&fx, "explicit-resume"),
            "replacement-run",
        ))
        .unwrap();
        let value = manifest_value(&second.manifest_path);
        assert_eq!(value["status"], "running");
        assert_eq!(value["shots"].as_array().unwrap().len(), 2);
        assert_eq!(value["shots"][1]["id"], "0005");
    }

    #[test]
    fn sink_still_edits_an_existing_manifest_in_place_and_keeps_other_keys() {
        let fx = fixture();
        let dir = feature_path(&fx, "login");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            format!("{dir}/manifest.json"),
            r#"{
  "version": 1,
  "description": "kept",
  "status": "pass",
  "feature": "login",
  "custom": {
    "a": [1, 2]
  },
  "run_id": "old-run",
  "shots": [
    {
      "id": "0001",
      "file": "0001-old.png",
      "slug": "old"
    }
  ]
}
"#,
        )
        .unwrap();

        // The old run is finished, so this starts a new one: run_id, status
        // and shots are reset in place, everything else stays where it was.
        let mut req = still_request(&fx, "fresh");
        req.run_id = Some("new-run".into());
        let result = sink_still(&req).unwrap();
        let text = fs::read_to_string(&result.manifest_path).unwrap();
        let stamp =
            regex::Regex::new(r#""captured_at": "\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ""#).unwrap();
        assert_eq!(
            stamp.replace_all(&text, r#""captured_at": "T""#),
            r#"{
  "version": 1,
  "description": "kept",
  "status": "running",
  "feature": "login",
  "custom": {
    "a": [
      1,
      2
    ]
  },
  "run_id": "new-run",
  "shots": [
    {
      "id": "0001",
      "file": "0001-fresh.png",
      "slug": "fresh",
      "title": "Fresh",
      "description": "",
      "captured_at": "T"
    }
  ]
}
"#
        );

        // Missing status, run_id and shots are appended in that order.
        fs::write(
            format!("{dir}/manifest.json"),
            "{\n  \"feature\": \"login\"\n}\n",
        )
        .unwrap();
        let result = sink_still(&still_request(&fx, "bare")).unwrap();
        let value = manifest_value(&result.manifest_path);
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["feature", "status", "run_id", "shots"]);
    }

    #[test]
    fn sink_still_generates_a_new_run_id_even_within_one_second() {
        let fx = fixture();
        let mut first = still_request(&fx, "one");
        first.status = Some(ManifestStatus::Pass);
        let first = sink_still(&first).unwrap();
        let mut second = still_request(&fx, "two");
        second.status = Some(ManifestStatus::Pass);
        let second = sink_still(&second).unwrap();
        let third = sink_still(&still_request(&fx, "three")).unwrap();
        assert_ne!(first.run_id, second.run_id);
        assert_ne!(second.run_id, third.run_id);
        assert!(second.run_id.starts_with(&first.run_id));
    }

    #[test]
    fn finalize_current_run_sets_the_status_and_adds_no_shot() {
        let fx = fixture();
        let result = sink_still(&still_request(&fx, "one")).unwrap();
        let before = manifest_value(&result.manifest_path);

        let done = finalize_current_run(&fx.root, "login", ManifestStatus::Pass, None).unwrap();
        assert_eq!(
            done,
            FinalizeCurrentRunResult {
                run_id: Some(result.run_id.clone()),
                manifest_path: result.manifest_path.clone(),
                created: false,
            }
        );
        let after = manifest_value(&result.manifest_path);
        assert_eq!(after["status"], "pass");
        assert_eq!(after["shots"], before["shots"]);
        assert_eq!(after["run_id"], before["run_id"]);
        let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version", "feature", "run_id", "status", "shots"]);
        assert_eq!(residue(&feature_path(&fx, "login")), Vec::<String>::new());
    }

    #[test]
    fn finalize_current_run_creates_a_missing_manifest_and_checks_names() {
        let fx = fixture();
        let done = finalize_current_run(&fx.root, "fresh", ManifestStatus::Fail, None).unwrap();
        assert!(done.created);
        let run_id = done.run_id.clone().unwrap();
        assert!(run_id.starts_with("fresh-"), "{run_id}");
        assert_eq!(
            fs::read_to_string(&done.manifest_path).unwrap(),
            format!(
                "{{\n  \"version\": 1,\n  \"feature\": \"fresh\",\n  \"run_id\": \"{run_id}\",\n  \"status\": \"fail\",\n  \"shots\": []\n}}\n"
            )
        );

        // A manifest without a run id gets its status set and nothing else.
        let dir = feature_path(&fx, "bare");
        fs::create_dir_all(&dir).unwrap();
        fs::write(format!("{dir}/manifest.json"), "{\"feature\":\"bare\"}").unwrap();
        let done = finalize_current_run(&fx.root, "bare", ManifestStatus::Idle, None).unwrap();
        assert_eq!(done.run_id, None);
        assert_eq!(
            fs::read_to_string(&done.manifest_path).unwrap(),
            "{\n  \"feature\": \"bare\",\n  \"status\": \"idle\"\n}\n"
        );

        for reserved in ["stories", "friction-logs"] {
            let error = finalize_current_run(&fx.root, reserved, ManifestStatus::Pass, None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("reserved for user stories"), "{error}");
        }
        assert!(
            finalize_current_run(&fx.root, "Bad_Name", ManifestStatus::Pass, None)
                .unwrap_err()
                .to_string()
                .starts_with("feature must be kebab-case")
        );
        assert!(!Path::new(&feature_path(&fx, "stories")).exists());
    }

    #[test]
    fn sink_still_waits_on_a_bash_style_lock_then_times_out_with_a_clear_error() {
        let fx = fixture();
        let dir = feature_path(&fx, "locked");
        fs::create_dir_all(&dir).unwrap();
        // A live owner: this test process stands in for a running helper.
        let lock = hold_bash_style_lock(&dir, &std::process::id().to_string());

        let mut req = still_request(&fx, "blocked");
        req.feature = "locked".into();
        req.lock_timeout = Some(Duration::from_secs(1));
        let started = Instant::now();
        let error = sink_still(&req).unwrap_err().to_string();
        assert_eq!(
            error,
            format!(
                "timed out after 1s waiting for capture lock: {}",
                lock.display()
            )
        );
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert!(lock.exists());
        assert!(!Path::new(&format!("{dir}/manifest.json")).exists());
        assert!(!Path::new(&format!("{dir}/0001-blocked.png")).exists());

        // The other entry points wait on the same lock.
        let mut movie = request(&fx, "clip", "r");
        movie.feature = "locked".into();
        assert!(
            sink_movie_with_lock_timeout(&movie, Some(Duration::from_secs(1)))
                .unwrap_err()
                .to_string()
                .starts_with("timed out after 1s waiting for capture lock")
        );
        assert!(
            finalize_manifest_with_lock_timeout(
                &fx.root,
                "locked",
                "r",
                ManifestStatus::Pass,
                Some(Duration::from_secs(1))
            )
            .unwrap_err()
            .to_string()
            .starts_with("timed out after 1s waiting for capture lock")
        );
        assert!(
            finalize_current_run(
                &fx.root,
                "locked",
                ManifestStatus::Pass,
                Some(Duration::from_secs(1))
            )
            .unwrap_err()
            .to_string()
            .starts_with("timed out after 1s waiting for capture lock")
        );
        assert!(!Path::new(&format!("{dir}/0001-clip.png")).exists());

        // Released while a capture waits: it goes through.
        let releaser = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(500));
                fs::remove_dir_all(lock).unwrap();
            })
        };
        let mut waiting = still_request(&fx, "after");
        waiting.feature = "locked".into();
        waiting.lock_timeout = Some(Duration::from_secs(30));
        let started = Instant::now();
        let result = sink_still(&waiting).unwrap();
        releaser.join().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert_eq!(result.sequence, "0001");
        assert_eq!(residue(&dir), Vec::<String>::new());
    }

    #[test]
    fn sink_still_reports_a_lock_from_a_dead_process_as_stale() {
        let fx = fixture();
        let dir = feature_path(&fx, "stale");
        fs::create_dir_all(&dir).unwrap();
        let lock = hold_bash_style_lock(&dir, "99999999");
        let mut req = still_request(&fx, "recovered");
        req.feature = "stale".into();
        let started = Instant::now();
        let error = sink_still(&req).unwrap_err().to_string();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            error,
            format!(
                "stale capture lock from dead pid 99999999: {}; remove it and retry",
                lock.display()
            )
        );
        assert!(error.contains(".capture.lock; remove it and retry"));
        assert!(lock.exists());
    }

    #[test]
    fn concurrent_sink_stills_get_distinct_sequences_and_manifest_entries() {
        let fx = fixture();
        let count = 12;
        let dir = feature_path(&fx, "login");
        let source = still_request(&fx, "seed").image_path;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Watch the manifest while writers publish: every visible manifest
        // parses and names only images that exist.
        let observer = {
            let stop = stop.clone();
            let dir = dir.clone();
            std::thread::spawn(move || {
                let mut seen = 0;
                while !stop.load(Ordering::SeqCst) {
                    if let Ok(text) = fs::read_to_string(format!("{dir}/manifest.json")) {
                        let value: Value = serde_json::from_str(&text).expect("valid manifest");
                        for shot in value["shots"].as_array().unwrap() {
                            let file = shot["file"].as_str().unwrap();
                            assert!(Path::new(&format!("{dir}/{file}")).exists(), "{file}");
                        }
                        seen += 1;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                seen
            })
        };
        let writers: Vec<_> = (0..count)
            .map(|n| {
                let mut req = SinkStillRequest::new(
                    fx.root.clone(),
                    "login",
                    format!("shot-{n}"),
                    source.clone(),
                );
                req.description = Some(format!("concurrent capture {n}"));
                std::thread::spawn(move || sink_still(&req).unwrap())
            })
            .collect();
        let results: Vec<SinkStillResult> =
            writers.into_iter().map(|w| w.join().unwrap()).collect();
        stop.store(true, Ordering::SeqCst);
        observer.join().unwrap();

        let mut sequences: Vec<&str> = results.iter().map(|r| r.sequence.as_str()).collect();
        sequences.sort_unstable();
        let expected: Vec<String> = (1..=count).map(|n| format!("{n:04}")).collect();
        assert_eq!(sequences, expected);

        let value: Value =
            serde_json::from_str(&fs::read_to_string(format!("{dir}/manifest.json")).unwrap())
                .unwrap();
        let shots = value["shots"].as_array().unwrap();
        assert_eq!(shots.len(), count);
        let mut ids: Vec<&str> = shots.iter().map(|s| s["id"].as_str().unwrap()).collect();
        ids.sort_unstable();
        assert_eq!(ids, expected);
        let images: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".png"))
            .collect();
        assert_eq!(images.len(), count);
        for shot in shots {
            assert!(Path::new(&format!("{dir}/{}", shot["file"].as_str().unwrap())).exists());
        }
        // One run for all writers.
        let run_ids: std::collections::HashSet<&str> =
            results.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(run_ids.len(), 1);
        assert_eq!(residue(&dir), Vec::<String>::new());
    }

    #[test]
    fn concurrent_stills_and_movies_share_one_sequence() {
        let fx = fixture();
        let dir = feature_path(&fx, "login");
        let image = still_request(&fx, "seed").image_path;
        let mut handles = Vec::new();
        for n in 0..6 {
            let still =
                SinkStillRequest::new(fx.root.clone(), "login", format!("s{n}"), image.clone());
            handles.push(std::thread::spawn(move || {
                sink_still(&still).unwrap().sequence
            }));
            let mut movie = request(&fx, &format!("m{n}"), "movie-run");
            movie.poster_path = image.clone();
            handles.push(std::thread::spawn(move || {
                sink_movie(&movie).unwrap().sequence
            }));
        }
        let mut sequences: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        sequences.sort();
        let expected: Vec<String> = (1..=12).map(|n| format!("{n:04}")).collect();
        assert_eq!(sequences, expected);
        assert_eq!(residue(&dir), Vec::<String>::new());
    }
}
