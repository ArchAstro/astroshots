//! Port of `packages/astroshot-review/src/video/ffmpeg.ts`.
//!
//! Movie playback through ffmpeg: decode the file at a modest frame rate,
//! scaled to the stage, as a stream of PNG frames the terminal can draw with
//! the graphics protocol. Seeking restarts the decoder at the new offset.
//!
//! Divergences: `FramePlayer::start` needs a tokio runtime; callbacks are
//! `Send + Sync` closures; spawn errors carry Rust's io error text rather than
//! Node's `spawn <bin> ENOENT`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

use super::super::images::png::{ImageSize, fit_inside, read_png_size};

const EXTRA_BIN_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];
const PNG_END: [u8; 8] = [0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInfo {
    pub ffmpeg: Option<String>,
    pub ffprobe: Option<String>,
    pub version: Option<String>,
}

static CACHED_INFO: Mutex<Option<FfmpegInfo>> = Mutex::new(None);

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn find_binary(name: &str, path_env: &str) -> Option<String> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    let dirs = path_env
        .split(separator)
        .chain(EXTRA_BIN_DIRS)
        .filter(|dir| !dir.is_empty());
    for dir in dirs {
        let candidate = Path::new(dir).join(name);
        if is_executable(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Extract `\S+` after `ffmpeg version `.
fn parse_version(stdout: &str) -> Option<String> {
    let marker = "ffmpeg version ";
    let start = stdout.find(marker)? + marker.len();
    let rest = &stdout[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_string())
}

/// Detect ffmpeg/ffprobe from the process environment (cached).
pub fn detect_ffmpeg() -> FfmpegInfo {
    detect_ffmpeg_with(|name| std::env::var(name).ok())
}

/// Like [`detect_ffmpeg`], reading variables through `env`.
pub fn detect_ffmpeg_with(env: impl Fn(&str) -> Option<String>) -> FfmpegInfo {
    let mut cache = CACHED_INFO.lock().unwrap();
    if let Some(info) = cache.as_ref() {
        return info.clone();
    }
    let path_env = env("PATH").unwrap_or_default();
    let ffmpeg = env("ASTROSHOT_REVIEW_FFMPEG").or_else(|| find_binary("ffmpeg", &path_env));
    let ffprobe = find_binary("ffprobe", &path_env);
    let mut version = None;
    if let Some(ffmpeg) = &ffmpeg
        && let Ok(output) = std::process::Command::new(ffmpeg).arg("-version").output()
    {
        version = parse_version(&String::from_utf8_lossy(&output.stdout));
    }
    let info = FfmpegInfo {
        ffmpeg,
        ffprobe,
        version,
    };
    *cache = Some(info.clone());
    info
}

pub fn reset_ffmpeg_cache() {
    *CACHED_INFO.lock().unwrap() = None;
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: Option<u64>,
}

fn parse_probe_output(output: &str) -> Option<VideoInfo> {
    let parsed: serde_json::Value = serde_json::from_str(output).ok()?;
    let stream = parsed.get("streams")?.as_array()?.first()?;
    let dimension = |key: &str| {
        stream
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .filter(|v| *v > 0)
            .map(|v| v as u32)
    };
    let width = dimension("width")?;
    let height = dimension("height")?;
    let duration = match parsed.get("format").and_then(|f| f.get("duration")) {
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().ok(),
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        _ => None,
    };
    Some(VideoInfo {
        width,
        height,
        duration_ms: duration
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(|d| (d * 1000.0 + 0.5).floor() as u64),
    })
}

/// Probe size and duration with ffprobe; `None` when unavailable or unparsable.
pub async fn probe_video(video_path: &str, info: Option<&FfmpegInfo>) -> Option<VideoInfo> {
    let detected;
    let info = match info {
        Some(info) => info,
        None => {
            detected = detect_ffmpeg();
            &detected
        }
    };
    let ffprobe = info.ffprobe.as_ref()?;
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height:format=duration",
            "-of",
            "json",
            video_path,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    parse_probe_output(&String::from_utf8_lossy(&output.stdout))
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Presentation time in milliseconds.
    pub t_ms: f64,
    pub index: u64,
}

pub struct FramePlayerOptions {
    pub video_path: String,
    /// Pixel box the frames must fit inside.
    pub bounds: ImageSize,
    pub source_size: ImageSize,
    pub fps: Option<f64>,
    pub start_ms: Option<f64>,
    pub on_frame: Box<dyn Fn(VideoFrame) + Send + Sync>,
    pub on_end: Box<dyn Fn() + Send + Sync>,
    pub on_error: Box<dyn Fn(anyhow::Error) + Send + Sync>,
    pub ffmpeg_path: Option<PathBuf>,
}

/// Splits a concatenated PNG byte stream into whole files.
pub fn split_png_stream(buffer: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut frames = Vec::new();
    let mut cursor = 0;
    while cursor < buffer.len() {
        let Some(offset) = buffer[cursor..]
            .windows(PNG_END.len())
            .position(|w| w == PNG_END)
        else {
            break;
        };
        let end = cursor + offset + PNG_END.len();
        frames.push(buffer[cursor..end].to_vec());
        cursor = end;
    }
    (frames, buffer[cursor..].to_vec())
}

/// Format a JS number the way a template literal would (`12`, `7.5`).
fn js_number_string(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

struct Callbacks {
    on_frame: Box<dyn Fn(VideoFrame) + Send + Sync>,
    on_end: Box<dyn Fn() + Send + Sync>,
    on_error: Box<dyn Fn(anyhow::Error) + Send + Sync>,
}

pub struct FramePlayer {
    video_path: String,
    ffmpeg_path: Option<PathBuf>,
    fps: f64,
    start_ms: f64,
    pub frame_size: ImageSize,
    callbacks: Arc<Callbacks>,
    /// Count of delivered frames.
    index: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    kill: Arc<Notify>,
}

impl FramePlayer {
    pub fn new(options: FramePlayerOptions) -> Self {
        let fitted = fit_inside(options.source_size, options.bounds);
        // Even dimensions keep every encoder happy.
        let frame_size = ImageSize {
            width: (fitted.width - fitted.width % 2).max(2),
            height: (fitted.height - fitted.height % 2).max(2),
        };
        Self {
            video_path: options.video_path,
            ffmpeg_path: options.ffmpeg_path,
            fps: options.fps.unwrap_or(12.0),
            start_ms: options.start_ms.unwrap_or(0.0),
            frame_size,
            callbacks: Arc::new(Callbacks {
                on_frame: options.on_frame,
                on_end: options.on_end,
                on_error: options.on_error,
            }),
            index: Arc::new(AtomicU64::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
            kill: Arc::new(Notify::new()),
        }
    }

    /// The exact ffmpeg argv (without the program name).
    pub fn ffmpeg_args(&self) -> Vec<String> {
        vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-nostdin".into(),
            "-re".into(),
            "-ss".into(),
            format!("{:.3}", self.start_ms / 1000.0),
            "-i".into(),
            self.video_path.clone(),
            "-an".into(),
            "-vf".into(),
            format!(
                "fps={},scale={}:{}:flags=fast_bilinear",
                js_number_string(self.fps),
                self.frame_size.width,
                self.frame_size.height
            ),
            "-f".into(),
            "image2pipe".into(),
            "-vcodec".into(),
            "png".into(),
            "-compression_level".into(),
            "3".into(),
            "-".into(),
        ]
    }

    /// Spawn ffmpeg and stream frames to the callbacks. Needs a tokio runtime.
    pub fn start(&self) {
        let ffmpeg = match &self.ffmpeg_path {
            Some(path) => Some(path.to_string_lossy().into_owned()),
            None => detect_ffmpeg().ffmpeg,
        };
        let Some(ffmpeg) = ffmpeg else {
            (self.callbacks.on_error)(anyhow::anyhow!("ffmpeg is not installed"));
            return;
        };
        let spawned = Command::new(&ffmpeg)
            .args(self.ffmpeg_args())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                if !self.stopped.swap(true, Ordering::SeqCst) {
                    (self.callbacks.on_error)(anyhow::Error::new(error));
                }
                return;
            }
        };
        let mut stdout = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let stderr_task = tokio::spawn(async move {
            let mut text = Vec::new();
            let _ = BufReader::new(stderr_pipe).read_to_end(&mut text).await;
            String::from_utf8_lossy(&text).into_owned()
        });

        let callbacks = Arc::clone(&self.callbacks);
        let stopped = Arc::clone(&self.stopped);
        let index = Arc::clone(&self.index);
        let kill = Arc::clone(&self.kill);
        let fps = self.fps;
        let start_ms = self.start_ms;
        tokio::spawn(async move {
            let mut pending: Vec<u8> = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                tokio::select! {
                    () = kill.notified() => {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                        return;
                    }
                    read = stdout.read(&mut chunk) => {
                        let Ok(n) = read else { break };
                        if n == 0 {
                            break;
                        }
                        if stopped.load(Ordering::SeqCst) {
                            continue;
                        }
                        pending.extend_from_slice(&chunk[..n]);
                        let (frames, rest) = split_png_stream(&pending);
                        pending = rest;
                        for png in frames {
                            let Some(size) = read_png_size(&png) else { continue };
                            let i = index.fetch_add(1, Ordering::SeqCst);
                            let t_ms = start_ms + ((i as f64 * 1000.0) / fps + 0.5).floor();
                            (callbacks.on_frame)(VideoFrame {
                                png,
                                width: size.width,
                                height: size.height,
                                t_ms,
                                index: i,
                            });
                        }
                    }
                }
            }
            let status = child.wait().await;
            let stderr = stderr_task.await.unwrap_or_default();
            if stopped.swap(true, Ordering::SeqCst) {
                return;
            }
            match status {
                Ok(status) => match status.code() {
                    Some(code) if code != 0 => {
                        let detail: String = stderr.trim().chars().take(300).collect();
                        (callbacks.on_error)(anyhow::anyhow!(
                            "ffmpeg exited with {code}: {detail}"
                        ));
                    }
                    _ => (callbacks.on_end)(),
                },
                Err(error) => (callbacks.on_error)(anyhow::Error::new(error)),
            }
        });
    }

    /// Time of the last delivered frame, in milliseconds.
    pub fn position_ms(&self) -> f64 {
        let delivered = self.index.load(Ordering::SeqCst);
        let last = delivered.saturating_sub(1) as f64;
        self.start_ms + ((last * 1000.0) / self.fps + 0.5).floor()
    }

    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        // notify_one stores a permit, so a kill before the task polls is kept.
        self.kill.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_whole_frames_and_keeps_the_remainder() {
        let frame_a = [b"AAAA".as_slice(), &PNG_END].concat();
        let frame_b = [b"BB".as_slice(), &PNG_END].concat();
        let partial = b"CC";
        let input = [frame_a.as_slice(), &frame_b, partial].concat();
        let (frames, rest) = split_png_stream(&input);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], frame_a);
        assert_eq!(frames[1], frame_b);
        assert_eq!(String::from_utf8(rest).unwrap(), "CC");
    }
}
