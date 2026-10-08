//! herdr pane-graphics transport.
//!
//! herdr (an agent terminal multiplexer) emulates the terminal itself and does
//! not forward a program's raw Kitty escapes. Instead it exposes a unix-socket
//! JSON API — `pane.graphics.set` / `pane.graphics.clear` place and remove
//! images on named layers composited over the pane's text. This module speaks
//! that API so the review tray shows pixel-perfect images inside herdr.
//!
//! Enabling it needs `[experimental] kitty_graphics = true` in herdr's
//! config.toml AND a client reattach: herdr 0.8.2 latches the client's graphics
//! setting at startup, so a client started with it off reports the host cell
//! size as unavailable until it is detached and reattached once.
//!
//! Wire format: newline-delimited JSON over the socket named by
//! HERDR_SOCKET_PATH, targeting HERDR_PANE_ID.
//!
//! Unix only: the transport is a unix-domain socket.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

/// Idle timeout of a short-lived request (`socket.setTimeout(2500)`).
const REQUEST_TIMEOUT: Duration = Duration::from_millis(2500);
/// Largest unterminated response a request accepts.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Probe gives up (and reports success) after this long.
const PROBE_TIMEOUT: Duration = Duration::from_millis(600);
/// A stream whose open reply never comes is assumed open after this grace so a
/// silent-ack build still renders.
const OPEN_GRACE: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Default)]
struct ReplyError {
    /// `None` renders as `undefined`, like the TS template string.
    code: Option<String>,
}

impl ReplyError {
    fn code(&self) -> &str {
        self.code.as_deref().unwrap_or("undefined")
    }
}

#[derive(Debug, Clone, Default)]
struct ReplyResult {
    cell_width_px: Option<f64>,
    cell_height_px: Option<f64>,
    max_layers_per_pane: Option<f64>,
}

#[derive(Debug, Clone, Default)]
struct Reply {
    error: Option<ReplyError>,
    result: Option<ReplyResult>,
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number
            .as_f64()
            .is_some_and(|number| number != 0.0 && !number.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

impl Reply {
    fn from_value(value: &Value) -> Self {
        let error = value
            .get("error")
            .filter(|error| js_truthy(error))
            .map(|error| ReplyError {
                code: error
                    .get("code")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        let result = value
            .get("result")
            .filter(|result| js_truthy(result))
            .map(|result| ReplyResult {
                cell_width_px: result.get("cell_width_px").and_then(Value::as_f64),
                cell_height_px: result.get("cell_height_px").and_then(Value::as_f64),
                max_layers_per_pane: result.get("max_layers_per_pane").and_then(Value::as_f64),
            });
        Self { error, result }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrAddress {
    pub socket: String,
    pub pane: String,
}

/// `env` looks up one variable; empty values count as unset (JS falsy).
pub fn herdr_address(env: impl Fn(&str) -> Option<String>) -> Option<HerdrAddress> {
    if env("HERDR_ENV").as_deref() != Some("1") {
        return None;
    }
    let socket = env("HERDR_SOCKET_PATH").filter(|value| !value.is_empty())?;
    let pane = env("HERDR_PANE_ID").filter(|value| !value.is_empty())?;
    Some(HerdrAddress { socket, pane })
}

/// `herdr_address` over the process environment.
pub fn herdr_address_from_env() -> Option<HerdrAddress> {
    herdr_address(|name| std::env::var(name).ok())
}

fn request_line(method: &str, params: Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(
        &json!({ "id": "astroshot-review", "method": method, "params": params }),
    )
    .expect("request serializes");
    line.push(b'\n');
    line
}

fn first_line(buffer: &[u8]) -> Option<&[u8]> {
    buffer
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|end| &buffer[..end])
}

/// A short-lived request/response over the socket.
async fn request(socket: &str, method: &str, params: Value) -> Result<Reply, String> {
    // Node's idle timeout covers connect and every wait for data.
    let idle = |message: &'static str| move |_| message.to_string();
    let mut client = timeout(REQUEST_TIMEOUT, UnixStream::connect(socket))
        .await
        .map_err(idle("herdr request timed out"))?
        .map_err(|error| error.to_string())?;
    client
        .write_all(&request_line(method, params))
        .await
        .map_err(|error| error.to_string())?;
    let mut text: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = timeout(REQUEST_TIMEOUT, client.read(&mut chunk))
            .await
            .map_err(idle("herdr request timed out"))?
            .map_err(|error| error.to_string())?;
        if read == 0 {
            // A hang-up before a full line leaves the request unanswered; it
            // ends through the idle timeout.
            sleep(REQUEST_TIMEOUT).await;
            return Err("herdr request timed out".to_string());
        }
        text.extend_from_slice(&chunk[..read]);
        let Some(line) = first_line(&text) else {
            if text.len() > MAX_RESPONSE_BYTES {
                return Err("herdr response too large".to_string());
            }
            continue;
        };
        return match serde_json::from_slice::<Value>(line) {
            Ok(value) => Ok(Reply::from_value(&value)),
            Err(_) => Err("invalid herdr response".to_string()),
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub ok: bool,
    pub reason: Option<String>,
}

impl ProbeResult {
    fn ok() -> Self {
        Self {
            ok: true,
            reason: None,
        }
    }
}

/// Confirm herdr accepts a `pane.graphics.stream` layer. A missing ack is
/// treated as success — the open reply may be silent — so only an explicit
/// error reply rejects it. The probe stream is closed immediately, which makes
/// herdr drop its layer.
pub async fn probe_herdr_set(address: &HerdrAddress) -> ProbeResult {
    let attempt = async {
        let Ok(mut client) = UnixStream::connect(&address.socket).await else {
            return ProbeResult::ok();
        };
        let line = request_line(
            "pane.graphics.stream",
            json!({ "pane_id": address.pane, "layer_id": "astroshot-review-probe", "z_index": -1 }),
        );
        if client.write_all(&line).await.is_err() {
            return ProbeResult::ok();
        }
        let mut text: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match client.read(&mut chunk).await {
                Err(_) => return ProbeResult::ok(),
                // Hung up without a line: nothing more arrives, the timer settles it.
                Ok(0) => std::future::pending::<()>().await,
                Ok(read) => text.extend_from_slice(&chunk[..read]),
            }
            let Some(line) = first_line(&text) else {
                continue;
            };
            return match serde_json::from_slice::<Value>(line) {
                Ok(value) => {
                    let reply = Reply::from_value(&value);
                    match reply.error {
                        Some(_) => ProbeResult {
                            ok: false,
                            reason: Some(reason_for(&reply)),
                        },
                        None => ProbeResult::ok(),
                    }
                }
                Err(_) => ProbeResult::ok(),
            };
        }
    };
    // Dropping the future on timeout closes the probe connection.
    timeout(PROBE_TIMEOUT, attempt)
        .await
        .unwrap_or_else(|_| ProbeResult::ok())
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HerdrDiscovery {
    pub ok: bool,
    pub cell_width: Option<u32>,
    pub cell_height: Option<u32>,
    pub max_layers: Option<u32>,
    /// Actionable reason when graphics can't be used yet.
    pub reason: Option<String>,
}

fn reason_for(reply: &Reply) -> String {
    if let Some(error) = &reply.error {
        match error.code() {
            "feature_disabled" => {
                return "herdr image rendering is off — set [experimental] kitty_graphics = true in ~/.config/herdr/config.toml, run `herdr server reload-config`, then reattach the herdr client".to_string();
            }
            "cell_size_unavailable" => {
                return "herdr hasn't reported pixel size — detach and reattach the herdr client once (its graphics setting latches at startup)".to_string();
            }
            code => return format!("herdr graphics unavailable ({code})"),
        }
    }
    "herdr returned no pixel dimensions for this pane".to_string()
}

#[derive(Default)]
pub struct DiscoverOptions {
    /// Defaults to 5000.
    pub timeout_ms: Option<u64>,
    /// Defaults to 150.
    pub retry_ms: Option<u64>,
    pub on_waiting: Option<Box<dyn FnMut() + Send>>,
}

fn truthy_number(value: Option<f64>) -> Option<f64> {
    value.filter(|value| *value != 0.0 && !value.is_nan())
}

/// Poll `pane.graphics.info` until herdr reports the cell size, or give up.
pub async fn discover_herdr(
    address: &HerdrAddress,
    mut options: DiscoverOptions,
) -> HerdrDiscovery {
    let deadline = Instant::now() + Duration::from_millis(options.timeout_ms.unwrap_or(5000));
    let mut announced = false;
    loop {
        let reply = match request(
            &address.socket,
            "pane.graphics.info",
            json!({ "pane_id": address.pane }),
        )
        .await
        {
            Ok(reply) => reply,
            Err(reason) => {
                return HerdrDiscovery {
                    ok: false,
                    reason: Some(reason),
                    ..Default::default()
                };
            }
        };
        let negotiating = reply
            .error
            .as_ref()
            .is_some_and(|error| error.code() == "cell_size_unavailable");
        if negotiating && Instant::now() < deadline {
            if !announced {
                announced = true;
                if let Some(on_waiting) = options.on_waiting.as_mut() {
                    on_waiting();
                }
            }
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u64;
            let wait = options.retry_ms.unwrap_or(150).min(remaining.max(1));
            sleep(Duration::from_millis(wait)).await;
            continue;
        }
        let size = reply.result.as_ref().and_then(|result| {
            Some((
                truthy_number(result.cell_width_px)?,
                truthy_number(result.cell_height_px)?,
                result,
            ))
        });
        let (Some((width, height, result)), None) = (size, &reply.error) else {
            return HerdrDiscovery {
                ok: false,
                reason: Some(reason_for(&reply)),
                ..Default::default()
            };
        };
        return HerdrDiscovery {
            ok: true,
            cell_width: Some(width as u32),
            cell_height: Some(height as u32),
            max_layers: Some(
                truthy_number(result.max_layers_per_pane).map_or(16, |layers| layers as u32),
            ),
            reason: None,
        };
    }
}

/// Extract PNG pixel dimensions from the IHDR chunk.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HerdrPlacement {
    pub col: u32,
    pub row: u32,
    pub cols: u32,
    pub rows: u32,
    pub z: i32,
}

struct Frame {
    png: Vec<u8>,
    placement: HerdrPlacement,
}

/// State shared between a layer and its stream task.
struct LayerShared {
    /// Latest frame to send once the stream is ready or drained.
    pending: Mutex<Option<Frame>>,
    wake: Notify,
}

struct StreamTask {
    /// Identifies the connection, so a task that outlived `clear()` can tell.
    token: u64,
    handle: JoinHandle<()>,
}

struct Layer {
    z: i32,
    shared: Arc<LayerShared>,
    /// The layer owns its stream task from the first moment of connecting:
    /// `clear` and `dispose` abort whatever is here, handshake or not.
    task: Option<StreamTask>,
}

type ErrorHandler = Arc<dyn Fn(anyhow::Error) + Send + Sync>;

struct Inner {
    address: HerdrAddress,
    on_error: ErrorHandler,
    layers: Mutex<HashMap<String, Layer>>,
    disposed: AtomicBool,
    generation: AtomicU64,
    next_token: AtomicU64,
    runtime: Handle,
}

impl Inner {
    fn layers(&self) -> MutexGuard<'_, HashMap<String, Layer>> {
        self.layers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Places images on herdr via one `pane.graphics.stream` connection PER layer.
/// herdr removes a stream's layer the moment its socket closes, so closing a
/// layer's connection — or the whole process dying — cleans up automatically,
/// with no persistent server-side state to leak (the trap of pane.graphics.set).
///
/// Each connection runs in its own tokio task; aborting the task drops the
/// socket and so closes it. Construct inside a tokio runtime.
pub struct HerdrSink {
    inner: Arc<Inner>,
}

impl HerdrSink {
    pub fn new(
        address: HerdrAddress,
        on_error: impl Fn(anyhow::Error) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                address,
                on_error: Arc::new(on_error),
                layers: Mutex::new(HashMap::new()),
                disposed: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                next_token: AtomicU64::new(0),
                runtime: Handle::current(),
            }),
        }
    }

    /// Bumps when a layer stream drops unexpectedly, so callers re-send.
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::SeqCst)
    }

    pub fn set(
        &self,
        layer_id: &str,
        png: Vec<u8>,
        _image_width: u32,
        _image_height: u32,
        placement: HerdrPlacement,
    ) {
        if self.inner.disposed.load(Ordering::SeqCst) {
            return;
        }
        let mut layers = self.inner.layers();
        let layer = layers.entry(layer_id.to_string()).or_insert_with(|| Layer {
            z: placement.z,
            shared: Arc::new(LayerShared {
                pending: Mutex::new(None),
                wake: Notify::new(),
            }),
            task: None,
        });
        *layer
            .shared
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Frame { png, placement });
        if layer.task.is_some() {
            // Flushed by the stream task once it is open (or right away if it
            // already is); a still-opening stream sends it on open.
            layer.shared.wake.notify_one();
        } else {
            let token = self.inner.next_token.fetch_add(1, Ordering::SeqCst);
            let handle = self.inner.runtime.spawn(run_stream(
                self.inner.clone(),
                layer_id.to_string(),
                token,
                layer.z,
                layer.shared.clone(),
            ));
            layer.task = Some(StreamTask { token, handle });
        }
    }

    pub fn clear(&self, layer_id: &str) {
        let layer = self.inner.layers().remove(layer_id);
        let Some(layer) = layer else { return };
        *layer
            .shared
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        // Closing the connection makes herdr drop this layer. This must happen
        // even mid-handshake: a connection that outlives its layer keeps a
        // ghost layer in herdr and keeps this process alive after the tray quits.
        if let Some(task) = layer.task {
            task.handle.abort();
        }
    }

    pub fn clear_all(&self) {
        let ids: Vec<String> = self.inner.layers().keys().cloned().collect();
        for layer_id in ids {
            self.clear(&layer_id);
        }
    }

    pub fn dispose(&self) {
        if self.inner.disposed.swap(true, Ordering::SeqCst) {
            return;
        }
        let layers: Vec<Layer> = self
            .inner
            .layers()
            .drain()
            .map(|(_, layer)| layer)
            .collect();
        for layer in layers {
            if let Some(task) = layer.task {
                task.handle.abort();
            }
        }
    }
}

impl Drop for HerdrSink {
    fn drop(&mut self) {
        // The process-exit guarantee of TS comes from the event loop; here a
        // dropped sink must not leave streams (and ghost layers) behind.
        self.dispose();
    }
}

async fn run_stream(
    inner: Arc<Inner>,
    layer_id: String,
    token: u64,
    z: i32,
    shared: Arc<LayerShared>,
) {
    let reason = stream_loop(&inner, &layer_id, z, &shared).await;
    // A socket that clear() already detached says nothing about the layer.
    let dropped = {
        let mut layers = inner.layers();
        match layers.get_mut(&layer_id) {
            Some(layer) if layer.task.as_ref().is_some_and(|task| task.token == token) => {
                layer.task = None;
                true
            }
            _ => false,
        }
    };
    // Unexpected drop (not from clear/dispose): let callers re-send everything.
    if dropped && !inner.disposed.load(Ordering::SeqCst) {
        inner.generation.fetch_add(1, Ordering::SeqCst);
        (inner.on_error)(anyhow::anyhow!("herdr stream {layer_id} {reason}"));
    }
}

/// Runs one layer stream until the connection ends; returns why it ended.
async fn stream_loop(inner: &Inner, layer_id: &str, z: i32, shared: &LayerShared) -> &'static str {
    let Ok(client) = UnixStream::connect(&inner.address.socket).await else {
        return "error";
    };
    let (mut reader, mut writer) = client.into_split();
    let open = request_line(
        "pane.graphics.stream",
        json!({ "pane_id": inner.address.pane, "layer_id": layer_id, "z_index": z }),
    );
    if writer.write_all(&open).await.is_err() {
        return "error";
    }
    let mut text: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut opened = false;
    let grace = sleep(OPEN_GRACE);
    tokio::pin!(grace);
    loop {
        let mut mark_open = false;
        tokio::select! {
            read = reader.read(&mut chunk) => {
                match read {
                    Ok(0) => return "closed",
                    Err(_) => return "error",
                    Ok(read) => text.extend_from_slice(&chunk[..read]),
                }
                while let Some(end) = text.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = text.drain(..=end).take(end).collect();
                    let Ok(value) = serde_json::from_slice::<Value>(&line) else { continue };
                    let reply = Reply::from_value(&value);
                    if let Some(error) = reply.error {
                        (inner.on_error)(anyhow::anyhow!("herdr stream {layer_id}: {}", error.code()));
                        continue;
                    }
                    mark_open = true;
                }
            }
            // If the open reply never comes, assume success after a short grace.
            () = &mut grace, if !opened => mark_open = true,
            () = shared.wake.notified(), if opened => {
                if flush(&mut writer, shared, inner).await.is_err() {
                    return "error";
                }
            }
        }
        if mark_open && !opened {
            opened = true;
            if flush(&mut writer, shared, inner).await.is_err() {
                return "error";
            }
        }
    }
}

/// Send the newest pending frame. A write that has to wait on a slow herdr
/// (backpressure) holds here, so frames set meanwhile collapse into the newest.
async fn flush(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    shared: &LayerShared,
    inner: &Inner,
) -> Result<(), ()> {
    let frame = shared
        .pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    let Some(frame) = frame else { return Ok(()) };
    let Some((width, height)) = png_size(&frame.png) else {
        (inner.on_error)(anyhow::anyhow!("png too short to read dimensions"));
        return Ok(());
    };
    let header = json!({
        "format": "png",
        "image_width": width,
        "image_height": height,
        "data_length": frame.png.len(),
        "placement": {
            "viewport_col": frame.placement.col,
            "viewport_row": frame.placement.row,
            "grid_cols": frame.placement.cols,
            "grid_rows": frame.placement.rows,
        },
    });
    let mut payload = serde_json::to_vec(&header).expect("header serializes");
    payload.push(b'\n');
    payload.extend_from_slice(&frame.png);
    writer.write_all(&payload).await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tokio::net::UnixListener;

    const ONE_PX_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    fn one_px_png() -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(ONE_PX_PNG_B64)
            .unwrap()
    }

    #[derive(Debug)]
    struct SeenFrame {
        layer: String,
        placement: Value,
        bytes: usize,
    }

    #[derive(Default)]
    struct Recorded {
        opened_layers: Vec<String>,
        frames: Vec<SeenFrame>,
        closed_layers: Vec<String>,
        live_connections: i64,
        info_count: usize,
    }

    type InfoPlan = Box<dyn Fn(usize) -> Value + Send + Sync>;

    /// A fake herdr that answers info per a plan and records stream layers/frames.
    struct FakeHerdr {
        _dir: tempfile::TempDir,
        socket_path: PathBuf,
        recorded: Arc<Mutex<Recorded>>,
        accept: JoinHandle<()>,
    }

    impl FakeHerdr {
        async fn start(info: Option<InfoPlan>, ack_stream_open: bool) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let socket_path = dir.path().join("api.sock");
            let listener = UnixListener::bind(&socket_path).unwrap();
            let recorded = Arc::new(Mutex::new(Recorded::default()));
            let info: Option<Arc<InfoPlan>> = info.map(Arc::new);
            let shared = recorded.clone();
            let accept = tokio::spawn(async move {
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        return;
                    };
                    shared.lock().unwrap().live_connections += 1;
                    tokio::spawn(serve(socket, shared.clone(), info.clone(), ack_stream_open));
                }
            });
            Self {
                _dir: dir,
                socket_path,
                recorded,
                accept,
            }
        }

        fn address(&self) -> HerdrAddress {
            HerdrAddress {
                socket: self.socket_path.to_string_lossy().into_owned(),
                pane: "w1:p2".to_string(),
            }
        }

        fn live_connections(&self) -> i64 {
            self.recorded.lock().unwrap().live_connections
        }

        fn opened_layers(&self) -> Vec<String> {
            self.recorded.lock().unwrap().opened_layers.clone()
        }

        fn closed_layers(&self) -> Vec<String> {
            self.recorded.lock().unwrap().closed_layers.clone()
        }

        fn close(&self) {
            self.accept.abort();
        }
    }

    async fn serve(
        mut socket: UnixStream,
        recorded: Arc<Mutex<Recorded>>,
        info: Option<Arc<InfoPlan>>,
        ack_stream_open: bool,
    ) {
        let mut layer_id: Option<String> = None;
        let mut buf: Vec<u8> = Vec::new();
        let mut expect_bytes = 0usize;
        let mut pending_placement: Option<Value> = None;
        let mut chunk = [0u8; 8192];
        'conn: while let Ok(read) = socket.read(&mut chunk).await {
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
            loop {
                if expect_bytes > 0 {
                    if buf.len() < expect_bytes {
                        continue 'conn;
                    }
                    buf.drain(..expect_bytes);
                    recorded.lock().unwrap().frames.push(SeenFrame {
                        layer: layer_id.clone().unwrap_or_else(|| "?".to_string()),
                        placement: pending_placement.take().unwrap_or(json!({})),
                        bytes: expect_bytes,
                    });
                    expect_bytes = 0;
                    continue;
                }
                let Some(newline) = buf.iter().position(|byte| *byte == 0x0a) else {
                    continue 'conn;
                };
                let line: Vec<u8> = buf.drain(..=newline).take(newline).collect();
                let Ok(message) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                let method = message.get("method").and_then(Value::as_str);
                if method == Some("pane.graphics.info") {
                    let count = {
                        let mut recorded = recorded.lock().unwrap();
                        recorded.info_count += 1;
                        recorded.info_count
                    };
                    let reply = match &info {
                        Some(plan) => plan(count),
                        None => {
                            json!({ "result": { "type": "pane_graphics_info", "cell_width_px": 8, "cell_height_px": 16, "max_layers_per_pane": 16 } })
                        }
                    };
                    let _ = socket.write_all(format!("{reply}\n").as_bytes()).await;
                } else if method == Some("pane.graphics.stream") {
                    let id = message["params"]["layer_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    recorded.lock().unwrap().opened_layers.push(id.clone());
                    layer_id = Some(id);
                    if ack_stream_open {
                        let _ = socket.write_all(b"{\"result\":{\"type\":\"ok\"}}\n").await;
                    }
                } else if message.get("format").is_some_and(Value::is_string)
                    && message.get("data_length").is_some_and(Value::is_number)
                {
                    // A stream frame header; the raw bytes follow.
                    expect_bytes = message["data_length"].as_u64().unwrap() as usize;
                    pending_placement = Some(message["placement"].clone());
                }
            }
        }
        let mut recorded = recorded.lock().unwrap();
        recorded.live_connections -= 1;
        if let Some(id) = layer_id {
            recorded.closed_layers.push(id);
        }
    }

    async fn flush_wait() {
        sleep(Duration::from_millis(120)).await;
    }

    /// Long enough for the sink's 400 ms silent-ack grace to elapse.
    async fn settle() {
        sleep(Duration::from_millis(600)).await;
    }

    fn placement(col: u32, row: u32, cols: u32, rows: u32) -> HerdrPlacement {
        HerdrPlacement {
            col,
            row,
            cols,
            rows,
            z: 0,
        }
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn requires_herdr_env_and_the_pane_context() {
        assert_eq!(herdr_address(env_of(&[])), None);
        assert_eq!(herdr_address(env_of(&[("HERDR_ENV", "1")])), None);
        assert_eq!(
            herdr_address(env_of(&[
                ("HERDR_ENV", "1"),
                ("HERDR_SOCKET_PATH", "/s"),
                ("HERDR_PANE_ID", "w1:p2")
            ])),
            Some(HerdrAddress {
                socket: "/s".to_string(),
                pane: "w1:p2".to_string()
            })
        );
    }

    #[tokio::test]
    async fn retries_while_the_cell_size_is_negotiating_then_returns_it() {
        let plan: InfoPlan = Box::new(|count| {
            if count < 3 {
                json!({ "error": { "code": "cell_size_unavailable", "message": "negotiating" } })
            } else {
                json!({ "result": { "type": "pane_graphics_info", "cell_width_px": 8, "cell_height_px": 16 } })
            }
        });
        let server = FakeHerdr::start(Some(plan), true).await;
        let options = DiscoverOptions {
            retry_ms: Some(5),
            timeout_ms: Some(2000),
            on_waiting: None,
        };
        let result = discover_herdr(&server.address(), options).await;
        assert!(result.ok);
        assert_eq!(result.cell_width, Some(8));
        assert_eq!(result.cell_height, Some(16));
        server.close();
    }

    #[tokio::test]
    async fn reports_an_actionable_reason_when_the_feature_is_disabled() {
        let plan: InfoPlan =
            Box::new(|_| json!({ "error": { "code": "feature_disabled", "message": "off" } }));
        let server = FakeHerdr::start(Some(plan), true).await;
        let options = DiscoverOptions {
            timeout_ms: Some(200),
            ..Default::default()
        };
        let result = discover_herdr(&server.address(), options).await;
        assert!(!result.ok);
        assert!(result.reason.unwrap().contains("kitty_graphics"));
        server.close();
    }

    #[tokio::test]
    async fn reports_the_reattach_hint_when_the_cell_size_never_arrives() {
        let plan: InfoPlan = Box::new(
            |_| json!({ "error": { "code": "cell_size_unavailable", "message": "no size" } }),
        );
        let server = FakeHerdr::start(Some(plan), true).await;
        let options = DiscoverOptions {
            retry_ms: Some(5),
            timeout_ms: Some(60),
            on_waiting: None,
        };
        let result = discover_herdr(&server.address(), options).await;
        assert!(!result.ok);
        assert!(result.reason.unwrap().to_lowercase().contains("reattach"));
        server.close();
    }

    #[tokio::test]
    async fn accepts_a_stream_that_opens_silent_ack_included() {
        let server = FakeHerdr::start(None, false).await;
        let result = probe_herdr_set(&server.address()).await;
        assert!(result.ok);
        server.close();
    }

    #[tokio::test]
    async fn opens_one_stream_per_layer_and_pushes_a_raw_frame_with_the_placement() {
        let server = FakeHerdr::start(None, true).await;
        let png = one_px_png();
        let sink = HerdrSink::new(server.address(), |_| {});
        sink.set("astro-1", png.clone(), 1, 1, placement(4, 7, 10, 5));
        flush_wait().await;
        assert!(server.opened_layers().contains(&"astro-1".to_string()));
        {
            let recorded = server.recorded.lock().unwrap();
            let frame = recorded
                .frames
                .iter()
                .find(|frame| frame.layer == "astro-1")
                .expect("frame arrived");
            assert_eq!(frame.bytes, png.len());
            assert_eq!(
                frame.placement,
                json!({ "viewport_col": 4, "viewport_row": 7, "grid_cols": 10, "grid_rows": 5 })
            );
        }
        sink.dispose();
        flush_wait().await;
        server.close();
    }

    #[tokio::test]
    async fn removes_a_layer_by_closing_its_stream_herdr_drops_it() {
        let server = FakeHerdr::start(None, true).await;
        let sink = HerdrSink::new(server.address(), |_| {});
        sink.set("astro-2", one_px_png(), 1, 1, placement(0, 0, 2, 1));
        flush_wait().await;
        sink.clear("astro-2");
        flush_wait().await;
        assert!(server.closed_layers().contains(&"astro-2".to_string()));
        server.close();
    }

    #[tokio::test]
    async fn closes_every_stream_on_dispose() {
        let server = FakeHerdr::start(None, true).await;
        let sink = HerdrSink::new(server.address(), |_| {});
        sink.set("astro-3", one_px_png(), 1, 1, placement(0, 0, 2, 1));
        sink.set("astro-4", one_px_png(), 1, 1, placement(0, 4, 2, 1));
        flush_wait().await;
        sink.dispose();
        flush_wait().await;
        let mut closed = server.closed_layers();
        closed.sort();
        assert_eq!(closed, vec!["astro-3".to_string(), "astro-4".to_string()]);
        server.close();
    }

    // A stream is a live socket from the moment it starts connecting. Clearing
    // or disposing before herdr acks the open must still close it: an orphaned
    // connection keeps the tray process alive after `q` and leaves a ghost layer.
    #[tokio::test]
    async fn closes_a_stream_that_is_cleared_while_still_opening_silent_ack() {
        let server = FakeHerdr::start(None, false).await;
        let sink = HerdrSink::new(server.address(), |_| {});
        sink.set("astro-5", one_px_png(), 1, 1, placement(0, 0, 2, 1));
        sink.clear("astro-5");
        settle().await;
        assert_eq!(server.live_connections(), 0);
        sink.dispose();
        server.close();
    }

    #[tokio::test]
    async fn closes_a_stream_that_is_cleared_before_the_open_ack_arrives() {
        let server = FakeHerdr::start(None, true).await;
        let sink = HerdrSink::new(server.address(), |_| {});
        sink.set("astro-6", one_px_png(), 1, 1, placement(0, 0, 2, 1));
        tokio::task::yield_now().await;
        sink.clear("astro-6");
        settle().await;
        assert_eq!(server.live_connections(), 0);
        sink.dispose();
        server.close();
    }

    #[tokio::test]
    async fn closes_streams_that_are_still_opening_on_dispose() {
        let server = FakeHerdr::start(None, false).await;
        let sink = HerdrSink::new(server.address(), |_| {});
        sink.set("astro-7", one_px_png(), 1, 1, placement(0, 0, 2, 1));
        sink.set("astro-8", one_px_png(), 1, 1, placement(0, 4, 2, 1));
        sink.dispose();
        settle().await;
        assert_eq!(server.live_connections(), 0);
        server.close();
    }

    #[tokio::test]
    async fn does_not_resurrect_a_cleared_layer_when_the_open_grace_period_elapses() {
        let server = FakeHerdr::start(None, false).await;
        let sink = HerdrSink::new(server.address(), |_| {});
        sink.set("astro-9", one_px_png(), 1, 1, placement(0, 0, 2, 1));
        flush_wait().await;
        sink.clear("astro-9");
        settle().await;
        assert_eq!(server.live_connections(), 0);
        assert!(
            server
                .recorded
                .lock()
                .unwrap()
                .frames
                .iter()
                .all(|frame| frame.layer != "astro-9")
        );
        sink.dispose();
        server.close();
    }
}
