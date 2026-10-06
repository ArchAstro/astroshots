//! Port of `packages/tui-shot/src/kitty-graphics.ts`.
//!
//! Kitty graphics protocol support for PTY captures. A headless terminal has
//! no picture layer, so this tracker sits between the child's output and the
//! terminal: it answers the capability queries a graphics-aware program
//! sends, records every transmitted image and placement at the cursor
//! position the terminal reports, strips the APC sequences from the text
//! stream, and finally yields the pictures as overlays for the rasterizer.
//!
//! Divergences from TS:
//! - `write` is synchronous (the TS awaited xterm's async parser).
//! - The tracker owns the [`HeadlessTerminal`]; reach it with
//!   [`KittyGraphicsTracker::terminal`].
//! - [`GraphicsOverlay`] also carries the decoded `png` bytes next to
//!   `data_url`, so callers can build a [`crate::raster::Overlay`].
//! - `overlays()` returns `Err` where the TS threw (corrupt zlib payload).

use std::collections::HashMap;
use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::raster::{HeadlessTerminal, Overlay, RasterError};

const APC_START: &str = "\x1b_G";
const ST: &str = "\x1b\\";

/// A picture visible in the terminal. `col`/`row`/`cols`/`rows` are cells.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicsOverlay {
    pub col: f64,
    pub row: f64,
    pub cols: f64,
    pub rows: f64,
    pub z: f64,
    /// PNG bytes as a data URL.
    pub data_url: String,
    /// The PNG bytes themselves.
    pub png: Vec<u8>,
}

impl GraphicsOverlay {
    /// The rasterizer overlay for this picture.
    pub fn to_raster_overlay(&self) -> Result<Overlay, RasterError> {
        Overlay::from_png(
            self.col as f32,
            self.row as f32,
            self.cols as f32,
            self.rows as f32,
            &self.png,
        )
    }
}

struct StoredImage {
    format: f64,
    width: Option<f64>,
    height: Option<f64>,
    compressed: bool,
    payload: String,
    medium: String,
    /// Lazily produced PNG bytes.
    png: Option<Vec<u8>>,
}

struct StoredPlacement {
    image_id: f64,
    placement_id: f64,
    col: f64,
    row: f64,
    cols: f64,
    rows: f64,
    z: f64,
}

struct PendingTransmit {
    keys: HashMap<String, String>,
    payload: String,
}

/// Where terminal replies (query answers, device attributes) are sent.
pub type ReplyFn = Box<dyn FnMut(&str) + Send>;

pub struct KittyTrackerOptions {
    pub terminal: HeadlessTerminal,
    pub cols: u32,
    pub rows: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub reply: ReplyFn,
}

/// JS `String(number)`.
pub(crate) fn js_number_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let abs = value.abs();
    if value.fract() == 0.0 && abs < 1e21 {
        return format!("{value:.0}");
    }
    if !(1e-6..1e21).contains(&abs) {
        let text = format!("{value:e}");
        return match text.split_once('e') {
            Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
            _ => text,
        };
    }
    format!("{value}")
}

/// JS `Number(string)`.
fn js_number(text: &str) -> f64 {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if text.is_empty() {
        return 0.0;
    }
    let radix = |digits: &str, radix: u32| {
        u64::from_str_radix(digits, radix)
            .map(|value| value as f64)
            .unwrap_or(f64::NAN)
    };
    let lower = text.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("0x") {
        return radix(rest, 16);
    }
    if let Some(rest) = lower.strip_prefix("0o") {
        return radix(rest, 8);
    }
    if let Some(rest) = lower.strip_prefix("0b") {
        return radix(rest, 2);
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust also accepts "inf" / "nan"; JS does not.
    if text
        .chars()
        .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
    {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// JS `Number(keys.x ?? fallback)`.
fn number_key(keys: &HashMap<String, String>, key: &str, fallback: f64) -> f64 {
    keys.get(key).map_or(fallback, |value| js_number(value))
}

fn parse_keys(text: &str) -> HashMap<String, String> {
    let mut keys = HashMap::new();
    for pair in text.split(',') {
        if let Some(equals) = pair.find('=')
            && equals > 0
        {
            keys.insert(pair[..equals].to_string(), pair[equals + 1..].to_string());
        }
    }
    keys
}

/// `Buffer.from(text, "base64")`: both alphabets, invalid characters skipped,
/// stops at the first `=`, padding optional.
fn decode_base64_lenient(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for ch in text.chars() {
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => break,
            _ => continue,
        };
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn png_chunk(kind: &str, data: &[u8]) -> Vec<u8> {
    let mut typed = Vec::with_capacity(4 + data.len());
    typed.extend_from_slice(kind.as_bytes());
    typed.extend_from_slice(data);
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(&typed);
    chunk.extend_from_slice(&crc32(&typed).to_be_bytes());
    chunk
}

/// Encode raw RGB/RGBA pixels as a PNG (filter type 0 on every scanline).
///
/// Like the TS `Buffer.copy`, a short `pixels` buffer leaves the rest of the
/// image zeroed, and a scanline that starts past the end is an error.
pub fn encode_png(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
) -> std::io::Result<Vec<u8>> {
    let too_big = || std::io::Error::other("PNG dimensions are too large");
    let stride = (width as usize)
        .checked_mul(usize::from(channels))
        .ok_or_else(too_big)?;
    let raw_len = (stride + 1)
        .checked_mul(height as usize)
        .ok_or_else(too_big)?;
    let mut raw = vec![0u8; raw_len];
    for y in 0..height as usize {
        let source = y * stride;
        if source > pixels.len() {
            return Err(std::io::Error::other(
                "The value of \"sourceStart\" is out of range.",
            ));
        }
        let end = (source + stride).min(pixels.len());
        let target = y * (stride + 1) + 1;
        raw[target..target + (end - source)].copy_from_slice(&pixels[source..end]);
    }
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw)?;
    let deflated = encoder.finish()?;
    let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    png.extend(png_chunk("IHDR", &header));
    png.extend(png_chunk("IDAT", &deflated));
    png.extend(png_chunk("IEND", &[]));
    Ok(png)
}

/// A `Map` that keeps insertion order; re-setting a key keeps its position.
struct OrderedMap<V> {
    entries: Vec<(String, V)>,
}

impl<V> OrderedMap<V> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn set(&mut self, key: String, value: V) {
        match self
            .entries
            .iter_mut()
            .find(|(existing, _)| *existing == key)
        {
            Some(entry) => entry.1 = value,
            None => self.entries.push((key, value)),
        }
    }

    fn get(&self, key: &str) -> Option<&V> {
        self.entries
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|entry| &entry.1)
    }

    fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    fn delete(&mut self, key: &str) {
        self.entries.retain(|(existing, _)| existing != key);
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

pub struct KittyGraphicsTracker {
    terminal: HeadlessTerminal,
    cols: u32,
    rows: u32,
    cell_width: u32,
    cell_height: u32,
    reply: ReplyFn,
    images: OrderedMap<StoredImage>,
    placements: OrderedMap<StoredPlacement>,
    pending: Option<PendingTransmit>,
    carry: String,
    query_log: Vec<String>,
}

impl KittyGraphicsTracker {
    pub fn new(options: KittyTrackerOptions) -> Self {
        Self {
            terminal: options.terminal,
            cols: options.cols,
            rows: options.rows,
            cell_width: options.cell_width,
            cell_height: options.cell_height,
            reply: options.reply,
            images: OrderedMap::new(),
            placements: OrderedMap::new(),
            pending: None,
            carry: String::new(),
            query_log: Vec::new(),
        }
    }

    /// The terminal the text is written to.
    pub fn terminal(&self) -> &HeadlessTerminal {
        &self.terminal
    }

    pub fn terminal_mut(&mut self) -> &mut HeadlessTerminal {
        &mut self.terminal
    }

    /// Queries the child sent, for assertions.
    pub fn queries(&self) -> &[String] {
        &self.query_log
    }

    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    /// Feed child output: text goes to the terminal, graphics are recorded.
    pub fn write(&mut self, data: &str) {
        let mut buffer = std::mem::take(&mut self.carry) + data;
        while !buffer.is_empty() {
            let Some(start) = buffer.find(APC_START) else {
                self.write_text(&buffer);
                return;
            };
            if start > 0 {
                self.write_text(&buffer[..start]);
            }
            let body_start = start + APC_START.len();
            let Some(relative_end) = buffer[body_start..].find(ST) else {
                // Wait for the rest of the sequence.
                self.carry = buffer[start..].to_string();
                return;
            };
            let end = body_start + relative_end;
            self.handle_command(&buffer[body_start..end]);
            buffer = buffer[end + ST.len()..].to_string();
        }
    }

    fn write_text(&mut self, text: &str) {
        // Answer the size reports a graphics-aware program asks for. The
        // terminal itself replies to device attributes.
        if text.contains("\x1b[16t") {
            let reply = format!("\x1b[6;{};{}t", self.cell_height, self.cell_width);
            (self.reply)(&reply);
        }
        if text.contains("\x1b[14t") {
            let reply = format!(
                "\x1b[4;{};{}t",
                u64::from(self.cell_height) * u64::from(self.rows),
                u64::from(self.cell_width) * u64::from(self.cols)
            );
            (self.reply)(&reply);
        }
        self.terminal.write(text.as_bytes());
    }

    fn handle_command(&mut self, body: &str) {
        let separator = body.find(';');
        let keys = parse_keys(separator.map_or(body, |at| &body[..at]));
        let payload = separator.map_or("", |at| &body[at + 1..]);
        let action = keys.get("a").map_or("t", String::as_str);

        if let Some(pending) = self.pending.as_mut() {
            pending.payload.push_str(payload);
            if keys.get("m").map(String::as_str) != Some("1") {
                let complete = self.pending.take().expect("pending was set");
                self.store_image(&complete.keys, &complete.payload);
                if complete.keys.get("a").map(String::as_str) == Some("T") {
                    self.place(&complete.keys);
                }
            }
            return;
        }

        match action {
            "q" => {
                self.query_log.push(body.to_string());
                let id = keys.get("i").map_or("0", String::as_str);
                let medium = keys.get("t").map(String::as_str);
                let supported = medium == Some("d") || medium == Some("f");
                let reply = format!(
                    "\x1b_Gi={id};{}\x1b\\",
                    if supported {
                        "OK"
                    } else {
                        "EINVAL:unsupported medium"
                    }
                );
                (self.reply)(&reply);
            }
            "t" | "T" => {
                if keys.get("m").map(String::as_str) == Some("1") {
                    self.pending = Some(PendingTransmit {
                        keys,
                        payload: payload.to_string(),
                    });
                    return;
                }
                self.store_image(&keys, payload);
                if action == "T" {
                    self.place(&keys);
                }
            }
            "p" => self.place(&keys),
            "d" => self.delete(&keys),
            _ => {}
        }
    }

    fn store_image(&mut self, keys: &HashMap<String, String>, payload: &str) {
        let id = number_key(keys, "i", 0.0);
        let sized = |key: &str| {
            keys.get(key)
                .filter(|value| !value.is_empty())
                .map(|value| js_number(value))
        };
        self.images.set(
            js_number_string(id),
            StoredImage {
                format: number_key(keys, "f", 32.0),
                width: sized("s"),
                height: sized("v"),
                compressed: keys.get("o").map(String::as_str) == Some("z"),
                payload: payload.to_string(),
                medium: keys.get("t").cloned().unwrap_or_else(|| "d".to_string()),
                png: None,
            },
        );
        let quiet = keys.get("q").map(String::as_str);
        if quiet != Some("1") && quiet != Some("2") {
            let reply = format!("\x1b_Gi={};OK\x1b\\", js_number_string(id));
            (self.reply)(&reply);
        }
    }

    fn place(&mut self, keys: &HashMap<String, String>) {
        let image_id = number_key(keys, "i", 0.0);
        if !self.images.contains(&js_number_string(image_id)) {
            return;
        }
        let (cursor_x, cursor_y) = self.terminal.cursor_position();
        let placement_id = number_key(keys, "p", 0.0);
        let placement = StoredPlacement {
            image_id,
            placement_id,
            col: f64::from(cursor_x),
            row: f64::from(cursor_y),
            cols: number_key(keys, "c", 0.0),
            rows: number_key(keys, "r", 0.0),
            z: number_key(keys, "z", 0.0),
        };
        self.placements.set(
            format!(
                "{}:{}",
                js_number_string(image_id),
                js_number_string(placement_id)
            ),
            placement,
        );
    }

    fn delete(&mut self, keys: &HashMap<String, String>) {
        let mode = keys.get("d").map_or("a", String::as_str);
        let free_data = mode == mode.to_uppercase();
        match mode.to_lowercase().as_str() {
            "a" => {
                self.placements.clear();
                if free_data {
                    self.images.clear();
                }
            }
            "i" => {
                let image_id = number_key(keys, "i", 0.0);
                let placement_id = keys
                    .get("p")
                    .filter(|value| !value.is_empty())
                    .map(|value| js_number(value));
                let doomed: Vec<String> = self
                    .placements
                    .entries
                    .iter()
                    .filter(|(_, placement)| {
                        placement.image_id == image_id
                            && placement_id.is_none_or(|id| placement.placement_id == id)
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in doomed {
                    self.placements.delete(&key);
                }
                if free_data && placement_id.is_none() {
                    self.images.delete(&js_number_string(image_id));
                }
            }
            _ => {}
        }
    }

    fn png_for(image: &mut StoredImage) -> std::io::Result<Option<Vec<u8>>> {
        if let Some(png) = &image.png {
            return Ok(Some(png.clone()));
        }
        let mut bytes = if image.medium == "f" {
            let path = String::from_utf8_lossy(&decode_base64_lenient(&image.payload)).to_string();
            match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(_) => return Ok(None),
            }
        } else {
            decode_base64_lenient(&image.payload)
        };
        if image.compressed {
            let mut inflated = Vec::new();
            ZlibDecoder::new(bytes.as_slice()).read_to_end(&mut inflated)?;
            bytes = inflated;
        }
        let png = if image.format == 100.0 {
            bytes
        } else {
            match (image.width, image.height) {
                (Some(width), Some(height))
                    if width != 0.0 && !width.is_nan() && height != 0.0 && !height.is_nan() =>
                {
                    let channels = if image.format == 24.0 { 3 } else { 4 };
                    encode_png(&bytes, width as u32, height as u32, channels)?
                }
                _ => return Ok(None),
            }
        };
        image.png = Some(png.clone());
        Ok(Some(png))
    }

    /// Pictures currently visible, ordered for painting.
    pub fn overlays(&mut self) -> std::io::Result<Vec<GraphicsOverlay>> {
        let mut result = Vec::new();
        for (_, placement) in &self.placements.entries {
            let key = js_number_string(placement.image_id);
            let Some((_, image)) = self.images.entries.iter_mut().find(|(k, _)| *k == key) else {
                continue;
            };
            let Some(png) = Self::png_for(image)? else {
                continue;
            };
            // `<= 0` is false for NaN, as in JS.
            if placement.cols <= 0.0 || placement.rows <= 0.0 {
                continue;
            }
            result.push(GraphicsOverlay {
                col: placement.col,
                row: placement.row,
                cols: placement.cols,
                rows: placement.rows,
                z: placement.z,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(&png)),
                png,
            });
        }
        result.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal));
        Ok(result)
    }
}

/// HTML for overlays, positioned in cell units so they track the font
/// metrics.
pub fn overlays_to_html(overlays: &[GraphicsOverlay], line_height: f64) -> String {
    overlays
        .iter()
        .map(|overlay| {
            format!(
                "<img class=\"tui-graphic\" src=\"{}\" style=\"left:{}ch;top:{}em;width:{}ch;height:{}em\" alt=\"\" />",
                overlay.data_url,
                js_number_string(overlay.col),
                js_number_string(overlay.row * line_height),
                js_number_string(overlay.cols),
                js_number_string(overlay.rows * line_height),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    fn make_tracker(cols: u16, rows: u16) -> (KittyGraphicsTracker, Arc<Mutex<Vec<String>>>) {
        let replies = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&replies);
        let tracker = KittyGraphicsTracker::new(KittyTrackerOptions {
            terminal: HeadlessTerminal::new(cols, rows),
            cols: u32::from(cols),
            rows: u32::from(rows),
            cell_width: 9,
            cell_height: 20,
            reply: Box::new(move |data| sink.lock().unwrap().push(data.to_string())),
        });
        (tracker, replies)
    }

    #[test]
    fn answers_the_capability_query_and_size_reports() {
        let (mut tracker, replies) = make_tracker(40, 10);
        tracker.write("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[16t\x1b[14t");
        assert_eq!(
            *replies.lock().unwrap(),
            vec!["\x1b_Gi=31;OK\x1b\\", "\x1b[6;20;9t", "\x1b[4;200;360t"]
        );
        assert_eq!(tracker.queries().len(), 1);
    }

    #[test]
    fn records_chunked_transmissions_and_placements_at_the_cursor_stripping_apc_from_text() {
        let (mut tracker, _) = make_tracker(40, 10);
        let (first, second) = (&PNG_1X1[..40], &PNG_1X1[40..]);
        tracker.write(&format!(
            "hello\x1b_Ga=t,i=5,f=100,q=2,t=d,m=1;{first}\x1b\\"
        ));
        tracker.write(&format!(
            "\x1b_Gm=0;{second}\x1b\\\x1b7\x1b[3;4H\x1b_Ga=p,i=5,p=1,c=6,r=2,C=1,q=2\x1b\\\x1b8 world"
        ));
        assert_eq!(tracker.terminal().plain_text(), "hello world");
        let overlays = tracker.overlays().unwrap();
        assert_eq!(overlays.len(), 1);
        assert_eq!(
            (
                overlays[0].col,
                overlays[0].row,
                overlays[0].cols,
                overlays[0].rows
            ),
            (3.0, 2.0, 6.0, 2.0)
        );
        assert!(
            overlays[0]
                .data_url
                .starts_with("data:image/png;base64,iVBORw0KGgo")
        );
        assert!(
            overlays_to_html(&overlays, 1.32)
                .contains("left:3ch;top:2.64em;width:6ch;height:2.64em")
        );
    }

    #[test]
    fn replaces_placements_with_the_same_ids_and_honors_deletes() {
        let (mut tracker, _) = make_tracker(40, 10);
        tracker.write(&format!("\x1b_Ga=t,i=5,f=100,q=2,t=d,m=0;{PNG_1X1}\x1b\\"));
        tracker.write(
            "\x1b[1;1H\x1b_Ga=p,i=5,p=1,c=2,r=1,q=2\x1b\\\x1b[2;1H\x1b_Ga=p,i=5,p=1,c=3,r=1,q=2\x1b\\",
        );
        assert_eq!(tracker.overlays().unwrap().len(), 1);
        let overlay = tracker.overlays().unwrap().remove(0);
        assert_eq!((overlay.row, overlay.cols), (1.0, 3.0));
        tracker.write("\x1b_Ga=d,d=i,i=5,p=1,q=2\x1b\\");
        assert_eq!(tracker.overlays().unwrap().len(), 0);
        assert_eq!(tracker.image_count(), 1);
        tracker.write("\x1b_Ga=d,d=A,q=2\x1b\\");
        assert_eq!(tracker.image_count(), 0);
    }

    #[test]
    fn buffers_a_sequence_split_across_writes() {
        let (mut tracker, _) = make_tracker(40, 10);
        let command = format!("\x1b_Ga=T,i=9,f=100,q=2,t=d,m=0;{PNG_1X1}\x1b\\");
        tracker.write(&format!("a{}", &command[..20]));
        tracker.write(&format!("{}b", &command[20..]));
        assert_eq!(tracker.terminal().plain_text(), "ab");
        // a=T without c/r is ignored for overlays
        assert_eq!(tracker.overlays().unwrap().len(), 0);
        assert_eq!(tracker.image_count(), 1);
    }

    #[test]
    fn encodes_raw_rgb_payloads_as_png_overlays() {
        let (mut tracker, _) = make_tracker(40, 10);
        let pixels = [255u8, 0, 0, 0, 255, 0];
        tracker.write(&format!(
            "\x1b_Ga=t,i=2,f=24,s=2,v=1,q=2,t=d,m=0;{}\x1b\\\x1b_Ga=p,i=2,p=1,c=2,r=1,q=2\x1b\\",
            STANDARD.encode(pixels)
        ));
        let overlay = tracker.overlays().unwrap().remove(0);
        let png = STANDARD
            .decode(overlay.data_url.split(',').nth(1).unwrap())
            .unwrap();
        assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 2);
        assert_eq!(encode_png(&pixels, 2, 1, 3).unwrap(), png);
        assert_eq!(overlay.png, png);
        assert!(overlay.to_raster_overlay().is_ok());
    }
}
