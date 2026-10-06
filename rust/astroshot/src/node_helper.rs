//! Client for the Node helper (`rust/node-helper/helper.mjs`, PORTING.md
//! decision 1).
//!
//! React and Ink fixtures are the user's own TSX, so something has to run it:
//! a small Node process loads configs and fixtures, serves React fixtures with
//! vite and renders Ink fixtures to ANSI. Rust spawns it and speaks
//! newline-delimited JSON over stdio:
//!
//! ```text
//! helper -> {"ready":true,"protocol":1,"node":"v22.14.0"}      once
//! rust   -> {"id":1,"cmd":"ink-render","fixture":"/abs/x.tsx"}
//! helper -> {"id":1,"result":{...}} | {"id":1,"error":{"message":"..."}}
//! ```
//!
//! Requests are answered in order, one at a time. Dropping [`NodeHelper`]
//! kills the child; [`NodeHelper::shutdown`] stops it cleanly (closing any vite
//! servers first).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command as TokioCommand};
use tokio::time::timeout;

use crate::react_shot::types::{ReactShotConfig, ReactShotFixtureMeta};

/// Wire protocol version; must match `PROTOCOL` in `helper.mjs`.
pub const PROTOCOL_VERSION: u32 = 1;
/// Minimum supported Node major version.
pub const MIN_NODE_MAJOR: u32 = 22;
/// Env var overriding the helper script location.
pub const HELPER_ENV: &str = "ASTROSHOT_NODE_HELPER";
/// Env var overriding the `node` executable.
pub const NODE_ENV: &str = "ASTROSHOT_NODE";

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const STDERR_TAIL: usize = 8 * 1024;

#[derive(Debug, Error)]
pub enum NodeHelperError {
    #[error(
        "React and Ink shots need Node.js ≥{MIN_NODE_MAJOR}, but `node` was not found on PATH. Install Node.js from https://nodejs.org and retry."
    )]
    NodeMissing,
    #[error("React and Ink shots need Node.js ≥{MIN_NODE_MAJOR}, but found {found}.")]
    NodeTooOld { found: String },
    #[error("could not find the astroshot Node helper (searched: {}). Set {HELPER_ENV} to helper.mjs.", .searched.join(", "))]
    HelperNotFound { searched: Vec<String> },
    #[error("could not start the astroshot Node helper: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("astroshot Node helper did not respond within {0:?}")]
    Timeout(Duration),
    #[error("astroshot Node helper exited unexpectedly{}", fmt_stderr(.stderr))]
    Exited { stderr: String },
    #[error("astroshot Node helper protocol error: {0}")]
    Protocol(String),
    /// An error reported by the helper. The message matches the TS CLIs.
    #[error("{message}")]
    Remote {
        message: String,
        stack: Option<String>,
    },
}

fn fmt_stderr(stderr: &str) -> String {
    if stderr.trim().is_empty() {
        String::new()
    } else {
        format!(":\n{}", stderr.trim_end())
    }
}

// ------------------------------------------------------------------ protocol

/// A request body. `id` is added by the client. Serializes as
/// `{"cmd":"react-serve","fixture":...}` with camelCase fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "cmd",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    Ping,
    /// Evaluate `react-shot.config.*` (`config.ts` `loadConfig`).
    LoadConfig {
        #[serde(skip_serializing_if = "Option::is_none")]
        config_path: Option<String>,
        /// Search upward from here when `config_path` is absent.
        #[serde(skip_serializing_if = "Option::is_none")]
        start_dir: Option<String>,
    },
    /// Start a vite server for one fixture (`create-server.ts` `startShotServer`).
    ReactServe {
        fixture: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        root: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        config_path: Option<String>,
    },
    ReactStop {
        server_id: u64,
    },
    /// Render an Ink fixture to one ANSI frame (`render-ink.ts`).
    InkRender {
        fixture: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cols: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rows: Option<u32>,
    },
    Shutdown,
}

#[derive(Debug, Serialize)]
struct Envelope<'a> {
    id: u64,
    #[serde(flatten)]
    command: &'a Command,
}

#[derive(Debug, Deserialize)]
struct Ready {
    ready: bool,
    protocol: u32,
    node: String,
}

#[derive(Debug, Deserialize)]
struct Reply {
    id: Option<u64>,
    result: Option<serde_json::Value>,
    error: Option<RemoteError>,
}

#[derive(Debug, Deserialize)]
struct RemoteError {
    message: String,
    stack: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingReply {
    pub protocol: u32,
    pub node: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadConfigReply {
    pub config_path: Option<String>,
    pub config: ReactShotConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactServeReply {
    pub server_id: u64,
    /// `http://127.0.0.1:<port>/`
    pub url: String,
    pub fixture_path: String,
    /// Resolved package root (`resolvePackageRoot`).
    pub package_root: String,
    pub config_path: Option<String>,
    pub config: ReactShotConfig,
    /// Metadata a plain Node import of the fixture could read; empty when the
    /// fixture needs vite to execute (the browser reports it then).
    pub node_meta: ReactShotFixtureMeta,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactStopReply {
    pub stopped: bool,
}

/// One Ink frame plus the fixture's raw styling. Defaults and validation
/// (`renderTuiShot`) and the `expectText` check happen in Rust.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InkRenderReply {
    pub ansi: String,
    pub cols: u32,
    pub rows: u32,
    #[serde(default)]
    pub expect_text: Option<Vec<String>>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub foreground: Option<String>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_size: Option<f64>,
    #[serde(default)]
    pub line_height: Option<f64>,
    #[serde(default)]
    pub padding: Option<f64>,
    #[serde(default)]
    pub border_radius: Option<f64>,
    #[serde(default)]
    pub scale: Option<f64>,
}

// ----------------------------------------------------------------- locating

/// Find the `node` executable: `ASTROSHOT_NODE`, else `PATH`.
pub fn find_node() -> Result<PathBuf, NodeHelperError> {
    if let Some(explicit) = std::env::var_os(NODE_ENV) {
        return Ok(PathBuf::from(explicit));
    }
    which::which("node").map_err(|_| NodeHelperError::NodeMissing)
}

/// Candidate helper locations in priority order: env override, next to the
/// executable (`node-helper/`, `../share/astroshot/node-helper/`,
/// `../lib/astroshot/node-helper/`), then the source checkout.
pub fn helper_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(explicit) = std::env::var_os(HELPER_ENV) {
        out.push(PathBuf::from(explicit));
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.canonicalize().unwrap_or(exe);
        if let Some(dir) = exe.parent() {
            out.push(dir.join("node-helper/helper.mjs"));
            out.push(dir.join("../share/astroshot/node-helper/helper.mjs"));
            out.push(dir.join("../lib/astroshot/node-helper/helper.mjs"));
        }
    }
    // Dev fallback: rust/astroshot/../node-helper in a source checkout.
    out.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../node-helper/helper.mjs"));
    out
}

pub fn find_helper() -> Result<PathBuf, NodeHelperError> {
    let candidates = helper_candidates();
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| NodeHelperError::HelperNotFound {
            searched: candidates.iter().map(|p| p.display().to_string()).collect(),
        })
}

fn node_major(version: &str) -> Option<u32> {
    version
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

// -------------------------------------------------------------------- client

/// A running helper process.
pub struct NodeHelper {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<String>>,
    next_id: u64,
    request_timeout: Duration,
    node_version: String,
}

impl NodeHelper {
    /// Locate node and the helper, spawn, and wait for the ready line.
    pub async fn spawn() -> Result<Self, NodeHelperError> {
        Self::spawn_with(&find_node()?, &find_helper()?, DEFAULT_STARTUP_TIMEOUT).await
    }

    pub async fn spawn_with(
        node: &Path,
        helper: &Path,
        startup_timeout: Duration,
    ) -> Result<Self, NodeHelperError> {
        let mut child = TokioCommand::new(node)
            .arg(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    NodeHelperError::NodeMissing
                } else {
                    NodeHelperError::Spawn(error)
                }
            })?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(mut pipe) = child.stderr.take() {
            let tail = Arc::clone(&stderr);
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                while let Ok(n) = pipe.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut tail = tail.lock().unwrap();
                    tail.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if tail.len() > STDERR_TAIL {
                        let mut cut = tail.len() - STDERR_TAIL;
                        while !tail.is_char_boundary(cut) {
                            cut += 1;
                        }
                        tail.drain(..cut);
                    }
                }
            });
        }
        let mut helper = Self {
            child,
            stdin,
            stdout,
            stderr,
            next_id: 1,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            node_version: String::new(),
        };
        let line = helper.read_line(startup_timeout).await?;
        let ready: Ready = serde_json::from_str(&line)
            .map_err(|e| NodeHelperError::Protocol(format!("bad ready line {line:?}: {e}")))?;
        if !ready.ready || ready.protocol != PROTOCOL_VERSION {
            return Err(NodeHelperError::Protocol(format!(
                "expected protocol {PROTOCOL_VERSION}, helper speaks {}",
                ready.protocol
            )));
        }
        if node_major(&ready.node).is_none_or(|major| major < MIN_NODE_MAJOR) {
            return Err(NodeHelperError::NodeTooOld { found: ready.node });
        }
        helper.node_version = ready.node;
        Ok(helper)
    }

    pub fn with_request_timeout(mut self, duration: Duration) -> Self {
        self.request_timeout = duration;
        self
    }

    pub fn node_version(&self) -> &str {
        &self.node_version
    }

    fn stderr_tail(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    async fn read_line(&mut self, limit: Duration) -> Result<String, NodeHelperError> {
        let mut line = String::new();
        match timeout(limit, self.stdout.read_line(&mut line)).await {
            Err(_) => {
                let _ = self.child.start_kill();
                Err(NodeHelperError::Timeout(limit))
            }
            Ok(Err(error)) => Err(NodeHelperError::Protocol(error.to_string())),
            Ok(Ok(0)) => {
                // Let the stderr reader drain before reporting.
                let _ = timeout(Duration::from_millis(200), self.child.wait()).await;
                tokio::task::yield_now().await;
                Err(NodeHelperError::Exited {
                    stderr: self.stderr_tail(),
                })
            }
            Ok(Ok(_)) => Ok(line),
        }
    }

    /// Send one command and decode its `result`.
    pub async fn call<R: DeserializeOwned>(
        &mut self,
        command: &Command,
    ) -> Result<R, NodeHelperError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut payload = serde_json::to_string(&Envelope { id, command })
            .map_err(|e| NodeHelperError::Protocol(e.to_string()))?;
        payload.push('\n');
        let stdin = self.stdin.as_mut().ok_or_else(|| NodeHelperError::Exited {
            stderr: String::new(),
        })?;
        if stdin.write_all(payload.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            return Err(NodeHelperError::Exited {
                stderr: self.stderr_tail(),
            });
        }
        loop {
            let line = self.read_line(self.request_timeout).await?;
            let reply: Reply = serde_json::from_str(&line).map_err(|e| {
                NodeHelperError::Protocol(format!("bad reply {:?}: {e}", line.trim_end()))
            })?;
            if reply.id.is_some_and(|got| got != id) {
                continue;
            }
            if let Some(error) = reply.error {
                return Err(NodeHelperError::Remote {
                    message: error.message,
                    stack: error.stack,
                });
            }
            let value = reply.result.unwrap_or(serde_json::Value::Null);
            return serde_json::from_value(value)
                .map_err(|e| NodeHelperError::Protocol(format!("bad result shape: {e}")));
        }
    }

    pub async fn ping(&mut self) -> Result<PingReply, NodeHelperError> {
        self.call(&Command::Ping).await
    }

    pub async fn load_config(
        &mut self,
        config_path: Option<&Path>,
        start_dir: Option<&Path>,
    ) -> Result<LoadConfigReply, NodeHelperError> {
        self.call(&Command::LoadConfig {
            config_path: config_path.map(|p| p.display().to_string()),
            start_dir: start_dir.map(|p| p.display().to_string()),
        })
        .await
    }

    pub async fn react_serve(
        &mut self,
        fixture: &Path,
        root: Option<&Path>,
        config_path: Option<&Path>,
    ) -> Result<ReactServeReply, NodeHelperError> {
        self.call(&Command::ReactServe {
            fixture: fixture.display().to_string(),
            root: root.map(|p| p.display().to_string()),
            config_path: config_path.map(|p| p.display().to_string()),
        })
        .await
    }

    pub async fn react_stop(&mut self, server_id: u64) -> Result<ReactStopReply, NodeHelperError> {
        self.call(&Command::ReactStop { server_id }).await
    }

    pub async fn ink_render(
        &mut self,
        fixture: &Path,
        cols: Option<u32>,
        rows: Option<u32>,
    ) -> Result<InkRenderReply, NodeHelperError> {
        self.call(&Command::InkRender {
            fixture: fixture.display().to_string(),
            cols,
            rows,
        })
        .await
    }

    /// Ask the helper to close its servers and exit, then reap it.
    pub async fn shutdown(mut self) -> Result<(), NodeHelperError> {
        let sent: Result<serde_json::Value, _> = self.call(&Command::Shutdown).await;
        self.stdin.take();
        if timeout(Duration::from_secs(5), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
        match sent {
            Ok(_) | Err(NodeHelperError::Exited { .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

// Dropping without `shutdown` kills the child (`kill_on_drop`); vite servers
// die with the process.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(command: &Command) -> serde_json::Value {
        serde_json::to_value(Envelope { id: 7, command }).unwrap()
    }

    #[test]
    fn serializes_commands_with_camel_case_fields() {
        assert_eq!(envelope(&Command::Ping), json!({"id": 7, "cmd": "ping"}));
        assert_eq!(
            envelope(&Command::ReactServe {
                fixture: "/a.tsx".into(),
                root: None,
                config_path: Some("/c.ts".into()),
            }),
            json!({"id": 7, "cmd": "react-serve", "fixture": "/a.tsx", "configPath": "/c.ts"})
        );
        assert_eq!(
            envelope(&Command::ReactStop { server_id: 3 }),
            json!({"id": 7, "cmd": "react-stop", "serverId": 3})
        );
        assert_eq!(
            envelope(&Command::InkRender {
                fixture: "/f.tsx".into(),
                cols: Some(42),
                rows: None
            }),
            json!({"id": 7, "cmd": "ink-render", "fixture": "/f.tsx", "cols": 42})
        );
        assert_eq!(
            envelope(&Command::LoadConfig {
                config_path: None,
                start_dir: Some("/d".into())
            }),
            json!({"id": 7, "cmd": "load-config", "startDir": "/d"})
        );
        assert_eq!(
            envelope(&Command::Shutdown),
            json!({"id": 7, "cmd": "shutdown"})
        );
    }

    #[test]
    fn deserializes_replies() {
        let ink: InkRenderReply = serde_json::from_value(json!({
            "ansi": "x", "cols": 42, "rows": 8, "expectText": ["a"], "scale": 1
        }))
        .unwrap();
        assert_eq!(ink.cols, 42);
        assert_eq!(ink.expect_text, Some(vec!["a".to_string()]));
        assert_eq!(ink.scale, Some(1.0));
        assert_eq!(ink.padding, None);

        let serve: ReactServeReply = serde_json::from_value(json!({
            "serverId": 1, "url": "http://127.0.0.1:1/", "fixturePath": "/f",
            "packageRoot": "/p", "configPath": null, "config": {},
            "nodeMeta": {"width": 800, "settleMs": 0, "selector": "[role=dialog]"}
        }))
        .unwrap();
        assert_eq!(serve.node_meta.width, Some(800.0));
        assert_eq!(serve.node_meta.wait_for, None);
        assert_eq!(serve.config, ReactShotConfig::default());
    }

    #[test]
    fn remote_error_displays_the_ts_message_only() {
        let reply: Reply = serde_json::from_str(
            r#"{"id":1,"error":{"message":"Fixture not found: /x","stack":"s"}}"#,
        )
        .unwrap();
        let error = reply.error.unwrap();
        let error = NodeHelperError::Remote {
            message: error.message,
            stack: error.stack,
        };
        assert_eq!(error.to_string(), "Fixture not found: /x");
    }

    #[test]
    fn missing_node_message_names_the_requirement() {
        assert!(
            NodeHelperError::NodeMissing
                .to_string()
                .starts_with("React and Ink shots need Node.js ≥22")
        );
    }

    #[test]
    fn parses_node_major() {
        assert_eq!(node_major("v22.14.0"), Some(22));
        assert_eq!(node_major("v20.1.0"), Some(20));
        assert_eq!(node_major("junk"), None);
    }

    #[test]
    fn helper_candidates_end_with_source_checkout() {
        let last = helper_candidates().pop().unwrap();
        assert!(last.ends_with("node-helper/helper.mjs"));
    }
}
