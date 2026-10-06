//! Port-side tests for `movie_harness::sources::browser::record_browser_movie`.
//! Needs Chrome and ffmpeg; the scriptPath test also needs `node` >=22 and the
//! workspace `node_modules` (playwright-core). Skipped with a reason otherwise.

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

#[tokio::test(flavor = "multi_thread")]
async fn records_a_data_url_journey_into_a_movie() {
    if let Some(reason) = skip_reason() {
        eprintln!("SKIP: {reason}");
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
        eprintln!("SKIP: {reason}");
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
        eprintln!("SKIP: {reason}");
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
