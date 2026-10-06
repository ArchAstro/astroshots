//! Ports the two `session.test.ts` cases that belong to this module
//! ("truecolor paint path"), plus recorded-PTY coverage with a scripted shell
//! (Unix only).

use super::*;
use crate::movie_harness::types::MovieFormat;

/// `preserves SGR truecolor in HTML`: the Rust paint path has no HTML, so the
/// color is checked on the terminal cell the rasterizer paints from.
#[test]
fn preserves_sgr_truecolor() {
    let mut terminal = HeadlessTerminal::new(24, 2);
    terminal.write(b"\x1b[38;2;124;92;255mPurple frame\x1b[0m");
    let frame = terminal.frame([0xff, 0xff, 0xff], [0x09, 0x0a, 0x12]);
    assert_eq!(frame.cell(0, 0).unwrap().foreground, [0x7c, 0x5c, 0xff]);
    assert!(terminal.plain_text().contains("Purple frame"));
}

fn options(root: &Path, feature: &str, slug: &str) -> MovieSessionOptions {
    MovieSessionOptions {
        feature: feature.into(),
        slug: slug.into(),
        root: Some(root.to_string_lossy().into_owned()),
        run_id: None,
        title: None,
        description: None,
        size: None,
        fps: Some(8.0),
        format: None,
        status: Some(ManifestStatus::Pass),
        source: MovieSourceKind::Pty,
    }
}

/// `records a truecolor pty-demo movie into .astroshot`.
#[tokio::test]
async fn records_a_truecolor_pty_demo_movie_into_astroshot() {
    let root = tempfile::tempdir().unwrap();
    let artifact = record_truecolor_demo_movie(options(root.path(), "pty-color", "brand"), None)
        .await
        .unwrap();
    assert!(Path::new(&artifact.poster_path).exists());
    assert!(Path::new(&artifact.video_path).exists());
    assert_eq!(artifact.source, MovieSourceKind::Pty);

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root.path().join(".astroshot/pty-color/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["shots"][0]["source"], "pty");
    assert_eq!(manifest["shots"][0]["kind"], "movie");
}

#[tokio::test]
async fn demo_movie_rejects_an_invalid_color() {
    let root = tempfile::tempdir().unwrap();
    let error =
        record_truecolor_demo_movie(options(root.path(), "pty-color", "bad"), Some("zzzzzz"))
            .await
            .unwrap_err();
    assert_eq!(error.to_string(), "invalid color zzzzzz");
}

#[test]
fn load_pty_movie_fixture_validates_the_document() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing.yaml");
    let error = load_pty_movie_fixture(missing.to_str().unwrap()).unwrap_err();
    assert!(error.to_string().starts_with("PTY fixture not found: "));

    let write = |name: &str, body: &str| {
        let path = temp.path().join(name);
        fs::write(&path, body).unwrap();
        load_pty_movie_fixture(path.to_str().unwrap())
            .unwrap_err()
            .to_string()
    };
    assert!(write("a.yaml", "- 1\n").ends_with(": document must be object"));
    assert!(write("b.yaml", "version: 2\ncommand: x\n").ends_with(": version must be 1"));
    assert!(write("c.json", r#"{"version":1,"command":" "}"#).ends_with(": command required"));

    let ok = temp.path().join("ok.yaml");
    fs::write(&ok, "version: 1\ncommand: sh\ncols: 30\n").unwrap();
    let fixture = load_pty_movie_fixture(ok.to_str().unwrap()).unwrap();
    assert_eq!(fixture.command, "sh");
    assert_eq!(fixture.cols, Some(30));
}

/// Records a scripted shell for about two seconds and checks the published
/// movie: manifest entry, poster and video on disk, session-sized frames.
#[cfg(unix)]
#[tokio::test]
async fn records_a_scripted_shell_into_a_movie() {
    let root = tempfile::tempdir().unwrap();
    let fixture = root.path().join("flow.pty.yaml");
    fs::write(
        &fixture,
        r#"version: 1
command: sh
args:
  - -c
  - "printf '\\033[38;2;124;92;255mready\\033[0m\\n'; sleep 1; echo done-step; sleep 1; echo finished"
cols: 40
rows: 8
movieFps: 10
timeoutMs: 20000
settleMs: 200
actions:
  - waitFor: ready
  - waitFor: done-step
  - waitFor: finished
expectText:
  - finished
"#,
    )
    .unwrap();

    let mut session = options(root.path(), "scripted-shell", "flow");
    session.format = Some(MovieFormat::Webm);
    let artifact = record_pty_movie(PtyMovieSessionOptions {
        session,
        fixture_path: fixture.to_string_lossy().into_owned(),
    })
    .await
    .unwrap();

    assert_eq!(artifact.source, MovieSourceKind::Pty);
    assert_eq!(artifact.sequence, "0001");
    assert!(Path::new(&artifact.video_path).exists());
    assert!(Path::new(&artifact.poster_path).exists());
    // Two seconds of a 10 fps sampler: a handful of frames, ending near 2s.
    assert!(
        artifact.duration_ms >= 1_000.0,
        "duration {}",
        artifact.duration_ms
    );

    // Poster is a frame: session size is css size + 32, frames are css * scale.
    let raster = RasterOptions::movie(40, 8);
    let (expected_width, expected_height) = raster.pixel_size();
    let poster = image::open(&artifact.poster_path).unwrap();
    assert_eq!(
        (poster.width(), poster.height()),
        (expected_width, expected_height)
    );

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root.path().join(".astroshot/scripted-shell/manifest.json")).unwrap(),
    )
    .unwrap();
    let shot = &manifest["shots"][0];
    assert_eq!(shot["source"], "pty");
    assert_eq!(shot["kind"], "movie");
    let (css_width, css_height) = raster.css_size();
    assert_eq!(
        shot["viewport"],
        format!("{}x{}", css_width + 32, css_height + 32)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pty_movie_fails_when_expected_text_never_renders() {
    let root = tempfile::tempdir().unwrap();
    let fixture = root.path().join("flow.pty.json");
    fs::write(
        &fixture,
        r#"{"version":1,"command":"echo","args":["hello"],"settleMs":100,"expectText":["nope"]}"#,
    )
    .unwrap();
    let error = record_pty_movie(PtyMovieSessionOptions {
        session: options(root.path(), "scripted-shell", "missing"),
        fixture_path: fixture.to_string_lossy().into_owned(),
    })
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("PTY movie did not render expected text \"nope\". Visible:\n"),
        "{error}"
    );
}
