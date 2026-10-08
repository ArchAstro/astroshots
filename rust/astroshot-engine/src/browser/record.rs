//! The video half of Playwright's `recordVideo`, which the TS browser movie
//! source relies on: screencast frames are laid onto a fixed 25 fps timeline
//! and piped to ffmpeg as WebM. The timeline is a port of playwright-core's
//! `FfmpegVideoRecorder` (`server/videoRecorder.ts`), so a journey recorded
//! here has the frame count and duration Playwright would have written. The
//! encode is not Playwright's: it wrote VP8 at 1 Mbit/s in realtime mode,
//! which smears text, and this crate's ffmpeg turned its full-range JPEG
//! input into limited range without converting, which made recordings dark.
//! Frames are encoded as constant-quality VP9 with explicit colour (see
//! `video_encode`). The timeline rules:
//!
//! - frame `i` sits at slot `floor((timestamp_i - timestamp_0) * 25)` and is
//!   repeated until the next frame's slot;
//! - on stop the last frame is held for `max(time since it arrived, 1 s)`, so
//!   a page that never repaints still yields about one second of video.

use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{BrowserError, Result, ScreencastFrame, Size};
use crate::video_encode::{COLOR_FILTER, COLOR_TAGS, VP9_QUALITY, even};

/// Playwright records at a fixed rate, whatever the session fps is.
pub const RECORDER_FPS: u32 = 25;

const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];

/// What [`write_recorder_webm`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedVideo {
    /// Frames piped to the encoder; the video lasts `frame_count / 25` s.
    pub frame_count: usize,
}

/// How many times each frame is written, given the frames' timestamps in
/// seconds and the time between the last frame's arrival and the stop
/// (`FfmpegVideoRecorder._writeFrame` / `_stop`). A frame that shares a slot
/// with its successor gets 0 and is dropped, as in Playwright.
pub fn recorder_repeats(timestamps: &[f64], idle_secs: f64) -> Vec<usize> {
    let Some((&first, _)) = timestamps.split_first() else {
        return Vec::new();
    };
    let fps = f64::from(RECORDER_FPS);
    let slot = |timestamp: f64| ((timestamp - first) * fps).floor() as i64;
    let last = timestamps[timestamps.len() - 1];
    // `Math.max((monotonicTime() - lastWriteNodeTime) / 1e3, 1)`
    let add_time = idle_secs.max(1.0);
    let end = slot(last + add_time);
    timestamps
        .iter()
        .enumerate()
        .map(|(index, &timestamp)| {
            let next = timestamps.get(index + 1).map_or(end, |&next| slot(next));
            (next - slot(timestamp)).max(0) as usize
        })
        .collect()
}

/// The recorder argv: Playwright's input side and frame rate, a leading
/// `crop` to at most the output size, and this crate's colour and quality
/// settings. Playwright only ever receives frames no larger than the video
/// (`pad` rejects larger ones); here a viewport below the window's minimum
/// size arrives inside a larger frame, top-left aligned. `input_codec` is
/// `mjpeg` for screencast JPEGs or `png`.
pub fn recorder_ffmpeg_args(input_codec: &str, size: Size, out_path: &str) -> Vec<String> {
    let (w, h) = (even(size.width), even(size.height));
    let fps = RECORDER_FPS;
    let input = format!(
        "-loglevel error -f image2pipe -avioflags direct -fpsprobesize 0 -probesize 32 -analyzeduration 0 -c:v {input_codec} -i pipe:0 -y -an -r {fps} -vf crop=min(iw\\,{w}):min(ih\\,{h}):0:0,pad={w}:{h}:0:0:gray,crop={w}:{h}:0:0,{COLOR_FILTER}"
    );
    let mut args: Vec<String> = input.split(' ').map(str::to_string).collect();
    args.extend(
        VP9_QUALITY
            .iter()
            .chain(&COLOR_TAGS)
            .map(|arg| arg.to_string()),
    );
    args.push(out_path.to_string());
    args
}

/// Encode `frames` to a VP9 WebM at `out_path` on Playwright's timeline (see
/// the module docs). `idle_secs` is the time between the last frame's arrival
/// and the stop. Frames must all be JPEG or all PNG. Needs `ffmpeg` on PATH.
pub async fn write_recorder_webm(
    frames: &[ScreencastFrame],
    idle_secs: f64,
    size: Size,
    out_path: &str,
) -> Result<RecordedVideo> {
    let Some(first) = frames.first() else {
        return Err(failed("no frames to record"));
    };
    let input_codec = if first.data.starts_with(&PNG_MAGIC) {
        "png"
    } else {
        "mjpeg"
    };
    let timestamps: Vec<f64> = frames.iter().map(|frame| frame.timestamp).collect();
    let repeats = recorder_repeats(&timestamps, idle_secs);

    let mut child = Command::new("ffmpeg")
        .args(recorder_ffmpeg_args(input_codec, size, out_path))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| failed(&format!("could not start ffmpeg: {error}")))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut frame_count = 0;
    // A write error means ffmpeg exited early; its stderr says why.
    'frames: for (frame, &count) in frames.iter().zip(&repeats) {
        for _ in 0..count {
            if stdin.write_all(&frame.data).await.is_err() {
                break 'frames;
            }
            frame_count += 1;
        }
    }
    let _ = stdin.shutdown().await;
    drop(stdin);
    let output = child
        .wait_with_output()
        .await
        .map_err(|error| failed(&format!("ffmpeg did not finish: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail: String = stderr.chars().take(800).collect();
        return Err(failed(&format!(
            "ffmpeg failed ({}): {detail}",
            output.status
        )));
    }
    Ok(RecordedVideo { frame_count })
}

fn failed(message: &str) -> BrowserError {
    BrowserError::Screencast(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_frame_is_held_for_at_least_a_second() {
        assert_eq!(recorder_repeats(&[10.0], 0.2), [25]);
        assert_eq!(recorder_repeats(&[10.0], 1.5), [37]);
        assert!(recorder_repeats(&[], 3.0).is_empty());
    }

    /// Epoch-sized timestamps lose the last bits of `t + 1 - t`, so Playwright
    /// writes 24 frames (0.96 s) for a still page. Keep that arithmetic.
    #[test]
    fn epoch_timestamps_keep_playwrights_float_rounding() {
        let first: f64 = 1_791_331_253.224_117;
        let repeats = recorder_repeats(&[first], 0.1);
        let expected = ((first + 1.0 - first) * 25.0).floor() as usize;
        assert_eq!(repeats, [expected]);
        assert!(expected == 24 || expected == 25, "{expected}");
    }

    #[test]
    fn frames_fill_their_slots_until_the_next_frame() {
        // Slots 0, 2, 2 (same slot as its predecessor), 10; then a 1 s tail.
        let repeats = recorder_repeats(&[0.0, 0.08, 0.09, 0.4], 0.0);
        assert_eq!(repeats, [2, 0, 8, 25]);
        assert_eq!(repeats.iter().sum::<usize>(), 35); // 0.4 s + 1 s at 25 fps
    }

    #[test]
    fn a_timestamp_that_goes_backwards_writes_nothing() {
        assert_eq!(recorder_repeats(&[1.0, 0.5, 1.25], 0.0), [0, 19, 25]);
    }

    #[test]
    fn ffmpeg_args_keep_playwrights_input_and_rate_with_explicit_colour() {
        let size = Size {
            width: 320,
            height: 201,
        };
        let args = recorder_ffmpeg_args("mjpeg", size, "/o/a.webm");
        assert_eq!(
            args.join(" "),
            "-loglevel error -f image2pipe -avioflags direct -fpsprobesize 0 -probesize 32 -analyzeduration 0 -c:v mjpeg -i pipe:0 -y -an -r 25 -vf crop=min(iw\\,320):min(ih\\,202):0:0,pad=320:202:0:0:gray,crop=320:202:0:0,scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,setparams=color_primaries=bt709:color_trc=iec61966-2-1 -c:v libvpx-vp9 -crf 15 -b:v 0 -row-mt 1 -color_range tv -colorspace bt709 -color_primaries bt709 -color_trc iec61966-2-1 /o/a.webm"
        );
    }
}
