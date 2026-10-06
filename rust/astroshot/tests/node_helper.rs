//! Spawns real `node` + `helper.mjs` against the packages' own fixtures.
//! Needs `node` >=22 and the workspace `node_modules` (npm install).

use std::path::{Path, PathBuf};
use std::time::Duration;

use astroshot::node_helper::{NodeHelper, NodeHelperError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn repo() -> PathBuf {
    // Not canonicalized: the worktree's node_modules may be a symlink.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

async fn spawn() -> NodeHelper {
    NodeHelper::spawn()
        .await
        .expect("node >=22 and rust/node-helper/helper.mjs")
        .with_request_timeout(Duration::from_secs(90))
}

#[tokio::test]
async fn ink_render_returns_the_ansi_frame_and_fixture_metadata() {
    let mut helper = spawn().await;
    let fixture = repo().join("packages/tui-shot/fixtures/basic.tsx");

    let frame = helper.ink_render(&fixture, None, None).await.unwrap();

    // basic.tsx sets cols/rows/scale and expectText itself.
    assert_eq!((frame.cols, frame.rows, frame.scale), (42, 8, Some(1.0)));
    assert!(frame.ansi.contains("✦ tui-shot"));
    assert!(frame.ansi.contains("\u{1b}[38;2;185;168;255m"), "truecolor");
    assert_eq!(frame.expect_text.as_ref().map(Vec::len), Some(2));

    // CLI overrides win over the fixture's own size.
    let wide = helper
        .ink_render(&fixture, Some(60), Some(10))
        .await
        .unwrap();
    assert_eq!((wide.cols, wide.rows), (60, 10));
    helper.shutdown().await.unwrap();
}

#[tokio::test]
async fn ink_render_reports_the_ts_error_messages() {
    let mut helper = spawn().await;
    let missing = repo().join("packages/tui-shot/fixtures/nope.tsx");
    let error = helper.ink_render(&missing, None, None).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Fixture not found: {}", missing.display())
    );

    let error = helper
        .ink_render(
            &repo().join("packages/tui-shot/fixtures/basic.tsx"),
            Some(0),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "cols must be a positive integer no greater than 1000"
    );

    // The helper survives errors.
    assert_eq!(helper.ping().await.unwrap().protocol, 1);
    helper.shutdown().await.unwrap();
}

#[tokio::test]
async fn react_serve_serves_the_fixture_page_until_stopped() {
    let mut helper = spawn().await;
    let fixture = repo().join("packages/react-shot/fixtures/cli-e2e.tsx");

    let served = helper.react_serve(&fixture, None, None).await.unwrap();

    assert!(served.url.starts_with("http://127.0.0.1:"));
    assert!(served.package_root.ends_with("packages/react-shot"));
    assert_eq!(served.config_path, None);

    // The host page and the virtual entry module come from vite.
    let html = http_get(&served.url, "/").await;
    assert!(html.contains("/@react-shot/entry.tsx"), "{html}");
    let entry = http_get(&served.url, "/@react-shot/entry.tsx").await;
    assert!(entry.contains("__REACT_SHOT_READY__"), "{entry}");

    assert!(helper.react_stop(served.server_id).await.unwrap().stopped);
    assert!(!helper.react_stop(served.server_id).await.unwrap().stopped);
    helper.shutdown().await.unwrap();
}

#[tokio::test]
async fn load_config_resolves_paths_against_the_config_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("react-shot.config.mjs"),
        r#"export default { alias: { "@": "./src" }, styles: ["./a.css"], stubModules: ["x"] };"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("deep/er")).unwrap();
    let mut helper = spawn().await;
    let root = dir.path().canonicalize().unwrap();
    let nested = root.join("deep/er");

    let loaded = helper.load_config(None, Some(&nested)).await.unwrap();

    let config_path = loaded.config_path.unwrap();
    assert!(config_path.ends_with("react-shot.config.mjs"));
    assert_eq!(
        loaded.config.alias.unwrap()["@"],
        root.join("src").display().to_string()
    );
    assert_eq!(loaded.config.stub_modules, Some(vec!["x".to_string()]));
    assert_eq!(loaded.config.root.unwrap(), root.display().to_string());

    let none = helper
        .load_config(None, Some(Path::new("/")))
        .await
        .unwrap();
    assert_eq!(none.config_path, None);
    let error = helper
        .load_config(Some(&root.join("missing.ts")), None)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("react-shot config not found: ")
    );
    helper.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_missing_helper_script_exits_with_a_clear_error() {
    let node = astroshot::node_helper::find_node().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("helper.mjs");
    std::fs::write(&script, "console.error('boom'); process.exit(3);").unwrap();
    let error = NodeHelper::spawn_with(&node, &script, Duration::from_secs(10))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&error, NodeHelperError::Exited { stderr } if stderr.contains("boom")),
        "{error}"
    );
}

#[tokio::test]
async fn a_nonexistent_node_binary_reports_the_node_requirement() {
    let error = NodeHelper::spawn_with(
        Path::new("/definitely/not/node"),
        Path::new("x.mjs"),
        Duration::from_secs(5),
    )
    .await
    .err()
    .unwrap();
    assert!(
        error
            .to_string()
            .starts_with("React and Ink shots need Node.js")
    );
}

#[tokio::test]
async fn dropping_the_client_kills_the_helper() {
    let mut helper = spawn().await;
    helper.ping().await.unwrap();
    drop(helper); // kill_on_drop: must not hang or leave the test process blocked
}

async fn http_get(base: &str, path: &str) -> String {
    let url = url::Url::parse(base).unwrap();
    let mut stream = tokio::net::TcpStream::connect((url.host_str().unwrap(), url.port().unwrap()))
        .await
        .unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        url.host_str().unwrap()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut body = String::new();
    stream.read_to_string(&mut body).await.unwrap();
    body
}
