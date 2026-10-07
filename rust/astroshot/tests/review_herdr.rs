//! Port of `packages/astroshot-review/src/herdr.e2e.test.ts`.
//!
//! Drives the built `astroshot review` against a fake herdr socket and proves
//! it renders through herdr's pane-graphics API: transport chosen from the
//! `HERDR_*` environment, one `pane.graphics.info` handshake, then one
//! `pane.graphics.stream` connection per layer carrying real PNG bytes at
//! 0-based pane-cell placements, and `q` ending the process while streams are
//! still opening.
//!
//! The TS test paces itself with sleeps (80 ms between the arrow key and `q`).
//! Here every wait polls what the fake herdr has recorded, or the child's
//! exit, against a deadline.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_astroshot");
const PROBE_LAYER: &str = "astroshot-review-probe";
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// "Ack a beat later, like a busy herdr."
const ACK_DELAY: Duration = Duration::from_millis(250);

fn demo_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/astroshot/fixtures/demo")
        .join(name)
}

/// The TS `beforeEach`: one feature with two stills.
fn seed(dir: &Path) -> PathBuf {
    let root = dir.join("root");
    let feature = root.join("demo-app/.astroshot/checkout");
    fs::create_dir_all(&feature).unwrap();
    fs::copy(
        demo_fixture("welcome.png"),
        feature.join("0001-welcome.png"),
    )
    .unwrap();
    fs::copy(
        demo_fixture("next-steps.png"),
        feature.join("0002-next-steps.png"),
    )
    .unwrap();
    let manifest = json!({ "version": 1, "run_id": "r", "status": "pass", "shots": [
        { "id": "0001", "file": "0001-welcome.png", "title": "Welcome" },
        { "id": "0002", "file": "0002-next-steps.png", "title": "Next steps" },
    ] });
    fs::write(feature.join("manifest.json"), manifest.to_string()).unwrap();
    root
}

/// One image the tray sent: the JSON header and what its payload was.
#[derive(Debug, Clone)]
struct Frame {
    layer: String,
    placement: Value,
    width: u64,
    height: u64,
    bytes: usize,
    /// PNG signature plus the IHDR size, read from the payload itself.
    png_size: Option<(u32, u32)>,
}

#[derive(Debug, Default)]
struct Recorded {
    info_requests: usize,
    /// Layer id of every `pane.graphics.stream` request, in arrival order.
    opened: Vec<String>,
    /// Connections (numbered from 1) that asked for a layer other than the
    /// probe and have not been acknowledged.
    opening: Vec<usize>,
    /// Connections the tray has closed.
    closed: Vec<usize>,
    frames: Vec<Frame>,
    connections: usize,
}

/// The TS test's `net.createServer`: answers `pane.graphics.info`, acks each
/// `pane.graphics.stream` after [`ACK_DELAY`], and records every frame.
struct FakeHerdr {
    socket_path: PathBuf,
    recorded: Arc<Mutex<Recorded>>,
    /// While set, stream opens are recorded but never acknowledged.
    hold_acks: Arc<AtomicBool>,
}

impl FakeHerdr {
    fn start(socket_path: PathBuf) -> Self {
        let listener = UnixListener::bind(&socket_path).unwrap();
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let hold_acks = Arc::new(AtomicBool::new(false));
        let (state, hold) = (recorded.clone(), hold_acks.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (state, hold) = (state.clone(), hold.clone());
                std::thread::spawn(move || serve(stream, &state, &hold));
            }
        });
        Self {
            socket_path,
            recorded,
            hold_acks,
        }
    }

    fn read<T>(&self, view: impl FnOnce(&Recorded) -> T) -> T {
        view(&self.recorded.lock().unwrap())
    }

    /// Poll the record until `done` holds; panics with the record otherwise.
    fn wait_for(&self, label: &str, timeout: Duration, done: impl Fn(&Recorded) -> bool) {
        let deadline = Instant::now() + timeout;
        loop {
            if self.read(&done) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "Timed out waiting for {label}. Recorded: {:#?}",
                    self.recorded.lock().unwrap()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// One connection: newline-delimited JSON, with `data_length` raw bytes after
/// each frame header.
fn serve(mut stream: UnixStream, state: &Arc<Mutex<Recorded>>, hold_acks: &Arc<AtomicBool>) {
    let connection = {
        let mut state = state.lock().unwrap();
        state.connections += 1;
        state.connections
    };
    let mut layer: Option<String> = None;
    let mut buffer: Vec<u8> = Vec::new();
    let mut header: Option<Value> = None;
    let mut chunk = [0u8; 65536];
    'connection: loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
        loop {
            if let Some(pending) = &header {
                let expect = pending["data_length"].as_u64().unwrap() as usize;
                if buffer.len() < expect {
                    continue 'connection;
                }
                let payload: Vec<u8> = buffer.drain(..expect).collect();
                let pending = header.take().unwrap();
                if let Some(layer) = layer.as_ref().filter(|layer| *layer != PROBE_LAYER) {
                    let png_size =
                        (payload.len() >= 24 && payload[..8] == PNG_SIGNATURE).then(|| {
                            let be = |at: usize| {
                                u32::from_be_bytes(payload[at..at + 4].try_into().unwrap())
                            };
                            (be(16), be(20))
                        });
                    state.lock().unwrap().frames.push(Frame {
                        layer: layer.clone(),
                        placement: pending.get("placement").cloned().unwrap_or(json!({})),
                        width: pending["image_width"].as_u64().unwrap_or(0),
                        height: pending["image_height"].as_u64().unwrap_or(0),
                        bytes: expect,
                        png_size,
                    });
                }
                continue;
            }
            let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') else {
                continue 'connection;
            };
            let line: Vec<u8> = buffer.drain(..=newline).take(newline).collect();
            let Ok(message) = serde_json::from_slice::<Value>(&line) else {
                continue;
            };
            match message["method"].as_str() {
                Some("pane.graphics.info") => {
                    state.lock().unwrap().info_requests += 1;
                    let reply = json!({ "result": { "type": "pane_graphics_info", "cell_width_px": 9, "cell_height_px": 20, "max_layers_per_pane": 16 } });
                    let _ = stream.write_all(format!("{reply}\n").as_bytes());
                }
                Some("pane.graphics.stream") => {
                    let id: String = message["params"]["layer_id"]
                        .as_str()
                        .unwrap_or("?")
                        .to_string();
                    layer = Some(id.clone());
                    {
                        let mut state = state.lock().unwrap();
                        state.opened.push(id.clone());
                        if id != PROBE_LAYER {
                            state.opening.push(connection);
                        }
                    }
                    if hold_acks.load(Ordering::SeqCst) {
                        continue;
                    }
                    // The server's own reply delay, not a wait in the test.
                    let (state, mut reply_to) = (state.clone(), stream.try_clone().unwrap());
                    std::thread::spawn(move || {
                        std::thread::sleep(ACK_DELAY);
                        let ack = json!({ "result": { "type": "ok" } });
                        if reply_to.write_all(format!("{ack}\n").as_bytes()).is_ok() {
                            state.lock().unwrap().opening.retain(|c| *c != connection);
                        }
                    });
                }
                _ if message["format"].is_string() && message["data_length"].is_number() => {
                    header = Some(message);
                }
                _ => {}
            }
        }
    }
    state.lock().unwrap().closed.push(connection);
}

/// The tray in a 140x40 PTY, pointed at the fake herdr by `HERDR_*`.
struct Tray {
    output: Arc<Mutex<Vec<u8>>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
}

impl Tray {
    fn launch(dir: &Path, root: &Path, herdr: &FakeHerdr) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 140,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(BIN);
        command.args(["review", "--root", &root.to_string_lossy(), "--no-index"]);
        command.cwd(dir);
        command.env("HERDR_ENV", "1");
        command.env("HERDR_SOCKET_PATH", &herdr.socket_path);
        command.env("HERDR_PANE_ID", "w9:p9");
        command.env("ASTROSHOT_REVIEW_CACHE_DIR", dir.join("cache"));
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        // The TS test inherits the caller's environment; drop what would
        // pick another transport or change the cell size.
        for name in [
            "TMUX",
            "MOSH_SERVER_NETWORK_TMOUT",
            "MOSH_CONNECTION",
            "MOSH_KEY",
            "ASTROSHOT_REVIEW_GRAPHICS",
            "ASTROSHOT_REVIEW_DEBUG",
            "ASTROSHOT_REVIEW_CELL_PX",
        ] {
            command.env_remove(name);
        }
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = output.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buffer[..read]);
            }
        });
        Self {
            output,
            writer,
            child,
            _master: pair.master,
        }
    }

    fn write(&mut self, data: &str) {
        self.writer.write_all(data.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    /// The child's exit code, waiting up to `timeout` for it to exit.
    fn wait_exit(&mut self, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status.exit_code());
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Poll the PTY output until it contains `text`.
    fn wait_for_output(&self, text: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.output().contains(text) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn temp_dir() -> tempfile::TempDir {
    // Short prefix: the socket path must fit in `sun_path`.
    tempfile::Builder::new()
        .prefix("herdr-e2e-")
        .tempdir()
        .unwrap()
}

fn placement(frame: &Frame, key: &str) -> i64 {
    frame.placement[key].as_i64().unwrap_or(-1)
}

#[test]
fn streams_real_png_frames_to_per_layer_herdr_streams_at_pane_cell_placements() {
    let dir = temp_dir();
    let root = seed(dir.path());
    let herdr = FakeHerdr::start(dir.path().join("api.sock"));
    let mut tray = Tray::launch(dir.path(), &root, &herdr);

    herdr.wait_for("three frames", Duration::from_secs(15), |recorded| {
        recorded.frames.len() >= 3
    });

    // `q` must end the process, not just the UI: every herdr stream is a live
    // socket, and one left open after quit kept the tray alive in the shell.
    // Move first so fresh preview streams are mid-handshake when we quit.
    let opened_before = herdr.read(|recorded| recorded.opened.len());
    tray.write("\x1b[B");
    herdr.wait_for(
        "a stream opened by the navigation",
        Duration::from_secs(5),
        |recorded| recorded.opened.len() > opened_before,
    );
    tray.write("q");
    let exit_code = tray.wait_exit(Duration::from_secs(5));
    assert_eq!(exit_code, Some(0), "output: {:?}", tray.output());

    // Transport selection: the tray took the alternate screen and drew its
    // pictures through the socket named by HERDR_SOCKET_PATH, after one
    // cell-size handshake and one probe stream.
    assert!(
        tray.wait_for_output("\x1b[?1049h", Duration::from_secs(5)),
        "output: {:?}",
        tray.output()
    );
    let (info_requests, opened, frames) = herdr.read(|recorded| {
        (
            recorded.info_requests,
            recorded.opened.clone(),
            recorded.frames.clone(),
        )
    });
    assert_eq!(info_requests, 1);
    let probes = opened.iter().filter(|layer| *layer == PROBE_LAYER).count();
    assert_eq!(probes, 1, "{opened:?}");
    assert_eq!(opened[0], PROBE_LAYER, "the probe precedes every layer");

    assert!(frames.len() >= 3, "{frames:#?}");
    for frame in &frames {
        assert!(frame.bytes > 100, "{frame:?}");
        assert!(frame.width > 0, "{frame:?}");
        assert!(placement(frame, "viewport_col") >= 0, "{frame:?}");
        assert!(placement(frame, "viewport_row") >= 0, "{frame:?}");
        assert!(placement(frame, "grid_cols") > 0, "{frame:?}");
        assert!(placement(frame, "grid_rows") > 0, "{frame:?}");
        // Real PNG bytes, the size the header announces, inside the pane.
        let (png_width, png_height) = frame.png_size.expect("payload is a PNG");
        assert_eq!(u64::from(png_width), frame.width, "{frame:?}");
        assert_eq!(u64::from(png_height), frame.height, "{frame:?}");
        let right = placement(frame, "viewport_col") + placement(frame, "grid_cols");
        let bottom = placement(frame, "viewport_row") + placement(frame, "grid_rows");
        assert!(right <= 140 && bottom <= 40, "{frame:?}");
    }
    // The detail preview (right pane) is wider than a left-rail thumbnail.
    assert!(
        frames
            .iter()
            .any(|frame| placement(frame, "grid_cols") >= 20),
        "{frames:#?}"
    );
    // Per-layer streams: every connection names one layer, and the frames
    // came over more than one of them. (A layer id is reused once its
    // picture is cleared, so the same id may open again after navigation.)
    let mut drawn: Vec<&String> = frames.iter().map(|frame| &frame.layer).collect();
    drawn.sort();
    drawn.dedup();
    assert!(drawn.len() >= 2, "{drawn:?}");
}

#[test]
fn q_exits_zero_while_herdr_streams_are_still_opening() {
    let dir = temp_dir();
    let root = seed(dir.path());
    let herdr = FakeHerdr::start(dir.path().join("api.sock"));
    let mut tray = Tray::launch(dir.path(), &root, &herdr);
    herdr.wait_for("three frames", Duration::from_secs(15), |recorded| {
        recorded.frames.len() >= 3
    });

    // From here herdr stops acknowledging, so the streams the navigation
    // opens are mid-handshake for as long as the tray keeps them.
    herdr.hold_acks.store(true, Ordering::SeqCst);
    let opened_before = herdr.read(|recorded| recorded.opened.len());
    tray.write("\x1b[B");
    herdr.wait_for(
        "a stream opened by the navigation",
        Duration::from_secs(5),
        |recorded| recorded.opened.len() > opened_before,
    );
    let opening = herdr.read(|recorded| recorded.opening.clone());
    assert!(!opening.is_empty(), "no stream is mid-handshake");

    let asked = Instant::now();
    tray.write("q");
    let exit_code = tray.wait_exit(Duration::from_secs(5));
    assert_eq!(exit_code, Some(0), "output: {:?}", tray.output());
    assert!(asked.elapsed() < Duration::from_secs(5));

    // Quitting closed every socket, including the ones still opening.
    herdr.wait_for(
        "every connection to close",
        Duration::from_secs(5),
        |recorded| recorded.closed.len() == recorded.connections,
    );
    let closed = herdr.read(|recorded| recorded.closed.clone());
    assert!(opening.iter().all(|c| closed.contains(c)), "{closed:?}");
}
