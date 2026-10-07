//! Ports `packages/react-shot/src/shot-concurrency.e2e.test.ts`.
//!
//! Two `take_shot` calls start together against one fixture. The fixture
//! fetches a slow local endpoint before it shows its ready text, and the
//! endpoint counts how many Chrome requests are in flight at once. Captures
//! that overlapped would both be waiting on it at the same time.
//!
//! This file is its own test binary with one test: `take_shot` shares a
//! process-wide Chrome that dies with the tokio runtime that launched it, so
//! everything here runs inside one runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use astroshot::browser::find_chrome;
use astroshot::node_helper::find_node;
use astroshot::react_shot::shot::{close_shared_browser, take_shot};
use astroshot::react_shot::types::ShotRequest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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
        eprintln!("SKIP: {error}");
        return false;
    }
    if let Err(error) = find_node() {
        eprintln!("SKIP: {error}");
        return false;
    }
    true
}

#[derive(Default)]
struct BrowserRequests {
    active: AtomicUsize,
    maximum_active: AtomicUsize,
    total: AtomicUsize,
}

/// The TS `readinessServer`: answers every request with "ready" after 500 ms
/// and tracks the requests whose User-Agent names Chrome.
async fn answer_after_delay(mut stream: TcpStream, requests: Arc<BrowserRequests>) {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let from_browser = head
        .lines()
        .filter_map(|line| line.strip_prefix("user-agent:"))
        .any(|agent| agent.contains("chrome"));
    if from_browser {
        requests.total.fetch_add(1, Ordering::SeqCst);
        let active = requests.active.fetch_add(1, Ordering::SeqCst) + 1;
        requests.maximum_active.fetch_max(active, Ordering::SeqCst);
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    if from_browser {
        requests.active.fetch_sub(1, Ordering::SeqCst);
    }
    let _ = stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready",
        )
        .await;
    let _ = stream.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn serializes_concurrent_captures_through_one_shared_browser_lifecycle() {
    if !tools_or_skip() {
        return;
    }
    let directory = tempfile::Builder::new()
        .prefix("react-shot-concurrency-")
        .tempdir()
        .unwrap();

    // Slow readiness endpoint on an ephemeral port.
    let requests = Arc::new(BrowserRequests::default());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn({
        let requests = requests.clone();
        async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(answer_after_delay(stream, requests.clone()));
            }
        }
    });

    // The fixture only shows "Concurrent ready" once its fetch resolves, so
    // each capture holds a request open for 500 ms.
    let fixture_path = directory.path().join("concurrent.tsx");
    std::fs::write(
        &fixture_path,
        format!(
            r#"import React, {{ useEffect, useState }} from "react";

const readiness = fetch("http://127.0.0.1:{port}/ready");

function ConcurrentFixture() {{
  const [ready, setReady] = useState(false);
  useEffect(() => {{
    void readiness.then(() => setReady(true));
  }}, []);
  return <div data-concurrent-shot>{{ready ? "Concurrent ready" : "Loading"}}</div>;
}}

export default {{
  width: 320,
  height: 120,
  selector: "[data-concurrent-shot]",
  waitFor: "text=Concurrent ready",
  component: <ConcurrentFixture />,
}};
"#
        ),
    )
    .unwrap();
    let outputs: Vec<String> = ["first.png", "second.png"]
        .iter()
        .map(|name| directory.path().join(name).to_string_lossy().into_owned())
        .collect();

    // Both captures start before either finishes (`Promise.all`). TS passes
    // `root: path.resolve(".")` with vitest running in packages/react-shot.
    let root = repo().join("packages/react-shot");
    let shot_requests: Vec<ShotRequest> = outputs
        .iter()
        .map(|out_path| ShotRequest {
            fixture_path: fixture_path.to_string_lossy().into_owned(),
            out_path: out_path.clone(),
            root: Some(root.to_string_lossy().into_owned()),
            ..ShotRequest::default()
        })
        .collect();
    let written = futures::future::join_all(shot_requests.iter().map(take_shot)).await;

    // TS `finally`: the shared browser closes whether or not the shots worked.
    // A close with no browser left is a no-op, as in TS.
    let closed = close_shared_browser().await;
    let closed_again = close_shared_browser().await;
    server.abort();

    let written: Vec<String> = written
        .into_iter()
        .map(|result| result.expect("take_shot"))
        .collect();
    assert_eq!(written, outputs);
    assert_eq!(
        requests.maximum_active.load(Ordering::SeqCst),
        1,
        "captures overlapped in the browser"
    );
    // One page load per capture reached the endpoint from Chrome.
    assert_eq!(requests.total.load(Ordering::SeqCst), 2);
    for output in &outputs {
        let png = std::fs::read(output).unwrap();
        assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "{output}");
    }
    closed.expect("close_shared_browser");
    closed_again.expect("second close_shared_browser");
}
