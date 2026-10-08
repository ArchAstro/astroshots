//! Renders `packages/tui-shot/fixtures/basic.tsx` end to end: real `node` +
//! `helper.mjs` load and render the Ink fixture, `tui_shot::shot` applies the
//! defaults and checks `expectText`, and the native rasterizer writes the PNG.
//! Needs `node` >=22 and the workspace `node_modules`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use astroshot_engine::node_helper::NodeHelper;
use astroshot_engine::raster::HeadlessTerminal;
use astroshot_engine::tui_shot::shot::take_tui_shot;
use astroshot_engine::tui_shot::types::TuiShotRequest;

fn repo() -> PathBuf {
    // Not canonicalized: the worktree's node_modules may be a symlink.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

#[tokio::test]
async fn basic_ink_fixture_renders_to_a_png_through_the_node_helper_and_rasterizer() {
    let fixture = repo().join("packages/tui-shot/fixtures/basic.tsx");
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("shots/basic.png");

    // Node loads and renders the fixture, Rust rasterizes. `expectText` from
    // the fixture is enforced inside; a missing string would be an Err here.
    let written = take_tui_shot(&TuiShotRequest {
        fixture_path: fixture.display().to_string(),
        out_path: out.display().to_string(),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(written, out.to_string_lossy());

    // basic.tsx: 42x8 cells, scale 1 -> ceil(42*15*.62+44) x ceil(8*15*1.32+44).
    let png = image::open(&out).unwrap().to_rgba8();
    assert_eq!((png.width(), png.height()), (435, 203));
    let background = [0x09, 0x0a, 0x12, 255];
    let ink = png
        .pixels()
        .filter(|pixel| pixel.0[3] == 255 && pixel.0[..3] != background[..3] && pixel.0[0] > 150);
    assert!(ink.count() > 100, "glyph pixels were painted");
    assert_eq!(png.get_pixel(435 / 2, 203 / 2).0[3], 255);

    // The text the PNG was drawn from is the frame Ink produced.
    let mut helper = NodeHelper::spawn()
        .await
        .unwrap()
        .with_request_timeout(Duration::from_secs(90));
    let frame = helper.ink_render(&fixture, None, None).await.unwrap();
    helper.shutdown().await.unwrap();
    let mut terminal = HeadlessTerminal::new(frame.cols as u16, frame.rows as u16);
    terminal.write(b"\x1b[?7l");
    terminal.write(frame.ansi.as_bytes());
    let text = terminal.plain_text();
    for expected in frame.expect_text.unwrap() {
        assert!(text.contains(&expected), "{expected:?} in\n{text}");
    }
}

#[tokio::test]
async fn missing_ink_fixture_reports_the_ts_error() {
    let missing = repo().join("packages/tui-shot/fixtures/nope.tsx");
    let error = take_tui_shot(&TuiShotRequest {
        fixture_path: missing.display().to_string(),
        out_path: "unused.png".to_string(),
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Fixture not found: {}", missing.display())
    );
}
