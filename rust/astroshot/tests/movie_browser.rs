//! Port-side tests for `movie_harness::sources::browser::record_browser_movie`.
//! Needs Chrome and ffmpeg; the scriptPath test also needs `node` >=22 and the
//! workspace `node_modules` (playwright-core). Skipped with a reason otherwise.

mod common;

use std::path::Path;
use std::process::Command;

use astroshot::browser::find_chrome;
use astroshot::movie_harness::sources::browser::{
    BrowserMovieSessionOptions, record_browser_movie,
};
use astroshot::movie_harness::types::{
    BrowserMovieOptions, MovieArtifact, MovieSessionOptions, MovieSourceKind,
};
use serde_json::Value;

fn data_url(html: &str) -> String {
    let encoded: String = html
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("data:text/html;charset=utf-8,{encoded}")
}

fn skip_reason() -> Option<String> {
    if let Err(error) = find_chrome() {
        return Some(error.to_string());
    }
    let ffmpeg = Command::new("ffmpeg").arg("-version").output();
    if ffmpeg.is_err() {
        return Some("ffmpeg not installed".into());
    }
    None
}

fn options(root: &Path, slug: &str, browser: BrowserMovieOptions) -> BrowserMovieSessionOptions {
    BrowserMovieSessionOptions {
        session: MovieSessionOptions {
            feature: "browser-journey".into(),
            slug: slug.into(),
            root: Some(root.to_string_lossy().into_owned()),
            run_id: Some("run-1".into()),
            fps: Some(10.0),
            size: Some(astroshot::movie_harness::types::Size {
                width: 320,
                height: 200,
            }),
            title: None,
            description: None,
            format: None,
            status: None,
            source: MovieSourceKind::Frames,
        },
        browser,
    }
}

fn manifest_for(artifact: &MovieArtifact) -> Value {
    let video = Path::new(&artifact.video_path);
    let feature_dir = video.parent().unwrap().parent().unwrap_or(video);
    for dir in [video.parent().unwrap(), feature_dir] {
        let candidate = dir.join("manifest.json");
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return serde_json::from_str(&text).unwrap();
        }
    }
    panic!("no manifest.json near {}", video.display());
}

/// RGB of the pixel at (x, y) in a PNG file.
fn pixel(png: &Path, x: u32, y: u32) -> [u8; 3] {
    let image = image::open(png).unwrap().to_rgb8();
    image.get_pixel(x, y).0
}

/// The last frame of a video as a PNG, via ffmpeg.
fn last_frame(video: &str, out: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-sseof", "-0.15", "-i", video])
        .args(["-frames:v", "1"])
        .arg(out)
        .status()
        .unwrap();
    assert!(status.success());
}

fn video_seconds(video: &str) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "csv=p=0", video])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// `codec_name`, `r_frame_rate` and decoded frame count of the video stream.
fn video_stream(video: &str) -> (String, String, u32) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
        .args([
            "-show_entries",
            "stream=codec_name,r_frame_rate,nb_read_frames",
        ])
        .args(["-of", "default=nw=1:nk=0", video])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {text}"))
            .to_string()
    };
    (
        field("codec_name"),
        field("r_frame_rate"),
        field("nb_read_frames").parse().unwrap(),
    )
}

/// Every frame of a video as PNG files, in order.
fn all_frames(video: &str, dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::create_dir_all(dir).unwrap();
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i", video])
        .arg(dir.join("%04d.png"))
        .status()
        .unwrap();
    assert!(status.success());
    let mut frames: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    frames.sort();
    frames
}

/// The TS source records with Playwright `recordVideo`: 25 fps whatever the
/// session fps, and a page that stops repainting is held for
/// `max(time since its last frame, 1 s)`. The manifest duration is the
/// session wall clock (`session.elapsedMs()`), not the video's length.
/// Observed from TS on this kind of page: 24 frames (0.96 s) with no settle,
/// 35-37 frames (about 1.44 s) with `settleMs: 1500`.
#[tokio::test(flavor = "multi_thread")]
async fn a_static_page_is_recorded_on_playwrights_timeline() {
    if let Some(reason) = skip_reason() {
        common::skip(&reason);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let html = "<body style=\"margin:0;background:#2040c0\"><p>static</p></body>";
    let record = |slug: &'static str, settle_ms: Option<u64>| {
        let request = options(
            root.path(),
            slug,
            BrowserMovieOptions {
                url: Some(data_url(html)),
                settle_ms,
                ..Default::default()
            },
        );
        async move {
            let started = std::time::Instant::now();
            let artifact = record_browser_movie(request).await.unwrap();
            (artifact, started.elapsed().as_millis() as f64)
        }
    };

    // No settle: the page's last frame is held for the 1 s minimum.
    let (still, still_wall_ms) = record("still", None).await;
    let (codec, rate, frames) = video_stream(&still.video_path);
    assert_eq!(codec, "vp9");
    assert_eq!(rate, "25/1", "the session fps (10) does not set the rate");
    // The upper bounds here leave room for a loaded machine: the hold runs
    // until the poster has been captured.
    assert!((24..=45).contains(&frames), "{frames} frames");
    let seconds = video_seconds(&still.video_path);
    assert!(
        (seconds - f64::from(frames) / 25.0).abs() < 0.05,
        "{seconds}"
    );
    // Wall clock from session creation: includes the browser launch, so it is
    // unrelated to the video's length and bounded by the whole call.
    assert!(still.duration_ms > 0.0 && still.duration_ms <= still_wall_ms);
    assert_eq!(still.duration_ms.fract(), 0.0);

    // 1.5 s settle: the hold is the idle time, about 1.5 s, not 1 s + 1.5 s.
    let (settled, settled_wall_ms) = record("settled", Some(1500)).await;
    let (_, _, frames) = video_stream(&settled.video_path);
    // 1 s + 1.5 s would be 62 frames.
    assert!((35..=58).contains(&frames), "{frames} frames");
    assert!(settled.duration_ms >= 1500.0 && settled.duration_ms <= settled_wall_ms);

    let manifest = manifest_for(&settled);
    let shots = manifest["shots"].as_array().unwrap();
    for (artifact, slug) in [(&still, "still"), (&settled, "settled")] {
        let shot = shots.iter().find(|shot| shot["slug"] == slug).unwrap();
        assert_eq!(shot["duration_ms"].as_f64(), Some(artifact.duration_ms));
        assert_eq!(shot["source"], "browser");
    }
    // The frame is the whole viewport at its own size, not a window-sized
    // crop padded out by the encoder.
    let frame = root.path().join("still-last.png");
    last_frame(&still.video_path, &frame);
    for (x, y) in [(5, 195), (315, 195), (315, 100)] {
        let [r, g, b] = pixel(&frame, x, y);
        assert!(r < 80 && g < 110 && b > 140, "({x},{y}) = {r} {g} {b}");
    }
}

/// Picture quality, where this deliberately leaves TS behind: the recording
/// is 2 device pixels per CSS pixel, its colours are the page's (the TS-era
/// encode showed dark backgrounds darker), and the stream is tagged so
/// players do not guess.
#[tokio::test(flavor = "multi_thread")]
async fn a_dark_page_is_recorded_at_twice_the_viewport_in_its_own_colours() {
    if let Some(reason) = skip_reason() {
        common::skip(&reason);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let html = r##"<body style="margin:0;background:#090a12"><div style="width:160px;height:200px;background:#808080"></div></body>"##;
    let artifact = record_browser_movie(options(
        root.path(),
        "dark",
        BrowserMovieOptions {
            url: Some(data_url(html)),
            settle_ms: Some(300),
            ..Default::default()
        },
    ))
    .await
    .unwrap();

    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=width,height,color_range,color_space,color_transfer,color_primaries")
        .args(["-of", "csv=p=0", &artifact.video_path])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&probe.stdout).trim(),
        "640,400,tv,bt709,iec61966-2-1,bt709"
    );
    let poster = image::open(&artifact.poster_path).unwrap();
    assert_eq!((poster.width(), poster.height()), (640, 400));

    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-sseof", "-0.15", "-i", &artifact.video_path])
        .args(["-frames:v", "1", "-vf"])
        .arg("scale=flags=accurate_rnd+full_chroma_int,format=rgb24")
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    let at = |x: usize, y: usize| &decoded.stdout[(y * 640 + x) * 3..][..3];
    for (x, y, want) in [(160, 200, [0x80u8; 3]), (500, 200, [0x09, 0x0a, 0x12])] {
        let got = at(x, y);
        for (got_channel, want_channel) in got.iter().zip(want) {
            assert!(
                got_channel.abs_diff(want_channel) <= 3,
                "({x},{y}) = {got:?}, page colour {want:?}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_animating_page_yields_changing_frames() {
    if let Some(reason) = skip_reason() {
        common::skip(&reason);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    // Red, then green, then blue, a second apart, then still. The page runs
    // on its own clock, so each colour stays up long enough to survive a
    // stalled test process.
    let html = r##"<body style="margin:0;background:#c00000"><script>
setTimeout(() => { document.body.style.background = '#00c000'; }, 1000);
setTimeout(() => { document.body.style.background = '#0000c0'; }, 2000);
</script></body>"##;
    let artifact = record_browser_movie(options(
        root.path(),
        "animated",
        BrowserMovieOptions {
            url: Some(data_url(html)),
            settle_ms: Some(2400),
            ..Default::default()
        },
    ))
    .await
    .unwrap();

    let frames = all_frames(&artifact.video_path, &root.path().join("frames"));
    let dominant = |path: &std::path::PathBuf| {
        let [r, g, b] = pixel(path, 160, 100);
        match (r > 120, g > 120, b > 120) {
            (true, false, false) => 'r',
            (false, true, false) => 'g',
            (false, false, true) => 'b',
            _ => '?',
        }
    };
    let mut seen: Vec<char> = frames.iter().map(dominant).filter(|c| *c != '?').collect();
    let total = seen.len();
    seen.dedup();
    assert_eq!(seen, ['r', 'g', 'b'], "colour runs across {total} frames");
    // Each colour is on screen for a second, about 25 frames at 25 fps; the
    // last one is then held for at least a second.
    let run = |colour: char| frames.iter().filter(|f| dominant(f) == colour).count();
    assert!((15..=32).contains(&run('r')), "red for {} frames", run('r'));
    assert!(
        (15..=32).contains(&run('g')),
        "green for {} frames",
        run('g')
    );
    assert!(run('b') >= 24, "blue for {} frames", run('b'));
}

#[tokio::test(flavor = "multi_thread")]
async fn records_a_data_url_journey_into_a_movie() {
    if let Some(reason) = skip_reason() {
        common::skip(&reason);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let html = "<body style=\"margin:0;background:#2040c0\"><p>hello journey</p></body>";
    let artifact = record_browser_movie(options(
        root.path(),
        "data-url",
        BrowserMovieOptions {
            url: Some(data_url(html)),
            settle_ms: Some(600),
            ..Default::default()
        },
    ))
    .await
    .unwrap();

    assert_eq!(artifact.source, MovieSourceKind::Browser);
    assert!(Path::new(&artifact.video_path).is_file());
    assert!(Path::new(&artifact.poster_path).is_file());
    assert!(artifact.duration_ms >= 400.0, "{}", artifact.duration_ms);
    let seconds = video_seconds(&artifact.video_path);
    assert!(seconds > 0.3 && seconds < 30.0, "{seconds}");
    assert_eq!(
        pixel(Path::new(&artifact.poster_path), 300, 190),
        [0x20, 0x40, 0xc0]
    );

    let manifest = manifest_for(&artifact);
    let text = manifest.to_string();
    assert!(text.contains("browser-journey"), "{text}");
    assert!(text.contains("\"browser\""), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_a_script_path_module_that_clicks_a_button() {
    if let Some(reason) = skip_reason() {
        common::skip(&reason);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("journey.mjs");
    std::fs::write(
        &script,
        r##"export default async function (page) {
  await page.click("#go");
  await page.waitForFunction(() => document.querySelector("#out").textContent === "clicked");
}
"##,
    )
    .unwrap();
    let html = r##"<body style="margin:0;background:#2040c0">
<button id="go" style="position:absolute;top:100px;left:10px" onclick="document.querySelector('#out').textContent='clicked';document.body.style.background='#c04020'">go</button>
<p id="out">idle</p></body>"##;
    let artifact = record_browser_movie(options(
        root.path(),
        "script",
        BrowserMovieOptions {
            url: Some(data_url(html)),
            script_path: Some(script.to_string_lossy().into_owned()),
            settle_ms: Some(300),
            ..Default::default()
        },
    ))
    .await
    .unwrap();

    // The poster is the final state; the encoded video ends on it too.
    assert_eq!(
        pixel(Path::new(&artifact.poster_path), 300, 190),
        [0xc0, 0x40, 0x20]
    );
    let frame = root.path().join("last.png");
    last_frame(&artifact.video_path, &frame);
    let [r, g, b] = pixel(&frame, 300, 190);
    assert!(r > 150 && g < 110 && b < 90, "{r} {g} {b}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_script_path_fails_with_the_ts_message() {
    if let Some(reason) = skip_reason() {
        common::skip(&reason);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("nope.mjs");
    let error = record_browser_movie(options(
        root.path(),
        "missing",
        BrowserMovieOptions {
            script_path: Some(missing.to_string_lossy().into_owned()),
            ..Default::default()
        },
    ))
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("browser script not found: {}", missing.display())
    );
}
