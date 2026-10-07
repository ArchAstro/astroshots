//! End to end: Node helper serves a real TSX fixture, Chrome renders it, and
//! `take_shot` writes the PNG. Skipped (with a printed reason) when Chrome or
//! Node is missing.

mod common;

use std::path::{Path, PathBuf};

use astroshot::browser::find_chrome;
use astroshot::node_helper::find_node;
use astroshot::react_shot::shot::{close_shared_browser, take_shot_detailed};
use astroshot::react_shot::types::ShotRequest;

fn repo() -> PathBuf {
    // Not canonicalized: the worktree's node_modules may be a symlink.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn tools_or_skip() -> bool {
    if let Err(error) = find_chrome() {
        common::skip(&error);
        return false;
    }
    if let Err(error) = find_node() {
        common::skip(&error);
        return false;
    }
    true
}

fn png_size(path: &Path) -> (u32, u32) {
    let image = image::open(path).expect("PNG decodes");
    (image.width(), image.height())
}

/// One runtime runs both scenarios: `take_shot` shares a process-wide Chrome,
/// and Chrome's handler task dies with the runtime that launched it, so
/// separate `#[tokio::test]` runtimes would pull it out from under each other.
#[tokio::test(flavor = "multi_thread")]
async fn react_shot_end_to_end() {
    if !tools_or_skip() {
        return;
    }
    shoots_the_cli_e2e_dialog_fixture_from_node_helper_through_chrome_to_png().await;
    full_page_shot_is_viewport_size_times_scale_and_cli_viewport_wins().await;
    close_shared_browser().await.unwrap();
}

async fn shoots_the_cli_e2e_dialog_fixture_from_node_helper_through_chrome_to_png() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("nested/dialog.png");

    // Node helper serves cli-e2e.tsx; Chrome loads it at 800x600, scale 1.
    let shot = take_shot_detailed(&ShotRequest {
        fixture_path: repo()
            .join("packages/react-shot/fixtures/cli-e2e.tsx")
            .to_string_lossy()
            .into_owned(),
        out_path: out.to_string_lossy().into_owned(),
        ..ShotRequest::default()
    })
    .await
    .unwrap();

    // Metadata comes from the fixture; the dialog selector turns on overlay
    // removal and transparency.
    assert_eq!(shot.out_path, out);
    assert_eq!(shot.meta.width, 800.0);
    assert_eq!(shot.meta.height, 600.0);
    assert_eq!(shot.meta.selector, "[role=dialog]");
    assert_eq!(shot.meta.wait_for, None);
    assert_eq!(shot.meta.settle_ms, 0.0);
    assert!(!shot.meta.full_page);
    assert!(shot.meta.strip_overlay);
    assert!(shot.meta.omit_background);

    // The element PNG is the 400px dialog at device scale 1 (not the viewport).
    let (width, height) = png_size(&out);
    assert_eq!(width, 400);
    assert!((100..600).contains(&height), "height {height}");
    // Overlay removal + omitBackground: the rounded corner is transparent.
    let image = image::open(&out).unwrap().to_rgba8();
    assert_eq!(image.get_pixel(0, 0)[3], 0, "corner is transparent");
    assert_eq!(image.get_pixel(width / 2, height / 2)[3], 255);
}

async fn full_page_shot_is_viewport_size_times_scale_and_cli_viewport_wins() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/react-full-page.tsx");
    let request = |out: &Path, width: Option<u32>| ShotRequest {
        fixture_path: fixture.to_string_lossy().into_owned(),
        out_path: out.to_string_lossy().into_owned(),
        root: Some(
            repo()
                .join("packages/react-shot")
                .to_string_lossy()
                .into_owned(),
        ),
        width,
        ..ShotRequest::default()
    };

    // Fixture size 640x360, scale 1.
    let out = dir.path().join("full.png");
    let shot = take_shot_detailed(&request(&out, None)).await.unwrap();
    assert_eq!((shot.meta.width, shot.meta.height), (640.0, 360.0));
    assert!(shot.meta.full_page && !shot.meta.strip_overlay && !shot.meta.omit_background);
    assert_eq!(shot.meta.selector, "#card");
    assert_eq!(png_size(&out), (640, 360));

    // A CLI width beats the fixture's.
    let out = dir.path().join("wide.png");
    let shot = take_shot_detailed(&request(&out, Some(900))).await.unwrap();
    assert_eq!(shot.meta.width, 900.0);
    assert_eq!(png_size(&out), (900, 360));
}
