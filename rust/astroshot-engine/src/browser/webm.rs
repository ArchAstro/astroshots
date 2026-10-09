//! Chromium-recorded WebM, the replacement for `encode.ts`'s Playwright
//! `recordVideo` fallback (used only when ffmpeg is missing).
//!
//! TS replays each frame as an `<img>` in a page that Playwright records in
//! real time, holding each for `1000 / fps` ms. Here the replay runs in-page
//! instead: frames are drawn onto a canvas, `canvas.captureStream(0)` plus `requestFrame()` per frame feeds a
//! `MediaRecorder` (VP8 WebM), each frame is held for `1000 / fps` ms of wall
//! clock, and the recorded blob comes back as bytes. Differences from
//! Playwright's recorder: the WebM has no duration/seek cues (players play it
//! fine; `ffprobe` may report no duration), and bitrate is fixed at 8 Mbps
//! rather than Playwright's ffmpeg defaults.

use base64::Engine as _;

use super::{Browser, BrowserError, PageOptions, Result, Size, js_str};

/// Upper bound on waiting for the encoder's first output after the last frame
/// was fed. It is a failure bound, not a delay: the wait ends as soon as the
/// recorder emits data.
const ENCODER_WARMUP_LIMIT_MS: u64 = 15_000;

const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];

/// Record `frames` (PNG or JPEG bytes, sniffed per frame) as a WebM, each
/// shown for `1000 / fps` ms, stretched to `size` with nearest-neighbour
/// scaling (`object-fit: fill; image-rendering: pixelated`). The last frame is
/// held an extra `max(frame_ms, 100)` ms so the encoder emits it.
pub async fn record_webm<F: AsRef<[u8]>>(
    browser: &Browser,
    frames: &[F],
    size: Size,
    fps: u32,
) -> Result<Vec<u8>> {
    if frames.is_empty() {
        return Err(BrowserError::Script(
            "record_webm requires at least one frame".into(),
        ));
    }
    let frame_ms = (1000.0 / f64::from(fps.max(1))).round().max(1.0) as u64;
    let page = browser
        .new_page(PageOptions::new(size.width, size.height))
        .await?;
    let outcome = replay(&page, frames, size, frame_ms, frame_ms.max(100)).await;
    let _ = page.close().await;
    outcome
}

async fn replay<F: AsRef<[u8]>>(
    page: &super::Page,
    frames: &[F],
    size: Size,
    frame_ms: u64,
    tail_hold_ms: u64,
) -> Result<Vec<u8>> {
    page.set_content(
        "<!doctype html><html><body style=\"margin:0;background:#000\"></body></html>",
    )
    .await?;
    let setup = format!(
        r#"(() => {{
  const W = {w}, H = {h};
  const canvas = document.createElement('canvas');
  canvas.width = W; canvas.height = H;
  document.body.appendChild(canvas);
  const ctx = canvas.getContext('2d');
  ctx.imageSmoothingEnabled = false;
  ctx.fillStyle = '#000'; ctx.fillRect(0, 0, W, H);
  const stream = canvas.captureStream(0);
  const track = stream.getVideoTracks()[0];
  const type = ['video/webm;codecs=vp8', 'video/webm'].find((t) => MediaRecorder.isTypeSupported(t));
  if (!type) throw new Error('MediaRecorder cannot record WebM in this browser');
  const recorder = new MediaRecorder(stream, {{ mimeType: type, videoBitsPerSecond: 8000000 }});
  const chunks = [];
  recorder.ondataavailable = (e) => {{ if (e.data.size) chunks.push(e.data); }};
  recorder.start(100);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  window.__astroshotRec = {{
    async add(b64, mime, holdMs) {{
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      const bmp = await createImageBitmap(new Blob([bytes], {{ type: mime }}));
      ctx.drawImage(bmp, 0, 0, W, H);
      bmp.close();
      track.requestFrame();
      await sleep(holdMs);
      track.requestFrame();
    }},
    async stop(holdMs) {{
      track.requestFrame();
      await sleep(holdMs);
      track.requestFrame();
      // The encoder starts lazily on the first frame and takes ~70 ms to
      // emit its first data on an idle machine, far longer under load.
      // `timeslice` chunks only arrive at stop, so flush with requestData()
      // until the recorder has produced something rather than guessing a hold.
      const deadline = performance.now() + {warmup_ms};
      while (!chunks.length && performance.now() < deadline) {{
        recorder.requestData();
        await sleep(5);
      }}
      recorder.requestData();
      const stopped = new Promise((r) => (recorder.onstop = r));
      recorder.stop();
      await stopped;
      const blob = new Blob(chunks, {{ type: 'video/webm' }});
      return await new Promise((resolve, reject) => {{
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(',')[1] || '');
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(blob);
      }});
    }},
  }};
  return true;
}})()"#,
        w = size.width,
        h = size.height,
        warmup_ms = ENCODER_WARMUP_LIMIT_MS,
    );
    page.evaluate::<bool>(&setup).await?;

    let engine = base64::engine::general_purpose::STANDARD;
    for frame in frames {
        let bytes = frame.as_ref();
        let mime = if bytes.starts_with(&PNG_MAGIC) {
            "image/png"
        } else {
            "image/jpeg"
        };
        let call = format!(
            "window.__astroshotRec.add({}, {}, {frame_ms}).then(() => true)",
            js_str(&engine.encode(bytes)),
            js_str(mime),
        );
        page.evaluate::<bool>(&call).await?;
    }

    let b64: String = page
        .evaluate(&format!("window.__astroshotRec.stop({tail_hold_ms})"))
        .await?;
    let webm = engine
        .decode(b64)
        .map_err(|e| BrowserError::Script(format!("bad WebM payload: {e}")))?;
    if webm.is_empty() {
        return Err(BrowserError::Script(
            "Chromium did not produce a WebM video".into(),
        ));
    }
    Ok(webm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{LaunchOptions, ScreenshotOptions, find_chrome};

    /// The encoder needs ~70 ms after the first frame before it emits data.
    /// With no tail hold at all, `stop()` runs inside that window, so the
    /// recorder must wait for output instead of finalizing an empty blob.
    #[tokio::test]
    async fn stop_waits_for_the_encoder_instead_of_a_fixed_hold() {
        if find_chrome().is_err() {
            eprintln!("skipping: no Chrome");
            return;
        }
        let browser = Browser::launch(LaunchOptions::default()).await.unwrap();
        let page = browser.new_page(PageOptions::new(64, 48)).await.unwrap();
        page.set_content("<body style='margin:0;background:#00f'></body>")
            .await
            .unwrap();
        let frame = page.screenshot(ScreenshotOptions::default()).await.unwrap();
        page.close().await.unwrap();
        let size = Size {
            width: 64,
            height: 48,
        };
        for _ in 0..5 {
            let page = browser
                .new_page(PageOptions::new(size.width, size.height))
                .await
                .unwrap();
            let webm = replay(&page, std::slice::from_ref(&frame), size, 1, 0)
                .await
                .unwrap();
            page.close().await.unwrap();
            assert_eq!(&webm[..4], &[0x1A, 0x45, 0xDF, 0xA3]);
        }
        browser.close().await.unwrap();
    }
}
