//! `take_tui_shot` end to end on the package's Ink fixtures: real `node` +
//! `helper.mjs` render the fixture, Rust checks `expectText` and rasterizes.
//! Ports `packages/tui-shot/src/shot-concurrency.e2e.test.ts` and the first
//! case of `packages/tui-shot/src/shot.e2e.test.ts` (the second is in
//! `tui_shot_helper.rs`).
//!
//! Needs `node` >=22 and the workspace `node_modules`; skipped without Node.

mod common;

use std::path::{Path, PathBuf};

use astroshot_engine::node_helper::find_node;
use astroshot_engine::tui_shot::shot::{close_shared_browser, take_tui_shot};
use astroshot_engine::tui_shot::types::TuiShotRequest;

fn repo() -> PathBuf {
    // Not canonicalized: the worktree's node_modules may be a symlink.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

/// Run from `packages/tui-shot`, as the TS suite does (`path.resolve(
/// "fixtures/...")`): tsx reads the JSX setting from the tsconfig.json in the
/// working directory. Every test in this binary sets the same directory.
fn node_or_skip() -> bool {
    if let Err(error) = find_node() {
        common::skip(&error);
        return false;
    }
    std::env::set_current_dir(repo().join("packages/tui-shot")).unwrap();
    true
}

fn request(fixture: &str, out: &Path) -> TuiShotRequest {
    TuiShotRequest {
        fixture_path: fixture.to_string(),
        out_path: out.display().to_string(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waits_for_all_queued_captures_before_close_shared_browser_resolves() {
    if !node_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let first_output = dir.path().join("first.png");
    let second_output = dir.path().join("second.png");

    // Queue both real Ink captures without awaiting either. A TS promise
    // starts when it is created; a spawned task is the Rust equivalent. The
    // queue slots are taken by the `take_tui_shot` calls, in this order.
    let first_capture = tokio::spawn(take_tui_shot(&request("fixtures/basic.tsx", &first_output)));
    let second_capture = tokio::spawn(take_tui_shot(&request(
        "fixtures/basic.tsx",
        &second_output,
    )));
    assert!(!first_output.exists() && !second_output.exists());

    // Closing is a lifecycle barrier: when it resolves, the captures queued
    // before it have finished and written their files.
    close_shared_browser().await.unwrap();
    assert!(
        first_capture.is_finished(),
        "first capture ended before close"
    );
    assert!(
        second_capture.is_finished(),
        "second capture ended before close"
    );
    assert!(first_output.exists() && second_output.exists());

    assert_eq!(
        first_capture.await.unwrap().unwrap(),
        first_output.to_string_lossy()
    );
    assert_eq!(
        second_capture.await.unwrap().unwrap(),
        second_output.to_string_lossy()
    );
    // basic.tsx: 42x8 cells at scale 1. TS asserted only a byte size
    // (> 4000) of the Chromium PNG; the native PNG is checked by pixel size
    // and by being identical for the same fixture.
    for output in [&first_output, &second_output] {
        let png = image::open(output).unwrap().to_rgba8();
        assert_eq!((png.width(), png.height()), (435, 203));
        assert!(std::fs::metadata(output).unwrap().len() > 4_000);
    }
    assert_eq!(
        std::fs::read(&first_output).unwrap(),
        std::fs::read(&second_output).unwrap()
    );
}

#[tokio::test]
async fn rejects_a_fixture_when_its_intended_visible_state_did_not_render() {
    if !node_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tui-shot-missing.png");

    // The real fixture renders "Unexpected screen" but expects "Intended
    // screen"; the rejection happens after the Node render, in Rust.
    let error = take_tui_shot(&request("fixtures/missing-expected-text.tsx", &out))
        .await
        .unwrap_err();

    // The full message `tui-shot shot` prints for this fixture.
    assert_eq!(
        error.to_string(),
        "Fixture did not render expected text \"Intended screen\". Visible frame:\nUnexpected screen"
    );
    assert!(!out.exists(), "no PNG is written for a rejected fixture");
}
