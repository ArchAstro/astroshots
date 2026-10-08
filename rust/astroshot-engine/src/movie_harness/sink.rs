//! Publish a movie into `.astroshot/<feature>/` and maintain `manifest.json`.
//!
//! Port of `packages/movie-harness/src/sink.ts`. The macOS app reads these
//! files, so bytes match `JSON.stringify(manifest, null, 2) + "\n"`: key order
//! is preserved (serde_json `preserve_order`), whole numbers have no `.0`, and
//! optional fields are omitted when absent. The manifest is edited as a JSON
//! map so keys of an existing manifest keep their position and unknown keys
//! survive, as with the TS object spread.
//!
//! [`sink_still`] is a Rust-only addition (no TS counterpart): it publishes an
//! existing image as a still shot with the same naming, run rules and manifest
//! writer, and writes the fields `skills/astroshots-review/scripts/astroshot-capture`
//! writes for a still.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, bail};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};

use super::encode::{extname_lower, js_round};
use super::paths::default_run_id;
use super::paths::{
    assert_kebab_case, assert_slug, ensure_dir, feature_dir, humanize, next_sequence,
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
pub fn sink_movie(request: &SinkMovieRequest) -> Result<SinkMovieResult> {
    assert_kebab_case(&request.feature, "feature")?;
    assert_slug(&request.slug)?;

    let dir = feature_dir(&request.root, &request.feature);
    ensure_dir(&dir)?;
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
    /// Kebab-case slug of the file name (`NNNN-<slug>.<ext>`).
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
/// signature, mandatory header chunk and terminator per format. The bash
/// helper also runs `sips`/`magick` when present; this does not decode.
fn is_valid_image(bytes: &[u8], extension: &str) -> bool {
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

/// Publish an existing image into `.astroshot/<feature>/` as `NNNN-<slug>.<ext>`
/// and append a still shot to `manifest.json`. Mirrors
/// `skills/astroshots-review/scripts/astroshot-capture --source`, except that
/// it takes no capture lock (callers serialize writers to one feature) and a
/// new run replaces the manifest instead of editing it in place.
pub fn sink_still(request: &SinkStillRequest) -> Result<SinkStillResult> {
    assert_kebab_case(&request.feature, "feature")?;
    if RESERVED_FEATURES.contains(&request.feature.as_str()) {
        bail!(
            "feature \"{}\" is reserved for user stories; pick another feature name",
            request.feature
        );
    }
    assert_slug(&request.slug)?;
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
    if !STILL_EXTENSIONS.contains(&extension.as_str()) {
        bail!(
            "unsupported image extension .{extension}; expected png|jpg|jpeg|webp|gif ({})",
            request.image_path
        );
    }
    if !is_valid_image(&fs::read(source)?, &extension) {
        bail!("not a valid .{extension} image: {}", request.image_path);
    }

    let dir = feature_dir(&request.root, &request.feature);
    ensure_dir(&dir)?;
    let sequence = next_sequence(&dir)?;
    if sequence.len() > 4 {
        bail!(
            "capture sequence limit 9999 reached for feature {}; start a new feature",
            request.feature
        );
    }
    let file_name = format!("{sequence}-{}.{extension}", request.slug);
    let image_dest = path_join(&dir, &file_name);

    let manifest_path = path_join(&dir, "manifest.json");
    let existing = read_manifest(&manifest_path)?;
    let existing_run = existing
        .as_ref()
        .and_then(|existing| existing.get("run_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let (run_id, continue_run) = match &request.run_id {
        Some(explicit) => (
            explicit.clone(),
            existing_run.as_deref() == Some(explicit.as_str()),
        ),
        None => match existing_run {
            Some(id) if continues_running_run(&existing, &id) => (id, true),
            _ => (default_run_id(&request.feature), false),
        },
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

    let mut manifest = match existing.filter(|_| continue_run) {
        Some(mut manifest) => {
            manifest.insert("status".into(), json!(status));
            manifest
        }
        None => {
            let mut manifest = new_manifest(&request.feature, &run_id, &status);
            manifest.insert("shots".into(), json!([]));
            manifest
        }
    };
    push_shot(&mut manifest, shot);

    // The manifest is the commit record: an image that never made it in is
    // removed again.
    fs::copy(source, &image_dest)?;
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

pub fn finalize_manifest(
    root: &str,
    feature: &str,
    run_id: &str,
    status: ManifestStatus,
) -> Result<()> {
    assert_kebab_case(feature, "feature")?;
    let dir = feature_dir(root, feature);
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
            "slug must be kebab-case [a-z0-9-]+, got \"Bad Slug\""
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
}
