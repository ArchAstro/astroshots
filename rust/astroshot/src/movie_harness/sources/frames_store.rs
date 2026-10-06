//! The "frames" source: a persisted PNG/JPEG frame session driven across
//! processes (start / push / mark / stop).
//!
//! Port of `packages/movie-harness/src/sources/frames-store.ts`. The state file
//! is `JSON.stringify(state, null, 2) + "\n"`, written via a `.tmp` rename.

use std::fs;
use std::path::Path;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::movie_harness::encode::{encode_frames, extname_lower, poster_from_frames};
use crate::movie_harness::paths::{
    assert_kebab_case, assert_slug, default_run_id, ensure_dir, movie_state_dir, resolve_root,
};
use crate::movie_harness::session::{list_frame_files, now_ms, random_bytes};
use crate::movie_harness::sink::sink_movie;
use crate::movie_harness::types::{
    EncodeFramesRequest, FramesSource, ManifestStatus, MovieArtifact, MovieChapter, MovieFormat,
    MovieSourceKind, PersistedFrameSession, SinkMovieRequest, Size, Version1,
};

const STATE_FILE: &str = "session.json";

fn join(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

fn session_dir(root: &str, feature: &str, id: &str) -> String {
    join(&movie_state_dir(root, feature), id)
}

fn state_path(dir: &str) -> String {
    join(dir, STATE_FILE)
}

fn write_state(state: &PersistedFrameSession) -> Result<()> {
    let dir = session_dir(&state.root, &state.feature, &state.id);
    ensure_dir(&dir)?;
    ensure_dir(&state.frame_dir)?;
    let tmp = format!("{}.tmp", state_path(&dir));
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(state)?))?;
    fs::rename(&tmp, state_path(&dir))?;
    Ok(())
}

/// `${value}` of a parsed JSON value, as JS would print it.
fn js_display(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) => "null".into(),
        Some(other) => other.to_string(),
    }
}

fn read_state(dir: &str) -> Result<PersistedFrameSession> {
    let raw: Value = serde_json::from_str(&fs::read_to_string(state_path(dir))?)?;
    if raw.get("version") != Some(&Value::from(1)) {
        bail!(
            "unsupported movie session version: {}",
            js_display(raw.get("version"))
        );
    }
    Ok(serde_json::from_value(raw)?)
}

/// Resolve the active frames session for a feature (latest if id omitted).
pub fn load_frame_session(
    root: &str,
    feature: &str,
    id: Option<&str>,
) -> Result<PersistedFrameSession> {
    let resolved_root = resolve_root(Some(root));
    assert_kebab_case(feature, "feature")?;
    let base = movie_state_dir(&resolved_root, feature);
    if !Path::new(&base).exists() {
        bail!("no movie session for feature {feature}");
    }
    if let Some(id) = id.filter(|id| !id.is_empty()) {
        let dir = session_dir(&resolved_root, feature, id);
        if !Path::new(&state_path(&dir)).exists() {
            bail!("movie session not found: {id}");
        }
        return read_state(&dir);
    }
    let mut ids: Vec<String> = fs::read_dir(&base)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    ids.retain(|name| Path::new(&state_path(&join(&base, name))).exists());
    ids.sort();
    let Some(latest) = ids.last() else {
        bail!("no movie session for feature {feature}");
    };
    read_state(&join(&base, latest))
}

#[derive(Debug, Clone, Default)]
pub struct StartFrameSessionOptions {
    pub feature: String,
    pub slug: String,
    pub root: Option<String>,
    pub run_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub size: Option<Size>,
    pub fps: Option<f64>,
    pub format: Option<MovieFormat>,
}

pub fn start_frame_session(options: StartFrameSessionOptions) -> Result<PersistedFrameSession> {
    assert_kebab_case(&options.feature, "feature")?;
    assert_slug(&options.slug)?;
    let root = resolve_root(options.root.as_deref());
    let id: String = random_bytes(6)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let dir = session_dir(&root, &options.feature, &id);
    let frame_dir = join(&dir, "frames");
    ensure_dir(&frame_dir)?;
    let state = PersistedFrameSession {
        version: Version1,
        id,
        run_id: options
            .run_id
            .unwrap_or_else(|| default_run_id(&options.feature)),
        feature: options.feature,
        slug: options.slug,
        root,
        title: options.title,
        description: options.description,
        size: options.size.unwrap_or(Size {
            width: 1280,
            height: 720,
        }),
        fps: options.fps.unwrap_or(15.0),
        format: options.format.unwrap_or(MovieFormat::Webm),
        source: FramesSource,
        started_at_ms: now_ms(),
        frame_dir,
        frame_count: 0,
        chapters: Vec::new(),
    };
    write_state(&state)?;
    Ok(state)
}

pub fn push_frame_to_session(
    state: &PersistedFrameSession,
    image_path: &str,
) -> Result<PersistedFrameSession> {
    if !Path::new(image_path).exists() {
        bail!("frame not found: {image_path}");
    }
    let mut ext = extname_lower(image_path).replacen('.', "", 1);
    if ext.is_empty() {
        ext = "png".into();
    }
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
        bail!("unsupported frame extension .{ext}");
    }
    let index = format!("{:06}", state.frame_count);
    let dest = join(&state.frame_dir, &format!("{index}.{ext}"));
    fs::copy(image_path, dest)?;
    let mut next = state.clone();
    next.frame_count = state.frame_count + 1;
    write_state(&next)?;
    Ok(next)
}

pub fn mark_frame_session(
    state: &PersistedFrameSession,
    slug: &str,
    note: Option<&str>,
) -> Result<PersistedFrameSession> {
    assert_slug(slug)?;
    let chapter = MovieChapter {
        slug: slug.to_string(),
        t_ms: now_ms().saturating_sub(state.started_at_ms) as f64,
        note: note.map(str::to_string),
    };
    let mut next = state.clone();
    next.chapters.push(chapter);
    write_state(&next)?;
    Ok(next)
}

pub async fn stop_frame_session(
    state: &PersistedFrameSession,
    status: Option<ManifestStatus>,
) -> Result<MovieArtifact> {
    let frames = list_frame_files(Path::new(&state.frame_dir))?;
    if frames.is_empty() {
        bail!("cannot stop movie session with zero frames");
    }

    let session = session_dir(&state.root, &state.feature, &state.id);
    let work = join(&session, "out");
    ensure_dir(&work)?;
    let out_ext = if state.format == MovieFormat::Mp4 {
        ".mp4"
    } else {
        ".webm"
    };
    let encoded = encode_frames(&EncodeFramesRequest {
        frame_paths: frames.clone(),
        out_path: join(&work, &format!("movie{out_ext}")),
        size: state.size,
        fps: state.fps,
        duration_ms: None,
    })
    .await?;
    let poster_path = poster_from_frames(&frames, &join(&work, "poster.png"))?;
    let duration_ms = encoded.duration_ms;

    let published = sink_movie(&SinkMovieRequest {
        root: state.root.clone(),
        feature: state.feature.clone(),
        slug: state.slug.clone(),
        run_id: state.run_id.clone(),
        title: state.title.clone(),
        description: state.description.clone(),
        status: Some(status.unwrap_or(ManifestStatus::Running)),
        source: MovieSourceKind::Frames,
        poster_path,
        video_path: encoded.video_path,
        duration_ms,
        chapters: state.chapters.clone(),
        size: Some(state.size),
    })?;

    // Drop session state; artifacts live in .astroshot/.
    let _ = fs::remove_dir_all(&session);

    Ok(MovieArtifact {
        video_path: published.video_dest,
        poster_path: published.poster_dest,
        duration_ms,
        chapters: state.chapters.clone(),
        source: MovieSourceKind::Frames,
        feature: state.feature.clone(),
        slug: state.slug.clone(),
        sequence: published.sequence,
        run_id: state.run_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_reports_missing_sessions_and_bad_versions() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let error = load_frame_session(&root_path, "nope", None).unwrap_err();
        assert_eq!(error.to_string(), "no movie session for feature nope");

        let started = start_frame_session(StartFrameSessionOptions {
            feature: "f".into(),
            slug: "s".into(),
            root: Some(root_path.clone()),
            ..StartFrameSessionOptions::default()
        })
        .unwrap();
        let error = load_frame_session(&root_path, "f", Some("zzz")).unwrap_err();
        assert_eq!(error.to_string(), "movie session not found: zzz");

        let path = state_path(&session_dir(&root_path, "f", &started.id));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.starts_with("{\n  \"version\": 1,\n  \"id\""));
        fs::write(&path, text.replace("\"version\": 1", "\"version\": 2")).unwrap();
        let error = load_frame_session(&root_path, "f", None).unwrap_err();
        assert_eq!(error.to_string(), "unsupported movie session version: 2");
    }

    #[test]
    fn push_rejects_missing_and_unsupported_frames() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let started = start_frame_session(StartFrameSessionOptions {
            feature: "f".into(),
            slug: "s".into(),
            root: Some(root_path.clone()),
            ..StartFrameSessionOptions::default()
        })
        .unwrap();
        let missing = root.path().join("none.png").to_string_lossy().into_owned();
        assert_eq!(
            push_frame_to_session(&started, &missing)
                .unwrap_err()
                .to_string(),
            format!("frame not found: {missing}")
        );
        let gif = root.path().join("a.gif");
        fs::write(&gif, b"x").unwrap();
        assert_eq!(
            push_frame_to_session(&started, &gif.to_string_lossy())
                .unwrap_err()
                .to_string(),
            "unsupported frame extension .gif"
        );
    }
}
