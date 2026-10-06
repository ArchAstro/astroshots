//! Port of `packages/tui-shot/src/pty-shot.ts`: run a real program in a PTY,
//! drive it with keys and waits, and capture the screen as a PNG.
//!
//! Replacements and divergences from TS:
//! - `node-pty` is `portable-pty`. A reader thread feeds the emulator in
//!   order, so TS's `writes` promise chain has no counterpart: once a chunk
//!   has been read it is already in the screen. The exit event is raised only
//!   after the reader has drained (bounded to 500ms, in case a grandchild
//!   keeps the slave open), like node-pty delivering data before `onExit`.
//! - `@xterm/headless` + the HTML screenshot are [`HeadlessTerminal`] and
//!   [`render_png`]. `fontFamily` is validated as a string only (the font is
//!   bundled) and `headed` is ignored (no browser).
//! - A program killed by a signal reports exit code 0, as node-pty does
//!   (`exitCode: 0, signal: n`); portable-pty reports 1.
//! - Kitty graphics: xterm answered device queries and the tracker forwarded
//!   its replies. The alacritty screen answers nothing, so graphics mode
//!   replies to primary DA (`CSI c`), DSR 5 and DSR 6 itself with xterm's
//!   answers (`ESC [ ? 1 ; 2 c`, `ESC [ 0 n`, `ESC [ row ; col R`).
//! - Kill: `portable-pty` sends SIGHUP, waits 200ms, then SIGKILL in one
//!   `kill()`; it is called twice with a 500ms wait after each, as in TS.
//! - Exit wrapper: TS spawns `node pty-exit-wrapper.js`. Here the wrapper is
//!   the current binary re-executed with the hidden subcommand
//!   `__pty-exit-wrapper <token> <status path> <command> [args...]`, which
//!   must call [`super::pty_exit_wrapper::run`] with the arguments after the
//!   subcommand name and exit with its return code. Under `cargo test`
//!   (`cfg(test)`), `current_exe()` is the test binary, so the wrapper is
//!   invoked as that binary re-running the `pty_exit_wrapper_helper` test with
//!   its arguments passed in `ASTROSHOT_TEST_PTY_WRAPPER_ARGS` (JSON).
//! - The wrapper is used on Windows, or when
//!   `ASTROSHOT_TEST_FORCE_PTY_EXIT_WRAPPER=1`, like TS.
//! - Fixture YAML is parsed by `serde_yaml_ng`, so a syntax error carries that
//!   library's wording.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::kitty_graphics::{KittyGraphicsTracker, KittyTrackerOptions, js_number_string};
use super::shot::{ValidPositiveOptions, queue_terminal_shot, valid_positive};
use super::types::{
    AlwaysTrue, PtyAction, PtyGraphics, PtyKey, PtyShotFixture, PtyShotRequest, VersionOne,
};
use crate::raster::{HeadlessTerminal, RasterOptions, render_png};

/// Subcommand the astroshot binary must expose for the exit wrapper.
pub const EXIT_WRAPPER_SUBCOMMAND: &str = "__pty-exit-wrapper";

const KEY_NAMES: [&str; 11] = [
    "enter",
    "up",
    "down",
    "right",
    "left",
    "tab",
    "escape",
    "backspace",
    "space",
    "ctrl-c",
    "ctrl-d",
];

/// `KEYSTROKES[key]`.
pub fn keystroke(key: PtyKey) -> &'static str {
    match key {
        PtyKey::Enter => "\r",
        PtyKey::Up => "\x1b[A",
        PtyKey::Down => "\x1b[B",
        PtyKey::Right => "\x1b[C",
        PtyKey::Left => "\x1b[D",
        PtyKey::Tab => "\t",
        PtyKey::Escape => "\x1b",
        PtyKey::Backspace => "\x7f",
        PtyKey::Space => " ",
        PtyKey::CtrlC => "\x03",
        PtyKey::CtrlD => "\x04",
    }
}

fn key_from_name(name: &str) -> Option<PtyKey> {
    Some(match name {
        "enter" => PtyKey::Enter,
        "up" => PtyKey::Up,
        "down" => PtyKey::Down,
        "right" => PtyKey::Right,
        "left" => PtyKey::Left,
        "tab" => PtyKey::Tab,
        "escape" => PtyKey::Escape,
        "backspace" => PtyKey::Backspace,
        "space" => PtyKey::Space,
        "ctrl-c" => PtyKey::CtrlC,
        "ctrl-d" => PtyKey::CtrlD,
        _ => return None,
    })
}

fn fixture_error(fixture_path: &str, detail: &str) -> anyhow::Error {
    anyhow!("Invalid PTY fixture {fixture_path}: {detail}")
}

/// `path.resolve`: absolute and lexically normalized, no filesystem access.
fn resolve_path(base: &Path, path: &str) -> PathBuf {
    let joined = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        base.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

fn resolve_cwd_relative(path: &str) -> PathBuf {
    resolve_path(&std::env::current_dir().unwrap_or_default(), path)
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// First `count` UTF-16 code units, like `String#slice(0, count)`.
fn slice_utf16(text: &str, count: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(count).collect();
    String::from_utf16_lossy(&units)
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("string serializes")
}

fn validate_action(value: &Value, fixture_path: &str, index: usize) -> Result<PtyAction> {
    let Some(record) = value.as_object() else {
        return Err(fixture_error(
            fixture_path,
            &format!("actions[{index}] must be an object"),
        ));
    };
    let operators: Vec<&str> = ["waitFor", "waitForExit", "key", "text", "pauseMs"]
        .into_iter()
        .filter(|name| record.contains_key(*name))
        .collect();
    if operators.len() != 1 {
        return Err(fixture_error(
            fixture_path,
            &format!(
                "actions[{index}] must set exactly one of waitFor, waitForExit, key, text, or pauseMs"
            ),
        ));
    }
    let validated_timeout = || -> Result<Option<f64>> {
        match record.get("timeoutMs") {
            None => Ok(None),
            Some(Value::Number(number)) => {
                let timeout = number.as_f64().unwrap_or(f64::NAN);
                if timeout.is_finite() && timeout > 0.0 {
                    Ok(Some(timeout))
                } else {
                    Err(fixture_error(
                        fixture_path,
                        &format!("actions[{index}].timeoutMs must be a positive number"),
                    ))
                }
            }
            Some(_) => Err(fixture_error(
                fixture_path,
                &format!("actions[{index}].timeoutMs must be a positive number"),
            )),
        }
    };
    match operators[0] {
        "waitFor" => match record.get("waitFor") {
            Some(Value::String(text)) if !text.is_empty() => Ok(PtyAction::WaitFor {
                wait_for: text.clone(),
                timeout_ms: validated_timeout()?,
            }),
            _ => Err(fixture_error(
                fixture_path,
                &format!("actions[{index}].waitFor must be text"),
            )),
        },
        "waitForExit" => {
            if record.get("waitForExit") != Some(&Value::Bool(true)) {
                return Err(fixture_error(
                    fixture_path,
                    &format!("actions[{index}].waitForExit must be true"),
                ));
            }
            Ok(PtyAction::WaitForExit {
                wait_for_exit: AlwaysTrue,
                timeout_ms: validated_timeout()?,
            })
        }
        "key" => {
            let key = record
                .get("key")
                .and_then(Value::as_str)
                .and_then(|name| key_from_name(&name.to_lowercase()));
            match key {
                Some(key) => Ok(PtyAction::Key { key }),
                None => Err(fixture_error(
                    fixture_path,
                    &format!(
                        "actions[{index}].key must be one of {}",
                        KEY_NAMES.join(", ")
                    ),
                )),
            }
        }
        "text" => match record.get("text") {
            Some(Value::String(text)) => Ok(PtyAction::Text { text: text.clone() }),
            _ => Err(fixture_error(
                fixture_path,
                &format!("actions[{index}].text must be a string"),
            )),
        },
        _ => match record.get("pauseMs") {
            Some(Value::Number(number)) => {
                let pause = number.as_f64().unwrap_or(f64::NAN);
                if pause.is_finite() && pause >= 0.0 {
                    Ok(PtyAction::Pause { pause_ms: pause })
                } else {
                    Err(fixture_error(
                        fixture_path,
                        &format!("actions[{index}].pauseMs must be a non-negative number"),
                    ))
                }
            }
            _ => Err(fixture_error(
                fixture_path,
                &format!("actions[{index}].pauseMs must be a non-negative number"),
            )),
        },
    }
}

fn string_array(
    value: Option<&Value>,
    fixture_path: &str,
    field: &str,
) -> Result<Option<Vec<String>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let strings = value.as_array().and_then(|entries| {
        entries
            .iter()
            .map(|entry| entry.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
    });
    match strings {
        Some(strings) => Ok(Some(strings)),
        None => Err(fixture_error(
            fixture_path,
            &format!("{field} must be an array of strings"),
        )),
    }
}

/// A numeric fixture field. TS leaves these unvalidated and rejects them
/// later with `validPositive` / `milliseconds`; a missing or null field takes
/// the default (`??`) and any non-number fails those checks, so it becomes
/// NaN here.
fn lenient_number(value: Option<&Value>) -> Option<f64> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => Some(number.as_f64().unwrap_or(f64::NAN)),
        Some(_) => Some(f64::NAN),
    }
}

/// Port of `loadPtyFixture`.
pub fn load_pty_fixture(fixture_path: &str) -> Result<PtyShotFixture> {
    let absolute = resolve_cwd_relative(fixture_path);
    let absolute_text = display(&absolute);
    if !absolute.exists() {
        bail!("Fixture not found: {absolute_text}");
    }
    let raw =
        fs::read_to_string(&absolute).with_context(|| format!("could not read {absolute_text}"))?;
    let is_json = absolute
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    let parsed: std::result::Result<Value, String> = if is_json {
        serde_json::from_str(&raw).map_err(|error| error.to_string())
    } else {
        serde_yaml_ng::from_str(&raw).map_err(|error| error.to_string())
    };
    let value = parsed.map_err(|detail| fixture_error(&absolute_text, &detail))?;
    let Some(record) = value.as_object() else {
        return Err(fixture_error(
            &absolute_text,
            "the document must be an object",
        ));
    };
    if record.get("version").and_then(Value::as_f64) != Some(1.0) {
        return Err(fixture_error(&absolute_text, "version must be 1"));
    }
    let command = match record.get("command") {
        Some(Value::String(command)) if !command.trim().is_empty() => command.clone(),
        _ => {
            return Err(fixture_error(
                &absolute_text,
                "command must be a non-empty string",
            ));
        }
    };
    let args = string_array(record.get("args"), &absolute_text, "args")?;
    let expect_text = string_array(record.get("expectText"), &absolute_text, "expectText")?;
    let cwd = match record.get("cwd") {
        None => None,
        Some(Value::String(cwd)) => Some(cwd.clone()),
        Some(_) => return Err(fixture_error(&absolute_text, "cwd must be a string")),
    };
    let allow_non_zero_exit = match record.get("allowNonZeroExit") {
        None => None,
        Some(Value::Bool(allow)) => Some(*allow),
        Some(_) => {
            return Err(fixture_error(
                &absolute_text,
                "allowNonZeroExit must be a boolean",
            ));
        }
    };
    let graphics = match record.get("graphics") {
        None => None,
        Some(Value::String(kind)) if kind == "kitty" => Some(PtyGraphics::Kitty),
        Some(_) => {
            return Err(fixture_error(
                &absolute_text,
                "graphics must be \"kitty\" when set",
            ));
        }
    };
    let mut appearance: HashMap<&str, Option<String>> = HashMap::new();
    for field in ["background", "foreground", "fontFamily"] {
        match record.get(field) {
            None => {
                appearance.insert(field, None);
            }
            Some(Value::String(text)) => {
                appearance.insert(field, Some(text.clone()));
            }
            Some(_) => {
                return Err(fixture_error(
                    &absolute_text,
                    &format!("{field} must be a string"),
                ));
            }
        }
    }
    let env = match record.get("env") {
        None => None,
        Some(Value::Object(entries)) => {
            let mut env = BTreeMap::new();
            for (name, entry) in entries {
                match entry {
                    Value::String(text) => {
                        env.insert(name.clone(), text.clone());
                    }
                    _ => return Err(fixture_error(&absolute_text, "env values must be strings")),
                }
            }
            Some(env)
        }
        Some(_) => return Err(fixture_error(&absolute_text, "env values must be strings")),
    };
    let actions = match record.get("actions") {
        None => None,
        Some(Value::Array(entries)) => Some(
            entries
                .iter()
                .enumerate()
                .map(|(index, action)| validate_action(action, &absolute_text, index))
                .collect::<Result<Vec<_>>>()?,
        ),
        Some(_) => return Err(fixture_error(&absolute_text, "actions must be an array")),
    };
    Ok(PtyShotFixture {
        version: VersionOne,
        command,
        args,
        cwd,
        env,
        cols: lenient_number(record.get("cols")),
        rows: lenient_number(record.get("rows")),
        timeout_ms: lenient_number(record.get("timeoutMs")),
        settle_ms: lenient_number(record.get("settleMs")),
        allow_non_zero_exit,
        graphics,
        actions,
        expect_text,
        background: appearance["background"].clone(),
        foreground: appearance["foreground"].clone(),
        font_family: appearance["fontFamily"].clone(),
        font_size: lenient_number(record.get("fontSize")),
        line_height: lenient_number(record.get("lineHeight")),
        padding: lenient_number(record.get("padding")),
        border_radius: lenient_number(record.get("borderRadius")),
        scale: lenient_number(record.get("scale")),
    })
}

/// Port of `milliseconds`.
fn milliseconds(value: Option<f64>, fallback: f64, name: &str, maximum: f64) -> Result<f64> {
    let resolved = value.unwrap_or(fallback);
    if !resolved.is_finite() || resolved < 0.0 || resolved > maximum {
        bail!("{name} must be between 0 and {}", js_number_string(maximum));
    }
    Ok(resolved)
}

async fn delay(duration_ms: f64) {
    tokio::time::sleep(Duration::from_secs_f64(duration_ms.max(0.0) / 1000.0)).await;
}

/// Port of `resolvePtyCommand`.
fn resolve_pty_command(command: &str, cwd: &Path, env: &HashMap<String, String>) -> Result<String> {
    let validate_executable = |resolved: String| -> Result<String> {
        if cfg!(windows) {
            let extension = Path::new(&resolved)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| format!(".{}", extension.to_lowercase()));
            if matches!(extension.as_deref(), Some(".bat" | ".cmd")) {
                bail!(
                    "PTY command resolves to a Windows batch script, which requires a shell: {resolved}. Use the underlying .exe executable to keep capture shell-free."
                );
            }
        }
        Ok(resolved)
    };

    if Path::new(command).is_absolute() {
        return validate_executable(command.to_string());
    }
    if command.contains('/') || command.contains('\\') {
        return validate_executable(display(&resolve_path(cwd, command)));
    }
    if !cfg!(windows) {
        return validate_executable(command.to_string());
    }

    let lookup = |wanted: &str| {
        env.iter()
            .find(|(name, _)| name.to_lowercase() == wanted)
            .map(|(_, value)| value.clone())
    };
    let path_value = lookup("path").unwrap_or_default();
    let has_extension = Path::new(command).extension().is_some();
    let extensions: Vec<String> = if has_extension {
        vec![String::new()]
    } else {
        lookup("pathext")
            .unwrap_or_else(|| ".COM;.EXE".to_string())
            .split(';')
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()
    };
    for directory in path_value.split(';') {
        let directory = directory
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(directory);
        if directory.is_empty() {
            continue;
        }
        for extension in &extensions {
            let candidate = Path::new(directory).join(format!("{command}{extension}"));
            if candidate.is_file() {
                return validate_executable(display(&candidate));
            }
        }
    }
    bail!("PTY command was not found on PATH: {command}")
}

/// The emulated terminal: plain, or wrapped by the kitty graphics tracker.
enum Emulator {
    Plain(Box<HeadlessTerminal>),
    Kitty(Box<KittyGraphicsTracker>),
}

impl Emulator {
    fn terminal(&self) -> &HeadlessTerminal {
        match self {
            Self::Plain(terminal) => terminal,
            Self::Kitty(tracker) => tracker.terminal(),
        }
    }

    fn write(&mut self, text: &str) {
        match self {
            Self::Plain(terminal) => terminal.write(text.as_bytes()),
            Self::Kitty(tracker) => tracker.write(text),
        }
    }
}

struct Shared {
    emulator: Emulator,
    /// Bytes of a UTF-8 sequence split across reads.
    utf8_pending: Vec<u8>,
    marker_buffer: String,
    wrapped_exit_code: Option<i64>,
    /// Set when the child has exited and its output has drained.
    exited: Option<i64>,
    reader_done: bool,
}

type SharedState = Arc<Mutex<Shared>>;
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Decode a chunk like node's `StringDecoder`: incomplete trailing sequences
/// wait for the next chunk, invalid bytes become U+FFFD.
fn decode_utf8_chunk(pending: &mut Vec<u8>, data: &[u8]) -> String {
    pending.extend_from_slice(data);
    let mut text = String::new();
    let mut offset = 0;
    loop {
        match std::str::from_utf8(&pending[offset..]) {
            Ok(valid) => {
                text.push_str(valid);
                offset = pending.len();
                break;
            }
            Err(error) => {
                let valid_end = offset + error.valid_up_to();
                text.push_str(
                    std::str::from_utf8(&pending[offset..valid_end]).expect("validated prefix"),
                );
                match error.error_len() {
                    Some(length) => {
                        text.push('\u{fffd}');
                        offset = valid_end + length;
                    }
                    None => {
                        offset = valid_end;
                        break;
                    }
                }
            }
        }
    }
    pending.drain(..offset);
    text
}

/// Answers xterm gives to the device queries a kitty-aware program sends.
fn terminal_replies(text: &str, cursor: (u16, u16)) -> String {
    let mut replies = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("\x1b[") {
        let after = &rest[start + 2..];
        let end = after
            .find(|character: char| !matches!(character, '0'..='9' | ';'))
            .unwrap_or(after.len());
        let params = &after[..end];
        match (after[end..].chars().next(), params) {
            (Some('c'), "" | "0") => replies.push_str("\x1b[?1;2c"),
            (Some('n'), "5") => replies.push_str("\x1b[0n"),
            (Some('n'), "6") => {
                replies.push_str(&format!("\x1b[{};{}R", cursor.1 + 1, cursor.0 + 1));
            }
            _ => {}
        }
        rest = &after[end..];
    }
    replies
}

struct ReaderContext {
    shared: SharedState,
    writer: SharedWriter,
    exited_flag: Arc<AtomicBool>,
    marker_prefix: Option<String>,
    graphics: bool,
}

/// `child.write`: errors (a closed PTY) are ignored.
fn write_raw(writer: &SharedWriter, data: &str) {
    let mut writer = lock(writer);
    let _ = writer.write_all(data.as_bytes());
    let _ = writer.flush();
}

/// A reply to the program: dropped once the program has exited.
fn write_to_pty(writer: &SharedWriter, exited_flag: &AtomicBool, data: &str) {
    if !exited_flag.load(Ordering::SeqCst) {
        write_raw(writer, data);
    }
}

fn process_chunk(context: &ReaderContext, data: &[u8]) {
    let replies = {
        let mut shared = lock(&context.shared);
        let text = decode_utf8_chunk(&mut shared.utf8_pending, data);
        if let Some(prefix) = &context.marker_prefix
            && shared.wrapped_exit_code.is_none()
        {
            let mut buffer = std::mem::take(&mut shared.marker_buffer);
            buffer.push_str(&text);
            let characters = buffer.chars().count();
            if characters > 4_096 {
                buffer = buffer.chars().skip(characters - 4_096).collect();
            }
            if let Some(start) = buffer.rfind(prefix.as_str()) {
                let value_start = start + prefix.len();
                if let Some(end) = buffer[value_start..].find('\x07') {
                    let value = &buffer[value_start..value_start + end];
                    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
                        shared.wrapped_exit_code = Some(value.parse().unwrap_or(i64::MAX));
                    }
                }
            }
            shared.marker_buffer = buffer;
        }
        shared.emulator.write(&text);
        context
            .graphics
            .then(|| terminal_replies(&text, shared.emulator.terminal().cursor_position()))
    };
    if let Some(replies) = replies.filter(|replies| !replies.is_empty()) {
        write_to_pty(&context.writer, &context.exited_flag, &replies);
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, context: ReaderContext) {
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => process_chunk(&context, &buffer[..count]),
        }
    }
    lock(&context.shared).reader_done = true;
}

type SharedChild = Arc<Mutex<Box<dyn Child + Send + Sync>>>;

/// Waits for the child, lets the reader drain, then publishes the exit.
fn wait_loop(child: SharedChild, shared: SharedState, exited_flag: Arc<AtomicBool>) {
    let status = loop {
        let polled = lock(&child).try_wait();
        match polled {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break None,
        }
    };
    let code = status.map_or(1, |status| {
        // node-pty reports `exitCode: 0` for a signal death.
        if status.signal().is_some() {
            0
        } else {
            i64::from(status.exit_code())
        }
    });
    let drain_deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < drain_deadline && !lock(&shared).reader_done {
        std::thread::sleep(Duration::from_millis(2));
    }
    lock(&shared).exited = Some(code);
    exited_flag.store(true, Ordering::SeqCst);
}

/// Program, arguments and extra environment that launch the exit wrapper.
struct WrapperInvocation {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

/// How to launch the exit wrapper around `command`.
fn wrapper_invocation(
    token: &str,
    status_path: &str,
    command: &str,
    args: &[String],
) -> Result<WrapperInvocation> {
    let executable =
        display(&std::env::current_exe().context("could not locate the current executable")?);
    let mut wrapper_args = vec![
        token.to_string(),
        status_path.to_string(),
        command.to_string(),
    ];
    wrapper_args.extend(args.iter().cloned());
    #[cfg(test)]
    {
        let json = serde_json::to_string(&wrapper_args).expect("strings serialize");
        Ok(WrapperInvocation {
            program: executable,
            args: vec![
                "--exact".to_string(),
                "tui_shot::pty_shot::tests::pty_exit_wrapper_helper".to_string(),
                "--nocapture".to_string(),
                "--test-threads=1".to_string(),
            ],
            env: vec![("ASTROSHOT_TEST_PTY_WRAPPER_ARGS".to_string(), json)],
        })
    }
    #[cfg(not(test))]
    {
        let mut args = vec![EXIT_WRAPPER_SUBCOMMAND.to_string()];
        args.extend(wrapper_args);
        Ok(WrapperInvocation {
            program: executable,
            args,
            env: Vec::new(),
        })
    }
}

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 16 random-looking bytes as hex (`randomBytes(16).toString("hex")`).
fn random_token() -> String {
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(TOKEN_COUNTER.fetch_add(1, Ordering::SeqCst).to_le_bytes());
    hasher.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
            .to_le_bytes(),
    );
    hasher.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Everything a running PTY needs cleaned up, even if the shot future is
/// dropped before `shutdown`.
struct PtySession {
    child: SharedChild,
    shared: SharedState,
    exited_flag: Arc<AtomicBool>,
    status_directory: Option<PathBuf>,
    shut_down: bool,
}

impl PtySession {
    fn kill(&self) {
        let _ = lock(&self.child).kill();
    }

    /// The `finally` block: terminate a program that is still running.
    async fn shutdown(&mut self) {
        self.shut_down = true;
        if !self.exited_flag.load(Ordering::SeqCst) {
            self.kill();
            self.wait_for_exit(500).await;
            if !self.exited_flag.load(Ordering::SeqCst) {
                self.kill();
                self.wait_for_exit(500).await;
            }
        }
        self.remove_status_directory();
    }

    async fn wait_for_exit(&self, timeout_ms: u64) {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline && !self.exited_flag.load(Ordering::SeqCst) {
            delay(5.0).await;
        }
    }

    fn remove_status_directory(&mut self) {
        if let Some(directory) = self.status_directory.take() {
            let _ = fs::remove_dir_all(directory);
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if !self.shut_down && !self.exited_flag.load(Ordering::SeqCst) {
            self.kill();
        }
        self.remove_status_directory();
    }
}

/// Result of a capture: the written PNG path and the plain-text frame.
#[derive(Clone, Debug)]
pub struct PtyCapture {
    pub out_path: String,
    pub plain_text: String,
}

/// Port of `takePtyShot`. Returns the absolute output path.
pub fn take_pty_shot(request: &PtyShotRequest) -> impl Future<Output = Result<String>> {
    let request = request.clone();
    queue_terminal_shot(move || async move {
        let force_wrapper =
            std::env::var("ASTROSHOT_TEST_FORCE_PTY_EXIT_WRAPPER").is_ok_and(|value| value == "1");
        take_isolated_pty_shot(&request, force_wrapper)
            .await
            .map(|capture| capture.out_path)
    })
}

/// Like [`take_pty_shot`] but with the exit wrapper forced on or off and the
/// plain-text frame returned. Not queued; callers that need serialization use
/// [`take_pty_shot`].
pub async fn take_isolated_pty_shot(
    request: &PtyShotRequest,
    force_exit_wrapper: bool,
) -> Result<PtyCapture> {
    let absolute_fixture_path = resolve_cwd_relative(&request.fixture_path);
    let fixture = load_pty_fixture(&display(&absolute_fixture_path))?;
    let integer = |maximum: f64| ValidPositiveOptions {
        integer: true,
        maximum,
    };
    let real = |maximum: f64| ValidPositiveOptions {
        integer: false,
        maximum,
    };
    let cols = valid_positive(
        request.cols.or(fixture.cols).unwrap_or(100.0),
        "cols",
        integer(1_000.0),
    )?;
    let rows = valid_positive(
        request.rows.or(fixture.rows).unwrap_or(30.0),
        "rows",
        integer(1_000.0),
    )?;
    let scale = valid_positive(
        request.scale.or(fixture.scale).unwrap_or(2.0),
        "scale",
        real(4.0),
    )?;
    let timeout_ms = milliseconds(fixture.timeout_ms, 15_000.0, "timeoutMs", 120_000.0)?;
    let settle_ms = milliseconds(fixture.settle_ms, 100.0, "settleMs", 10_000.0)?;
    let background = fixture
        .background
        .clone()
        .unwrap_or_else(|| "#090a12".to_string());
    let foreground = fixture
        .foreground
        .clone()
        .unwrap_or_else(|| "#e8e8f2".to_string());
    let font_size = valid_positive(fixture.font_size.unwrap_or(15.0), "fontSize", real(200.0))?;
    let line_height = valid_positive(
        fixture.line_height.unwrap_or(1.32),
        "lineHeight",
        real(10.0),
    )?;
    let padding = milliseconds(fixture.padding, 22.0, "padding", 1_000.0)?;
    let border_radius = milliseconds(fixture.border_radius, 12.0, "borderRadius", 1_000.0)?;
    let fixture_directory = absolute_fixture_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let cwd = resolve_path(&fixture_directory, fixture.cwd.as_deref().unwrap_or("."));
    if !cwd.is_dir() {
        bail!("PTY fixture cwd is not a directory: {}", display(&cwd));
    }
    let (cols_u16, rows_u16) = (cols as u16, rows as u16);

    let graphics = fixture.graphics == Some(PtyGraphics::Kitty);
    let mut child_environment: HashMap<String, String> = std::env::vars_os()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect();
    child_environment.insert(
        "TERM".to_string(),
        if graphics {
            "xterm-kitty"
        } else {
            "xterm-256color"
        }
        .to_string(),
    );
    child_environment.insert("COLORTERM".to_string(), "truecolor".to_string());
    child_environment.extend(fixture.env.clone().unwrap_or_default());

    // Cell metrics the emulated terminal reports; overlays are positioned in
    // the same ratios in the PNG. `Math.round` rounds halves up.
    let cell_width = ((font_size * 0.62 + 0.5).floor() as u32).max(1);
    let cell_height = ((font_size * line_height + 0.5).floor() as u32).max(1);
    let command = resolve_pty_command(&fixture.command, &cwd, &child_environment)?;
    let use_exit_wrapper = cfg!(windows) || force_exit_wrapper;
    let exit_marker_token = if use_exit_wrapper {
        random_token()
    } else {
        String::new()
    };
    let marker_prefix =
        use_exit_wrapper.then(|| format!("\x1b]777;astroshot-exit-{exit_marker_token}="));
    let status_directory = if use_exit_wrapper {
        let directory = std::env::temp_dir().join(format!("astroshot-pty-exit-{}", random_token()));
        fs::create_dir_all(&directory)
            .with_context(|| format!("could not create {}", directory.display()))?;
        Some(directory)
    } else {
        None
    };
    let status_path = status_directory
        .as_ref()
        .map(|directory| display(&directory.join("status")));
    let fixture_args = fixture.args.clone().unwrap_or_default();

    let deadline = Instant::now() + Duration::from_secs_f64(timeout_ms / 1000.0);
    let remaining = move || {
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as f64
    };

    let terminal = HeadlessTerminal::new(cols_u16, rows_u16);
    let writer_slot: Arc<Mutex<Option<SharedWriter>>> = Arc::new(Mutex::new(None));
    let exited_flag = Arc::new(AtomicBool::new(false));
    let emulator = if graphics {
        let reply_writer = Arc::clone(&writer_slot);
        let reply_flag = Arc::clone(&exited_flag);
        Emulator::Kitty(Box::new(KittyGraphicsTracker::new(KittyTrackerOptions {
            terminal,
            cols: u32::from(cols_u16),
            rows: u32::from(rows_u16),
            cell_width,
            cell_height,
            reply: Box::new(move |data: &str| {
                let writer = lock(&reply_writer).clone();
                if let Some(writer) = writer {
                    write_to_pty(&writer, &reply_flag, data);
                }
            }),
        })))
    } else {
        Emulator::Plain(Box::new(terminal))
    };
    let shared: SharedState = Arc::new(Mutex::new(Shared {
        emulator,
        utf8_pending: Vec::new(),
        marker_buffer: String::new(),
        wrapped_exit_code: None,
        exited: None,
        reader_done: false,
    }));

    // Everything from here is cleaned up by `PtySession`.
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows_u16,
            cols: cols_u16,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| anyhow!("could not open a PTY: {error}"))?;
    let mut builder;
    if use_exit_wrapper {
        let invocation = wrapper_invocation(
            &exit_marker_token,
            status_path.as_deref().unwrap_or_default(),
            &command,
            &fixture_args,
        )?;
        builder = CommandBuilder::new(invocation.program);
        builder.args(invocation.args);
        for (name, value) in invocation.env {
            builder.env(name, value);
        }
    } else {
        builder = CommandBuilder::new(&command);
        builder.args(&fixture_args);
    }
    for (name, value) in &child_environment {
        builder.env(name, value);
    }
    builder.cwd(&cwd);
    let spawned = pair.slave.spawn_command(builder);
    drop(pair.slave);
    let child = match spawned {
        Ok(child) => child,
        Err(error) => {
            if let Some(directory) = &status_directory {
                let _ = fs::remove_dir_all(directory);
            }
            return Err(anyhow!("{error:#}"));
        }
    };
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| anyhow!("could not read the PTY: {error}"))?;
    let writer: SharedWriter =
        Arc::new(Mutex::new(pair.master.take_writer().map_err(|error| {
            anyhow!("could not write to the PTY: {error}")
        })?));
    *lock(&writer_slot) = Some(Arc::clone(&writer));
    let master: Box<dyn MasterPty + Send> = pair.master;

    let child: SharedChild = Arc::new(Mutex::new(child));
    let mut session = PtySession {
        child: Arc::clone(&child),
        shared: Arc::clone(&shared),
        exited_flag: Arc::clone(&exited_flag),
        status_directory: status_directory.clone(),
        shut_down: false,
    };
    {
        let context = ReaderContext {
            shared: Arc::clone(&shared),
            writer: Arc::clone(&writer),
            exited_flag: Arc::clone(&exited_flag),
            marker_prefix,
            graphics,
        };
        std::thread::spawn(move || read_loop(reader, context));
        let (wait_shared, wait_flag) = (Arc::clone(&shared), Arc::clone(&exited_flag));
        std::thread::spawn(move || wait_loop(child, wait_shared, wait_flag));
    }

    let run = PtyRun {
        shared: &session.shared,
        status_path: status_path.as_deref(),
        use_exit_wrapper,
        timeout_ms,
        deadline,
    };
    let outcome = async {
        for action in fixture.actions.iter().flatten() {
            if remaining() == 0.0 {
                bail!("PTY fixture exceeded timeoutMs {}", js_number_string(timeout_ms));
            }
            match action {
                PtyAction::WaitFor { wait_for, timeout_ms } => {
                    run.wait_for_text(wait_for, *timeout_ms).await?;
                }
                PtyAction::WaitForExit { timeout_ms, .. } => {
                    run.wait_for_program_exit(*timeout_ms).await?;
                }
                PtyAction::Key { key } => write_raw(&writer, keystroke(*key)),
                PtyAction::Text { text } => write_raw(&writer, text),
                PtyAction::Pause { pause_ms } => {
                    let pause = pause_ms.min(remaining());
                    delay(pause).await;
                    if pause < *pause_ms {
                        bail!("PTY fixture exceeded timeoutMs {}", js_number_string(timeout_ms));
                    }
                }
            }
        }
        if settle_ms > remaining() {
            bail!("PTY fixture exceeded timeoutMs {}", js_number_string(timeout_ms));
        }
        delay(settle_ms).await;
        // Re-read after settle: the child may have exited during the settle
        // window (status file or OSC marker arriving just after the last poll).
        run.refresh_wrapped_exit_code()?;
        // If the exit event arrived but the status bridge file is still
        // racing the rename, give the wrapper a short fixed window.
        if use_exit_wrapper && run.exited().is_some() && run.wrapped().is_none() {
            let bridge_deadline = Instant::now() + Duration::from_millis(1_000);
            while Instant::now() <= bridge_deadline && run.wrapped().is_none() {
                delay(20.0).await;
                run.refresh_wrapped_exit_code()?;
            }
        }
        let plain_frame = run.screen();
        let completed = run.exited();
        if use_exit_wrapper && completed.is_some() && run.wrapped().is_none() {
            bail!("The PTY status wrapper exited without reporting the program exit code.");
        }
        let completed_exit_code = run.wrapped().or(completed);
        if let Some(code) = completed_exit_code
            && code != 0
            && fixture.allow_non_zero_exit != Some(true)
        {
            bail!(
                "PTY program exited with code {code} before capture. Set allowNonZeroExit: true only when documenting an intentional failure state. Visible frame:\n{}",
                slice_utf16(&plain_frame, 1_200)
            );
        }
        for expected in fixture.expect_text.iter().flatten() {
            if !plain_frame.contains(expected.as_str()) {
                bail!(
                    "Fixture did not render expected text {}. Visible frame:\n{}",
                    json_string(expected),
                    slice_utf16(&plain_frame, 1_200)
                );
            }
        }
        let out_path = capture_png(
            &run,
            &request.out_path,
            RenderSettings {
                cols: cols_u16,
                rows: rows_u16,
                scale,
                background: &background,
                foreground: &foreground,
                font_size,
                line_height,
                padding,
                border_radius,
            },
        )?;
        Ok(PtyCapture {
            out_path,
            plain_text: plain_frame,
        })
    }
    .await;
    session.shutdown().await;
    drop(master);
    outcome
}

/// Shared handles the action loop reads.
struct PtyRun<'a> {
    shared: &'a SharedState,
    status_path: Option<&'a str>,
    use_exit_wrapper: bool,
    timeout_ms: f64,
    deadline: Instant,
}

impl PtyRun<'_> {
    fn screen(&self) -> String {
        lock(self.shared).emulator.terminal().plain_text()
    }

    fn exited(&self) -> Option<i64> {
        lock(self.shared).exited
    }

    fn wrapped(&self) -> Option<i64> {
        lock(self.shared).wrapped_exit_code
    }

    /// Port of `refreshWrappedExitCode`.
    fn refresh_wrapped_exit_code(&self) -> Result<Option<i64>> {
        let Some(status_path) = self.status_path else {
            return Ok(self.wrapped());
        };
        if let Some(code) = self.wrapped() {
            return Ok(Some(code));
        }
        let value = match fs::read_to_string(status_path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            bail!("The PTY status wrapper reported an invalid exit code.");
        }
        let code = value.parse().unwrap_or(i64::MAX);
        lock(self.shared).wrapped_exit_code = Some(code);
        Ok(Some(code))
    }

    fn until_deadline_ms(&self) -> f64 {
        self.deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as f64
    }

    async fn wait_for_text(&self, text: &str, action_timeout: Option<f64>) -> Result<()> {
        let allowed = milliseconds(
            action_timeout,
            self.timeout_ms,
            "action timeoutMs",
            120_000.0,
        )?;
        let action_deadline = self
            .deadline
            .min(Instant::now() + Duration::from_secs_f64(allowed / 1000.0));
        while Instant::now() <= action_deadline {
            if self.screen().contains(text) {
                return Ok(());
            }
            if self.exited().is_some() {
                break;
            }
            let left = action_deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as f64;
            delay(20f64.min(left.max(1.0))).await;
        }
        let exit_detail = self
            .exited()
            .map(|code| format!(" The program exited with code {code}."))
            .unwrap_or_default();
        bail!(
            "Timed out waiting for {}.{exit_detail} Visible frame:\n{}",
            json_string(text),
            slice_utf16(&self.screen(), 1_200)
        )
    }

    async fn wait_for_program_exit(&self, action_timeout: Option<f64>) -> Result<()> {
        let started = Instant::now();
        let wait_duration = self.until_deadline_ms().min(milliseconds(
            action_timeout,
            self.timeout_ms,
            "action timeoutMs",
            120_000.0,
        )?);
        let action_deadline = started + Duration::from_secs_f64(wait_duration / 1000.0);
        while Instant::now() <= action_deadline {
            let done = if self.use_exit_wrapper {
                self.refresh_wrapped_exit_code()?.is_some()
            } else {
                self.exited().is_some()
            };
            if done {
                return Ok(());
            }
            let left = action_deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as f64;
            delay(20f64.min(left.max(1.0))).await;
        }
        if self.use_exit_wrapper && self.exited().is_some() {
            bail!("The PTY status wrapper exited without reporting the program exit code.");
        }
        bail!(
            "Timed out waiting for the PTY program to exit within {}ms. Visible frame:\n{}",
            js_number_string(wait_duration),
            slice_utf16(&self.screen(), 1_200)
        )
    }
}

struct RenderSettings<'a> {
    cols: u16,
    rows: u16,
    scale: f64,
    background: &'a str,
    foreground: &'a str,
    font_size: f64,
    line_height: f64,
    padding: f64,
    border_radius: f64,
}

/// `captureTerminalHtml` over the live emulator: check the output path,
/// rasterize the screen and any kitty overlays, write the PNG.
fn capture_png(run: &PtyRun<'_>, out_path: &str, settings: RenderSettings<'_>) -> Result<String> {
    let mut options = RasterOptions::new(settings.cols, settings.rows)
        .with_css_colors(settings.foreground, settings.background)?;
    options.font_size = settings.font_size as f32;
    options.line_height = settings.line_height as f32;
    options.padding = settings.padding as f32;
    options.border_radius = settings.border_radius as f32;
    options.scale = settings.scale as f32;

    let frame = {
        let mut shared = lock(run.shared);
        if let Emulator::Kitty(tracker) = &mut shared.emulator {
            for overlay in tracker
                .overlays()
                .context("could not read kitty graphics")?
            {
                options.overlays.push(overlay.to_raster_overlay()?);
            }
        }
        shared
            .emulator
            .terminal()
            .frame(options.foreground_rgb(), options.background_rgb())
    };

    let out = resolve_cwd_relative(out_path);
    let is_png = out
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !is_png {
        bail!("Output must use a .png extension: {}", display(&out));
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let png = render_png(&frame, &options)?;
    fs::write(&out, png).with_context(|| format!("could not write {}", out.display()))?;
    Ok(display(&out))
}

#[cfg(test)]
mod tests;
