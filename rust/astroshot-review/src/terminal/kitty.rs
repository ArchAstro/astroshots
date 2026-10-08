//! Kitty terminal graphics protocol encoder.
//!
//! Only the subset the review tray needs: transmit PNG or raw RGB data by id,
//! place a transmitted image over a cell box, and delete placements or image
//! data. Every command is emitted quietly (q=2) so the terminal never answers
//! on stdin while the TUI owns it.
//!
//! Spec: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub const APC: &str = "\x1b_G";
pub const ST: &str = "\x1b\\";

/// Payload chunk size; the protocol caps chunks at 4096 bytes.
pub const CHUNK_SIZE: usize = 4096;

/// `f=` value: 100 = PNG bytes, 24 = RGB, 32 = RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png = 100,
    Rgb = 24,
    Rgba = 32,
}

#[derive(Debug, Clone)]
pub struct TransmitRequest<'a> {
    pub id: u32,
    pub format: ImageFormat,
    pub data: &'a [u8],
    /// Required for raw formats.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Raw payload is zlib-compressed (o=z).
    pub compressed: bool,
    /// When set (and non-empty) the payload is a base64 file path the
    /// terminal reads itself (t=f). Only valid when the terminal runs on this
    /// machine.
    pub file_path: Option<String>,
}

impl<'a> TransmitRequest<'a> {
    pub fn new(id: u32, format: ImageFormat, data: &'a [u8]) -> Self {
        Self {
            id,
            format,
            data,
            width: None,
            height: None,
            compressed: false,
            file_path: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlaceRequest {
    pub id: u32,
    pub placement_id: u32,
    pub cols: u32,
    pub rows: u32,
    /// Stacking order; images with z >= 0 draw above text. Defaults to 0.
    pub z: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteRequest {
    Placement { id: u32, placement_id: u32 },
    Image { id: u32 },
    AllPlacements,
    All,
}

/// Ordered `key=value` pairs joined by commas; `None` values are skipped.
fn control(pairs: &[(&str, Option<String>)]) -> String {
    pairs
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|value| format!("{key}={value}")))
        .collect::<Vec<_>>()
        .join(",")
}

fn num(value: impl ToString) -> Option<String> {
    Some(value.to_string())
}

fn text(value: &str) -> Option<String> {
    Some(value.to_string())
}

pub fn chunk_payload(base64: &str, size: usize) -> Vec<String> {
    if base64.is_empty() {
        return vec![String::new()];
    }
    base64
        .as_bytes()
        .chunks(size)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect()
}

/// Transmit image data without displaying it (a=t).
pub fn encode_transmit(request: &TransmitRequest<'_>) -> String {
    let png = request.format == ImageFormat::Png;
    let base: Vec<(&str, Option<String>)> = vec![
        ("a", text("t")),
        ("i", num(request.id)),
        ("f", num(request.format as u32)),
        ("q", num(2)),
        (
            "s",
            if png {
                None
            } else {
                request.width.and_then(num)
            },
        ),
        (
            "v",
            if png {
                None
            } else {
                request.height.and_then(num)
            },
        ),
        ("o", if request.compressed { text("z") } else { None }),
    ];
    if let Some(path) = request.file_path.as_deref().filter(|path| !path.is_empty()) {
        let payload = STANDARD.encode(path.as_bytes());
        let mut pairs = base;
        pairs.push(("t", text("f")));
        return format!("{APC}{};{payload}{ST}", control(&pairs));
    }
    let chunks = chunk_payload(&STANDARD.encode(request.data), CHUNK_SIZE);
    let last = chunks.len() - 1;
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let more = if index < last { 1 } else { 0 };
            let keys = if index == 0 {
                let mut pairs = base.clone();
                pairs.push(("t", text("d")));
                pairs.push(("m", num(more)));
                control(&pairs)
            } else {
                format!("m={more}")
            };
            format!("{APC}{keys};{chunk}{ST}")
        })
        .collect()
}

/// Display a transmitted image over a c×r cell box at the cursor (a=p). C=1
/// keeps the cursor where it was so the TUI's own cursor bookkeeping stays
/// valid. A placement with the same (i, p) pair replaces the previous one.
pub fn encode_place(request: &PlaceRequest) -> String {
    format!(
        "{APC}{}{ST}",
        control(&[
            ("a", text("p")),
            ("i", num(request.id)),
            ("p", num(request.placement_id)),
            ("c", num(request.cols)),
            ("r", num(request.rows)),
            ("z", num(request.z.unwrap_or(0))),
            ("C", num(1)),
            ("q", num(2)),
        ])
    )
}

pub fn encode_delete(request: &DeleteRequest) -> String {
    let pairs = match request {
        DeleteRequest::Placement { id, placement_id } => {
            vec![
                ("a", text("d")),
                ("d", text("i")),
                ("i", num(id)),
                ("p", num(placement_id)),
                ("q", num(2)),
            ]
        }
        DeleteRequest::Image { id } => vec![
            ("a", text("d")),
            ("d", text("I")),
            ("i", num(id)),
            ("q", num(2)),
        ],
        DeleteRequest::AllPlacements => vec![("a", text("d")), ("d", text("a")), ("q", num(2))],
        DeleteRequest::All => vec![("a", text("d")), ("d", text("A")), ("q", num(2))],
    };
    format!("{APC}{}{ST}", control(&pairs))
}

/// The 1×1 RGB query kitty documents for capability detection.
pub fn encode_query(id: u32) -> String {
    format!(
        "{APC}{};AAAA{ST}",
        control(&[
            ("i", num(id)),
            ("s", num(1)),
            ("v", num(1)),
            ("a", text("q")),
            ("t", text("d")),
            ("f", num(24))
        ])
    )
}

pub fn encode_file_query(id: u32, file_path: &str) -> String {
    let payload = STANDARD.encode(file_path.as_bytes());
    format!(
        "{APC}{};{payload}{ST}",
        control(&[
            ("i", num(id)),
            ("a", text("q")),
            ("t", text("f")),
            ("f", num(100))
        ])
    )
}

/// Move the cursor to a 1-based row/column (CUP).
pub fn cursor_to(row: u32, col: u32) -> String {
    format!("\x1b[{row};{col}H")
}

pub const SAVE_CURSOR: &str = "\x1b7";
pub const RESTORE_CURSOR: &str = "\x1b8";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGraphicsCommand {
    pub keys: HashMap<String, String>,
    pub payload: String,
}

/// Parse one `ESC _ G <keys> ; <payload> ESC \` command body.
pub fn parse_graphics_command(body: &str) -> ParsedGraphicsCommand {
    let (key_text, payload) = match body.find(';') {
        Some(separator) => (&body[..separator], &body[separator + 1..]),
        None => (body, ""),
    };
    let mut keys = HashMap::new();
    for pair in key_text.split(',') {
        if pair.is_empty() {
            continue;
        }
        let Some(equals) = pair.find('=') else {
            continue;
        };
        keys.insert(pair[..equals].to_string(), pair[equals + 1..].to_string());
    }
    ParsedGraphicsCommand {
        keys,
        payload: payload.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `\x1b_G([^\x1b]*)\x1b\\` matches of the TS test, as command bodies.
    fn command_bodies(output: &str) -> Vec<&str> {
        output
            .split(APC)
            .skip(1)
            .map(|part| part.strip_suffix(ST).expect("command ends with ST"))
            .collect()
    }

    #[test]
    fn chunks_base64_payloads_at_the_protocol_limit_with_continuation_flags() {
        let data = vec![1u8; 5000];
        let output = encode_transmit(&TransmitRequest::new(7, ImageFormat::Png, &data));
        let commands: Vec<_> = command_bodies(&output)
            .into_iter()
            .map(parse_graphics_command)
            .collect();
        assert_eq!(commands.len(), 2);
        for (key, value) in [
            ("a", "t"),
            ("i", "7"),
            ("f", "100"),
            ("t", "d"),
            ("m", "1"),
            ("q", "2"),
        ] {
            assert_eq!(
                commands[0].keys.get(key).map(String::as_str),
                Some(value),
                "key {key}"
            );
        }
        assert_eq!(
            commands[1].keys,
            HashMap::from([("m".to_string(), "0".to_string())])
        );
        assert_eq!(commands[0].payload.len(), 4096);
        let joined: String = commands
            .iter()
            .map(|command| command.payload.as_str())
            .collect();
        assert_eq!(STANDARD.decode(joined).unwrap(), data);
    }

    #[test]
    fn uses_the_file_medium_when_a_path_is_given() {
        let mut request = TransmitRequest::new(3, ImageFormat::Png, &[]);
        request.file_path = Some("/tmp/a.png".to_string());
        let output = encode_transmit(&request);
        let command = parse_graphics_command(&output[3..output.len() - 2]);
        assert_eq!(command.keys.get("t").map(String::as_str), Some("f"));
        assert_eq!(
            String::from_utf8(STANDARD.decode(&command.payload).unwrap()).unwrap(),
            "/tmp/a.png"
        );
    }

    #[test]
    fn declares_raw_dimensions_and_compression_for_rgb_payloads() {
        let mut request = TransmitRequest::new(1, ImageFormat::Rgb, &[0, 0, 0, 0, 0, 0]);
        request.width = Some(2);
        request.height = Some(1);
        request.compressed = true;
        let output = encode_transmit(&request);
        assert!(output.contains("f=24"));
        assert!(output.contains("s=2,v=1"));
        assert!(output.contains("o=z"));
    }

    #[test]
    fn places_without_moving_the_cursor_and_quietly() {
        assert_eq!(
            encode_place(&PlaceRequest {
                id: 9,
                placement_id: 2,
                cols: 10,
                rows: 3,
                z: None
            }),
            "\x1b_Ga=p,i=9,p=2,c=10,r=3,z=0,C=1,q=2\x1b\\"
        );
    }

    #[test]
    fn encodes_every_delete_form() {
        assert_eq!(
            encode_delete(&DeleteRequest::Placement {
                id: 9,
                placement_id: 2
            }),
            "\x1b_Ga=d,d=i,i=9,p=2,q=2\x1b\\"
        );
        assert_eq!(
            encode_delete(&DeleteRequest::Image { id: 9 }),
            "\x1b_Ga=d,d=I,i=9,q=2\x1b\\"
        );
        assert_eq!(
            encode_delete(&DeleteRequest::AllPlacements),
            "\x1b_Ga=d,d=a,q=2\x1b\\"
        );
        assert_eq!(
            encode_delete(&DeleteRequest::All),
            "\x1b_Ga=d,d=A,q=2\x1b\\"
        );
    }

    #[test]
    fn formats_the_capability_query_kitty_documents() {
        assert_eq!(
            encode_query(31),
            "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"
        );
    }

    #[test]
    fn chunks_empty_payloads_as_one_empty_chunk() {
        assert_eq!(chunk_payload("", CHUNK_SIZE), vec![String::new()]);
    }
}
