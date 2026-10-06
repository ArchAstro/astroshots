//! Publish a movie into `.astroshot/<feature>/` and maintain `manifest.json`.
//!
//! Port of `packages/movie-harness/src/sink.ts`. The macOS app reads these
//! files, so bytes match `JSON.stringify(manifest, null, 2) + "\n"`: key order
//! is preserved (serde_json `preserve_order`), whole numbers have no `.0`, and
//! optional fields are omitted when absent. The manifest is edited as a JSON
//! map so keys of an existing manifest keep their position and unknown keys
//! survive, as with the TS object spread.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, bail};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};

use super::encode::{extname_lower, js_round};
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
    let continue_run = existing.as_ref().is_some_and(|existing| {
        existing.get("run_id").and_then(Value::as_str) == Some(request.run_id.as_str())
            && existing.get("status").and_then(Value::as_str) == Some("running")
    });

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
            let mut shots = match manifest.get("shots") {
                Some(Value::Array(shots)) => shots.clone(),
                _ => Vec::new(),
            };
            shots.push(Value::Object(shot));
            manifest.insert("shots".into(), Value::Array(shots));
            manifest
        }
        None => {
            let mut manifest = Map::new();
            manifest.insert("version".into(), json!(1));
            manifest.insert("feature".into(), json!(request.feature));
            manifest.insert("run_id".into(), json!(request.run_id));
            manifest.insert(
                "status".into(),
                json!(request.status.map_or("running".to_string(), status_str)),
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
        let mut manifest = Map::new();
        manifest.insert("version".into(), json!(1));
        manifest.insert("feature".into(), json!(feature));
        manifest.insert("run_id".into(), json!(run_id));
        manifest.insert("status".into(), json!(status));
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
}
