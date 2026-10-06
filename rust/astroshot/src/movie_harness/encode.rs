//! Encode a PNG sequence to WebM/MP4.
//!
//! Port of `packages/movie-harness/src/encode.ts`. Prefers ffmpeg when present
//! (identical argv); otherwise replays the frames in headless Chromium through
//! [`crate::browser::record_webm`] in place of Playwright's `recordVideo`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow, bail};

use super::paths::{ensure_dir, resolve_lexically};
use super::types::{EncodeFramesRequest, Size};
use crate::browser::{Browser, LaunchOptions, record_webm};

/// `Math.round`: halves round toward +Infinity.
pub(crate) fn js_round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `path.extname(..).toLowerCase()` (a leading dot alone is not an extension).
pub(crate) fn extname_lower(path: &str) -> String {
    let base = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    match base.rfind('.') {
        Some(index) if index > 0 => base[index..].to_lowercase(),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncodedVideo {
    pub video_path: String,
    pub duration_ms: f64,
}

fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// `request.durationMs ?? Math.max(1, Math.round(frames / fps * 1000))`.
pub fn duration_ms(request: &EncodeFramesRequest) -> f64 {
    request.duration_ms.unwrap_or_else(|| {
        js_round(request.frame_paths.len() as f64 / request.fps * 1000.0).max(1.0)
    })
}

pub async fn encode_frames(request: &EncodeFramesRequest) -> Result<EncodedVideo> {
    if request.frame_paths.is_empty() {
        bail!("encodeFrames requires at least one frame");
    }
    for frame in &request.frame_paths {
        if !Path::new(frame).exists() {
            bail!("frame not found: {frame}");
        }
    }

    let out_path = resolve_lexically(Path::new(&request.out_path))
        .to_string_lossy()
        .into_owned();
    if let Some(parent) = Path::new(&out_path).parent() {
        ensure_dir(&parent.to_string_lossy())?;
    }
    let ext = extname_lower(&out_path);
    let duration_ms = duration_ms(request);

    if has_ffmpeg() {
        encode_with_ffmpeg(request, &out_path)?;
        return Ok(EncodedVideo {
            video_path: out_path,
            duration_ms,
        });
    }

    if ext == ".mp4" {
        // Chromium emits WebM; without ffmpeg we cannot remux to MP4.
        let webm_path = format!("{}.webm", &out_path[..out_path.len() - 4]);
        encode_with_browser(request, &webm_path).await?;
        return Ok(EncodedVideo {
            video_path: webm_path,
            duration_ms,
        });
    }

    encode_with_browser(request, &out_path).await?;
    Ok(EncodedVideo {
        video_path: out_path,
        duration_ms,
    })
}

/// Concat-demuxer list: one `file`/`duration` pair per frame, then the last
/// frame again (the demuxer needs a trailing file entry).
pub fn concat_list(frame_paths: &[String], fps: f64) -> String {
    let frame_duration = 1.0 / fps;
    let quote = |frame: &str| format!("file '{}'", frame.replace('\'', "'\\''"));
    let mut lines: Vec<String> = Vec::new();
    for frame in frame_paths {
        lines.push(quote(frame));
        lines.push(format!("duration {frame_duration:.6}"));
    }
    if let Some(last) = frame_paths.last() {
        lines.push(quote(last));
    }
    format!("{}\n", lines.join("\n"))
}

/// The ffmpeg argv: H.264/MP4 for `.mp4` outputs, VP9/WebM for everything else.
pub fn ffmpeg_args(list_path: &str, out_path: &str, size: Size) -> Vec<String> {
    let scale = format!("scale={}:{}:flags=neighbor", size.width, size.height);
    let mut args: Vec<&str> = vec![
        "-y", "-f", "concat", "-safe", "0", "-i", list_path, "-vf", &scale,
    ];
    if extname_lower(out_path) == ".mp4" {
        args.extend([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ]);
    } else {
        args.extend(["-c:v", "libvpx-vp9", "-b:v", "2M", "-pix_fmt", "yuv420p"]);
    }
    args.push(out_path);
    args.into_iter().map(str::to_string).collect()
}

/// `fs.mkdtempSync(path.join(os.tmpdir(), prefix))`.
fn make_temp_dir(prefix: &str) -> std::io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("{prefix}{}-{nanos:x}-{seq}", std::process::id()));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// `stderr.slice(0, 800)` counted in UTF-16 units, like JS.
fn slice_utf16(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut units = 0;
    for c in text.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out
}

fn encode_with_ffmpeg(request: &EncodeFramesRequest, out_path: &str) -> Result<()> {
    let list_dir = make_temp_dir("astroshot-movie-ff-")?;
    let outcome = (|| -> Result<()> {
        let list_path = list_dir.join("frames.txt");
        fs::write(&list_path, concat_list(&request.frame_paths, request.fps))?;
        let args = ffmpeg_args(&list_path.to_string_lossy(), out_path, request.size);
        let result = Command::new("ffmpeg").args(&args).output()?;
        if !result.status.success() {
            let status = result
                .status
                .code()
                .map_or_else(|| "null".to_string(), |code| code.to_string());
            bail!(
                "ffmpeg failed ({status}): {}",
                slice_utf16(&String::from_utf8_lossy(&result.stderr), 800)
            );
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&list_dir);
    outcome
}

async fn encode_with_browser(request: &EncodeFramesRequest, out_path: &str) -> Result<()> {
    let frames = request
        .frame_paths
        .iter()
        .map(fs::read)
        .collect::<std::io::Result<Vec<_>>>()?;
    let size = crate::browser::Size {
        width: request.size.width,
        height: request.size.height,
    };
    let browser = Browser::launch(LaunchOptions::default()).await?;
    let recorded = record_webm(&browser, &frames, size, request.fps.round().max(1.0) as u32).await;
    let _ = browser.close().await;
    let webm = recorded.map_err(|error| anyhow!(error))?;
    fs::write(out_path, webm)?;
    Ok(())
}

/// Copy the last frame as the poster PNG.
pub fn poster_from_frames(frame_paths: &[String], out_path: &str) -> Result<String> {
    let Some(last) = frame_paths.last() else {
        bail!("posterFromFrames requires at least one frame");
    };
    if let Some(parent) = Path::new(out_path).parent() {
        ensure_dir(&parent.to_string_lossy())?;
    }
    fs::copy(last, out_path)?;
    Ok(out_path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    fn request(frames: Vec<String>, out: &str, fps: f64) -> EncodeFramesRequest {
        EncodeFramesRequest {
            frame_paths: frames,
            out_path: out.to_string(),
            size: size(64, 48),
            fps,
            duration_ms: None,
        }
    }

    fn write_frames(dir: &Path, count: u8) -> Vec<String> {
        (0..count)
            .map(|index| {
                let path = dir.join(format!("frame-{index}.png"));
                let file = fs::File::create(&path).unwrap();
                let mut encoder = png::Encoder::new(file, 16, 12);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                let pixels: Vec<u8> = (0..16 * 12)
                    .flat_map(|_| [index * 100, 255 - index * 100, 64])
                    .collect();
                writer.write_image_data(&pixels).unwrap();
                drop(writer);
                path.to_string_lossy().into_owned()
            })
            .collect()
    }

    #[test]
    fn ffmpeg_args_for_webm() {
        assert_eq!(
            ffmpeg_args("/t/frames.txt", "/o/a.webm", size(640, 360)),
            [
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                "/t/frames.txt",
                "-vf",
                "scale=640:360:flags=neighbor",
                "-c:v",
                "libvpx-vp9",
                "-b:v",
                "2M",
                "-pix_fmt",
                "yuv420p",
                "/o/a.webm"
            ]
        );
    }

    #[test]
    fn ffmpeg_args_for_mp4() {
        assert_eq!(
            ffmpeg_args("/t/frames.txt", "/o/a.MP4", size(640, 360)),
            [
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                "/t/frames.txt",
                "-vf",
                "scale=640:360:flags=neighbor",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "/o/a.MP4"
            ]
        );
    }

    #[test]
    fn ffmpeg_args_for_mov_use_the_webm_codec_path() {
        let args = ffmpeg_args("/t/l.txt", "/o/a.mov", size(2, 3));
        assert_eq!(args[9..12], ["-c:v", "libvpx-vp9", "-b:v"]);
        assert_eq!(args.last().unwrap(), "/o/a.mov");
        assert!(!args.contains(&"libx264".to_string()));
    }

    #[test]
    fn concat_list_has_durations_trailing_entry_and_quote_escaping() {
        let frames = vec!["/a/1.png".to_string(), "/a/it's.png".to_string()];
        assert_eq!(
            concat_list(&frames, 8.0),
            "file '/a/1.png'\nduration 0.125000\nfile '/a/it'\\''s.png'\nduration 0.125000\nfile '/a/it'\\''s.png'\n"
        );
        assert_eq!(
            concat_list(&frames[..1], 3.0),
            "file '/a/1.png'\nduration 0.333333\nfile '/a/1.png'\n"
        );
    }

    #[test]
    fn duration_defaults_to_frame_count_over_fps_and_honours_override() {
        let frames = vec![String::new(); 3];
        assert_eq!(duration_ms(&request(frames.clone(), "o.webm", 10.0)), 300.0);
        assert_eq!(duration_ms(&request(frames.clone(), "o.webm", 7.0)), 429.0);
        assert_eq!(duration_ms(&request(frames.clone(), "o.webm", 1e9)), 1.0);
        let mut overridden = request(frames, "o.webm", 10.0);
        overridden.duration_ms = Some(1234.5);
        assert_eq!(duration_ms(&overridden), 1234.5);
    }

    #[test]
    fn extname_matches_node() {
        assert_eq!(extname_lower("/a/b.MP4"), ".mp4");
        assert_eq!(extname_lower("/a/.webm"), "");
        assert_eq!(extname_lower("/a/b"), "");
        assert_eq!(extname_lower("/a.d/b"), "");
        assert_eq!(extname_lower("a."), ".");
    }

    #[tokio::test]
    async fn encode_frames_requires_at_least_one_frame() {
        let error = encode_frames(&request(vec![], "/tmp/x.webm", 10.0))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "encodeFrames requires at least one frame"
        );
    }

    #[tokio::test]
    async fn encode_frames_rejects_missing_frame() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.png").to_string_lossy().into_owned();
        let error = encode_frames(&request(vec![missing.clone()], "/tmp/x.webm", 10.0))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), format!("frame not found: {missing}"));
    }

    #[test]
    fn poster_from_frames_copies_last_frame() {
        let dir = tempfile::tempdir().unwrap();
        let frames = write_frames(dir.path(), 3);
        let out = dir
            .path()
            .join("deep/poster.png")
            .to_string_lossy()
            .into_owned();
        assert_eq!(poster_from_frames(&frames, &out).unwrap(), out);
        assert_eq!(fs::read(&out).unwrap(), fs::read(&frames[2]).unwrap());
        assert_eq!(
            poster_from_frames(&[], &out).unwrap_err().to_string(),
            "posterFromFrames requires at least one frame"
        );
    }

    #[tokio::test]
    async fn encodes_three_frames_to_a_real_video() {
        let dir = tempfile::tempdir().unwrap();
        let frames = write_frames(dir.path(), 3);
        let out = dir
            .path()
            .join("out/movie.webm")
            .to_string_lossy()
            .into_owned();
        let req = request(frames, &out, 10.0);
        let outcome = if has_ffmpeg() {
            encode_frames(&req).await.unwrap()
        } else if crate::browser::find_chrome().is_ok() {
            eprintln!("skip ffmpeg path: ffmpeg not installed; using the browser fallback");
            encode_frames(&req).await.unwrap()
        } else {
            eprintln!("skip: neither ffmpeg nor Chrome is installed");
            return;
        };
        assert_eq!(outcome.video_path, out);
        assert_eq!(outcome.duration_ms, 300.0);
        let bytes = fs::read(&out).unwrap();
        // EBML header magic: both ffmpeg (VP9) and the browser recorder emit WebM.
        assert_eq!(bytes[..4], [0x1a, 0x45, 0xdf, 0xa3]);
    }

    #[tokio::test]
    async fn browser_fallback_encodes_three_frames_to_webm() {
        if crate::browser::find_chrome().is_err() {
            eprintln!("skip: no Chrome/Chromium installed for the browser fallback");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let frames = write_frames(dir.path(), 3);
        let out = dir
            .path()
            .join("fallback.webm")
            .to_string_lossy()
            .into_owned();
        encode_with_browser(&request(frames, &out, 10.0), &out)
            .await
            .unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(bytes[..4], [0x1a, 0x45, 0xdf, 0xa3]);
    }

    #[tokio::test]
    async fn ffmpeg_encodes_three_frames_to_webm_and_mp4() {
        if !has_ffmpeg() {
            eprintln!("skip: ffmpeg not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let frames = write_frames(dir.path(), 3);
        for (name, magic_at, magic) in [
            ("a.webm", 0, &[0x1a, 0x45, 0xdf, 0xa3][..]),
            ("a.mp4", 4, b"ftyp"),
        ] {
            let out = dir.path().join(name).to_string_lossy().into_owned();
            let outcome = encode_frames(&request(frames.clone(), &out, 10.0))
                .await
                .unwrap();
            assert_eq!(outcome.video_path, out);
            let bytes = fs::read(&out).unwrap();
            assert_eq!(&bytes[magic_at..magic_at + magic.len()], magic, "{name}");
        }
    }
}
