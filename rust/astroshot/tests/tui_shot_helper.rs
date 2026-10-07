//! The Ink render's global React bridge, through the real Node helper.
//!
//! TS `takeIsolatedTuiShot` (packages/tui-shot/src/shot.ts) sets
//! `globalThis.React` for the length of a capture and puts back whatever was
//! there, also when the capture fails
//! (packages/tui-shot/src/shot.e2e.test.ts, "restores an existing global React
//! bridge after a failed capture"). In the port that code is `cmdInkRender` in
//! `rust/node-helper/helper.mjs`, and the global lives in the helper process,
//! so the test reads it from there: a fixture module runs in the helper and
//! reports `globalThis.React` back through its `expectText`.
//!
//! Needs `node` >=22 and the workspace `node_modules`; skipped without Node.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use astroshot::node_helper::{NodeHelper, find_node};
use astroshot::tui_shot::shot::render_tui_shot;
use astroshot::tui_shot::types::TuiShotRequest;

fn repo() -> PathBuf {
    // Not canonicalized: the worktree's node_modules may be a symlink.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn own_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ink")
        .join(name)
}

/// Run from `packages/tui-shot`, as the TS suite does: tsx reads the JSX
/// setting (`react-jsx`) from the tsconfig.json in the working directory, and
/// missing-expected-text.tsx relies on it. Every test in this binary sets the
/// same directory, so the process-wide change cannot race.
fn enter_package_root() {
    std::env::set_current_dir(repo().join("packages/tui-shot")).unwrap();
}

async fn spawn_or_skip() -> Option<NodeHelper> {
    enter_package_root();
    if let Err(error) = find_node() {
        common::skip(&error);
        return None;
    }
    Some(
        NodeHelper::spawn()
            .await
            .expect("node >=22 and rust/node-helper/helper.mjs")
            .with_request_timeout(Duration::from_secs(90)),
    )
}

/// What the probe fixture saw in `globalThis.React` when it loaded.
async fn probe_react_global(helper: &mut NodeHelper) -> serde_json::Value {
    let reply = helper
        .ink_render(&own_fixture("react-bridge-probe.tsx"), None, None)
        .await
        .unwrap();
    serde_json::from_str(&reply.expect_text.unwrap()[0]).unwrap()
}

#[tokio::test]
async fn restores_an_existing_global_react_bridge_after_a_failed_capture() {
    let Some(mut helper) = spawn_or_skip().await else {
        return;
    };

    // Setup: the owner fixture defines a read-only `globalThis.React`
    // sentinel when it loads. Its component prints nothing, so the Ink render
    // throws while the helper's bridge has replaced the sentinel.
    let error = helper
        .ink_render(&own_fixture("react-bridge-owner.tsx"), None, None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Ink produced no printable frame. Check that the fixture renders visible content."
    );

    // The failed render put the sentinel back, descriptor included.
    let owned = serde_json::json!({
        "present": true,
        "source": "test-owner",
        "writable": false,
        "configurable": true,
    });
    assert_eq!(probe_react_global(&mut helper).await, owned);

    // The TS test's own failure: missing-expected-text.tsx renders, then the
    // capture is rejected for its expectText. The render happens in the
    // helper and the rejection in Rust; the sentinel survives both.
    let frame = helper
        .ink_render(
            &repo().join("packages/tui-shot/fixtures/missing-expected-text.tsx"),
            None,
            None,
        )
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("react.png");
    let rejected = render_tui_shot(
        &TuiShotRequest {
            fixture_path: "missing-expected-text.tsx".to_string(),
            out_path: out.display().to_string(),
            ..Default::default()
        },
        &frame,
    )
    .unwrap_err();
    assert!(
        rejected
            .to_string()
            .contains("did not render expected text"),
        "{rejected}"
    );
    assert_eq!(probe_react_global(&mut helper).await, owned);

    helper.shutdown().await.unwrap();
}

#[tokio::test]
async fn removes_its_global_react_bridge_when_none_existed() {
    let Some(mut helper) = spawn_or_skip().await else {
        return;
    };
    let absent = serde_json::json!({
        "present": false,
        "source": null,
        "writable": null,
        "configurable": null,
    });

    // A fresh helper has no `globalThis.React`.
    assert_eq!(probe_react_global(&mut helper).await, absent);
    // A successful render and a failed one both leave it absent again
    // (`Reflect.deleteProperty` in the `finally`).
    helper
        .ink_render(
            &repo().join("packages/tui-shot/fixtures/basic.tsx"),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(probe_react_global(&mut helper).await, absent);
    helper
        .ink_render(
            &repo().join("packages/tui-shot/fixtures/basic.tsx"),
            Some(0),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(probe_react_global(&mut helper).await, absent);

    helper.shutdown().await.unwrap();
}
