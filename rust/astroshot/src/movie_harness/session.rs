//! A movie capture session: chapters, timing, frame collection, then encode
//! and publish.
//!
//! Port of `packages/movie-harness/src/session.ts`. `Date.now()` becomes
//! [`now_ms`]; error texts are the TS messages.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use super::encode::{encode_frames, extname_lower, poster_from_frames};
use super::paths::{assert_kebab_case, assert_slug, default_run_id, ensure_dir, resolve_root};
use super::sink::{finalize_manifest, sink_movie};
use super::types::{
    EncodeFramesRequest, ManifestStatus, MovieArtifact, MovieChapter, MovieFormat,
    MovieSessionOptions, MovieSourceKind, SinkMovieRequest, Size,
};

/// `Date.now()`: whole milliseconds since the Unix epoch.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// `n` random bytes from the std hasher's per-instance random keys.
pub(crate) fn random_bytes(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n + 8);
    while out.len() < n {
        let value = RandomState::new().build_hasher().finish();
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.truncate(n);
    out
}

/// `/\.(png|jpe?g)$/i` on a file name.
pub(crate) fn is_frame_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")
}

/// Frame file extensions a session accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameExtension {
    #[default]
    Png,
    Jpg,
    Jpeg,
}

impl FrameExtension {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
            Self::Jpeg => "jpeg",
        }
    }
}

fn path_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

/// `fs.mkdtempSync(path.join(os.tmpdir(), prefix))`.
fn mkdtemp(prefix: &str) -> Result<PathBuf> {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let base = std::env::temp_dir();
    for _ in 0..100 {
        let suffix: String = random_bytes(6)
            .iter()
            .map(|byte| CHARS[*byte as usize % CHARS.len()] as char)
            .collect();
        let dir = base.join(format!("{prefix}{suffix}"));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!(
        "could not create a temporary directory under {}",
        base.display()
    )
}

/// Optional overrides for [`MovieSession::stop`].
#[derive(Debug, Clone, Default)]
pub struct StopOptions {
    pub status: Option<ManifestStatus>,
    /// Skip encode and only publish when frames already form the movie via another path.
    pub video_path: Option<String>,
    pub poster_path: Option<String>,
    pub duration_ms: Option<f64>,
}

#[derive(Debug)]
pub struct MovieSession {
    pub feature: String,
    pub slug: String,
    pub root: String,
    pub run_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub size: Size,
    pub fps: f64,
    pub format: MovieFormat,
    pub source: MovieSourceKind,

    work_dir: PathBuf,
    frame_dir: PathBuf,
    started_at_ms: u64,
    frame_count: u64,
    chapters: Vec<MovieChapter>,
    stopped: bool,
    status: ManifestStatus,
}

impl MovieSession {
    fn new(options: MovieSessionOptions, work_dir: PathBuf) -> Result<Self> {
        let frame_dir = work_dir.join("frames");
        let session = Self {
            root: resolve_root(options.root.as_deref()),
            run_id: options
                .run_id
                .unwrap_or_else(|| default_run_id(&options.feature)),
            feature: options.feature,
            slug: options.slug,
            title: options.title,
            description: options.description,
            size: options.size.unwrap_or(Size {
                width: 1280,
                height: 720,
            }),
            fps: options.fps.unwrap_or(15.0),
            format: options.format.unwrap_or(MovieFormat::Webm),
            source: options.source,
            status: options.status.unwrap_or(ManifestStatus::Running),
            work_dir,
            frame_dir,
            started_at_ms: now_ms(),
            frame_count: 0,
            chapters: Vec::new(),
            stopped: false,
        };
        ensure_dir(&path_string(session.frame_dir.clone()))?;
        Ok(session)
    }

    pub fn create(options: MovieSessionOptions) -> Result<Self> {
        assert_kebab_case(&options.feature, "feature")?;
        assert_slug(&options.slug)?;
        if let Some(fps) = options.fps
            && !(fps > 0.0 && fps <= 60.0)
        {
            bail!("fps must be in (0, 60]");
        }
        let work_dir = mkdtemp("astroshot-movie-")?;
        Self::new(options, work_dir)
    }

    /// Elapsed ms since session start (for chapter timestamps).
    pub fn elapsed_ms(&self) -> f64 {
        now_ms().saturating_sub(self.started_at_ms) as f64
    }

    pub fn chapters(&self) -> &[MovieChapter] {
        &self.chapters
    }

    pub fn mark(&mut self, slug: &str, note: Option<&str>) -> Result<()> {
        self.assert_open()?;
        assert_slug(slug)?;
        let t_ms = self.elapsed_ms();
        self.chapters.push(MovieChapter {
            slug: slug.to_string(),
            t_ms,
            note: note.map(str::to_string),
        });
        Ok(())
    }

    /// Push a PNG (or JPEG) frame into the session. Bytes are written to disk
    /// immediately so a crash still leaves a recoverable sequence.
    pub fn push_frame(&mut self, image: &[u8], extension: FrameExtension) -> Result<String> {
        self.assert_open()?;
        if image.len() < 8 {
            bail!("pushFrame: empty image buffer");
        }
        let index = format!("{:06}", self.frame_count);
        let file = self
            .frame_dir
            .join(format!("{index}.{}", extension.as_str()));
        fs::write(&file, image)?;
        self.frame_count += 1;
        Ok(path_string(file))
    }

    pub fn push_frame_file(&mut self, image_path: &str) -> Result<String> {
        self.assert_open()?;
        let bytes = fs::read(image_path)?;
        let ext = extname_lower(image_path).replacen('.', "", 1);
        let extension = match ext.as_str() {
            "png" => FrameExtension::Png,
            "jpg" => FrameExtension::Jpg,
            "jpeg" => FrameExtension::Jpeg,
            _ => bail!("pushFrameFile: unsupported extension .{ext}"),
        };
        self.push_frame(&bytes, extension)
    }

    pub fn list_frames(&self) -> Result<Vec<String>> {
        if !self.frame_dir.exists() {
            return Ok(Vec::new());
        }
        list_frame_files(&self.frame_dir)
    }

    /// Encode collected frames, write poster + video into .astroshot/, update
    /// manifest. Cleans the temp work directory afterward.
    pub async fn stop(&mut self, options: StopOptions) -> Result<MovieArtifact> {
        self.assert_open()?;
        self.stopped = true;
        let result = self.stop_inner(options).await;
        let _ = fs::remove_dir_all(&self.work_dir); // `finally { rmSync(force) }`
        result
    }

    async fn stop_inner(&self, options: StopOptions) -> Result<MovieArtifact> {
        let status = options.status.unwrap_or(self.status);
        let mut video_path = options.video_path.filter(|path| !path.is_empty());
        let mut poster_path = options.poster_path.filter(|path| !path.is_empty());
        let mut duration_ms = options.duration_ms;

        let frames = self.list_frames()?;

        if video_path.is_none() {
            if frames.is_empty() {
                bail!("MovieSession.stop: no frames and no videoPath");
            }
            let out_ext = if self.format == MovieFormat::Mp4 {
                ".mp4"
            } else {
                ".webm"
            };
            let encoded = encode_frames(&EncodeFramesRequest {
                frame_paths: frames.clone(),
                out_path: path_string(self.work_dir.join(format!("movie{out_ext}"))),
                size: self.size,
                fps: self.fps,
                duration_ms: None,
            })
            .await?;
            video_path = Some(encoded.video_path);
            duration_ms = Some(encoded.duration_ms);
        }

        if poster_path.is_none() {
            if frames.is_empty() {
                bail!("MovieSession.stop: no frames for poster");
            }
            poster_path = Some(poster_from_frames(
                &frames,
                &path_string(self.work_dir.join("poster.png")),
            )?);
        }

        let duration_ms = duration_ms.unwrap_or_else(|| self.elapsed_ms());

        let published = sink_movie(&SinkMovieRequest {
            root: self.root.clone(),
            feature: self.feature.clone(),
            slug: self.slug.clone(),
            run_id: self.run_id.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            status: Some(status),
            source: self.source,
            poster_path: poster_path.unwrap_or_default(),
            video_path: video_path.unwrap_or_default(),
            duration_ms,
            chapters: self.chapters.clone(),
            size: Some(self.size),
        })?;

        Ok(MovieArtifact {
            video_path: published.video_dest,
            poster_path: published.poster_dest,
            duration_ms,
            chapters: self.chapters.clone(),
            source: self.source,
            feature: self.feature.clone(),
            slug: self.slug.clone(),
            sequence: published.sequence,
            run_id: self.run_id.clone(),
        })
    }

    pub async fn finalize(&self, status: ManifestStatus) -> Result<()> {
        finalize_manifest(&self.root, &self.feature, &self.run_id, status)
            .context("finalize movie manifest")
    }

    fn assert_open(&self) -> Result<()> {
        if self.stopped {
            bail!("MovieSession is already stopped");
        }
        Ok(())
    }
}

/// Frame files (`.png`/`.jpg`/`.jpeg`) in `dir`, name-sorted, as full paths.
pub(crate) fn list_frame_files(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    names.retain(|name| is_frame_file_name(name));
    names.sort();
    Ok(names
        .into_iter()
        .map(|name| path_string(dir.join(name)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movie_harness::png::encode_solid_png;
    use crate::movie_harness::sources::frames_store::{
        StartFrameSessionOptions, load_frame_session, mark_frame_session, push_frame_to_session,
        start_frame_session, stop_frame_session,
    };
    use serde_json::Value;

    fn temp_root() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("movie-harness-test-")
            .tempdir()
            .unwrap()
    }

    #[tokio::test]
    async fn movie_session_frames_path_encodes_frames_writes_poster_video_and_updates_manifest() {
        let root = temp_root();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut session = MovieSession::create(MovieSessionOptions {
            feature: "demo-journey".into(),
            slug: "flow".into(),
            root: Some(root_path.clone()),
            run_id: None,
            title: Some("Flow".into()),
            description: Some("Synthetic color frames".into()),
            size: Some(Size {
                width: 160,
                height: 90,
            }),
            fps: Some(8.0),
            format: None,
            status: None,
            source: MovieSourceKind::Frames,
        })
        .unwrap();

        let colors: [[u8; 3]; 4] = [
            [124, 92, 255],
            [80, 220, 160],
            [240, 114, 122],
            [90, 200, 250],
        ];
        for rgb in colors {
            session
                .push_frame(&encode_solid_png(160, 90, rgb), FrameExtension::Png)
                .unwrap();
        }
        session.mark("mid", Some("halfway")).unwrap();

        let artifact = session
            .stop(StopOptions {
                status: Some(ManifestStatus::Pass),
                ..StopOptions::default()
            })
            .await
            .unwrap();

        assert!(Path::new(&artifact.poster_path).exists());
        assert!(Path::new(&artifact.video_path).exists());
        assert_eq!(extname_lower(&artifact.video_path), ".webm");
        assert_eq!(artifact.sequence, "0001");
        assert_eq!(artifact.chapters.len(), 1);

        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(
                root.path()
                    .join(".astroshot")
                    .join("demo-journey")
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["feature"], "demo-journey");
        assert_eq!(manifest["status"], "pass");
        assert_eq!(manifest["shots"].as_array().unwrap().len(), 1);
        assert_eq!(manifest["shots"][0]["kind"], "movie");
        assert_eq!(manifest["shots"][0]["video"], "0001-flow.webm");
        assert_eq!(manifest["shots"][0]["file"], "0001-flow.png");
        assert_eq!(manifest["shots"][0]["chapters"][0]["slug"], "mid");
    }

    #[tokio::test]
    async fn persisted_frames_cli_store_survives_start_push_mark_stop_across_loads() {
        let root = temp_root();
        let root_path = root.path().to_string_lossy().into_owned();
        let started = start_frame_session(StartFrameSessionOptions {
            feature: "cli-frames".into(),
            slug: "walk".into(),
            root: Some(root_path.clone()),
            size: Some(Size {
                width: 120,
                height: 80,
            }),
            fps: Some(6.0),
            ..StartFrameSessionOptions::default()
        })
        .unwrap();
        assert_eq!(started.frame_count, 0);

        let frame = root.path().join("a.png");
        fs::write(&frame, encode_solid_png(120, 80, [124, 92, 255])).unwrap();
        let frame = frame.to_string_lossy().into_owned();
        push_frame_to_session(&started, &frame).unwrap();
        push_frame_to_session(
            &load_frame_session(&root_path, "cli-frames", Some(&started.id)).unwrap(),
            &frame,
        )
        .unwrap();
        mark_frame_session(
            &load_frame_session(&root_path, "cli-frames", Some(&started.id)).unwrap(),
            "step-a",
            None,
        )
        .unwrap();

        let artifact = stop_frame_session(
            &load_frame_session(&root_path, "cli-frames", Some(&started.id)).unwrap(),
            Some(ManifestStatus::Pass),
        )
        .await
        .unwrap();
        assert_eq!(artifact.sequence, "0001");
        assert!(Path::new(&artifact.video_path).exists());
        assert_eq!(artifact.chapters[0].slug, "step-a");
    }
}
