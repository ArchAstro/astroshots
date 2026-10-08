//! Color tables and CSS color parsing.
//!
//! `ANSI_16`, the 256-color cube and grayscale ramp are copied from
//! `packages/tui-shot/src/terminal-html.ts` (identical in
//! `packages/movie-harness/src/terminal-paint.ts`).

/// Opaque color, `[red, green, blue]`.
pub type Rgb = [u8; 3];

/// Color with straight (non-premultiplied) alpha, `[r, g, b, a]`.
pub type Rgba = [u8; 4];

/// The 16 ANSI colors, as `#rrggbb` values from the TS `ANSI_16` table.
pub const ANSI_16: [Rgb; 16] = [
    [0x2b, 0x2f, 0x3a],
    [0xf0, 0x72, 0x7a],
    [0x54, 0xe0, 0xa0],
    [0xf0, 0xb8, 0x6e],
    [0x7a, 0xa2, 0xf7],
    [0xb9, 0xa8, 0xff],
    [0x5b, 0xd6, 0xe0],
    [0xe8, 0xe8, 0xf2],
    [0x5a, 0x5a, 0x7a],
    [0xff, 0x8d, 0x94],
    [0x78, 0xef, 0xb6],
    [0xff, 0xd0, 0x8a],
    [0x9b, 0xbc, 0xff],
    [0xd1, 0xc5, 0xff],
    [0x86, 0xea, 0xf0],
    [0xff, 0xff, 0xff],
];

/// Port of `paletteColor`: indexes 0-15 use `ANSI_16`, 16-231 the 6x6x6 cube
/// with levels `[0, 95, 135, 175, 215, 255]`, 232-255 the grayscale ramp
/// `8 + (index - 232) * 10`.
pub fn palette_color(index: u8) -> Rgb {
    let index = usize::from(index);
    if index < ANSI_16.len() {
        return ANSI_16[index];
    }
    if index < 232 {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let value = index - 16;
        return [
            LEVELS[value / 36],
            LEVELS[(value % 36) / 6],
            LEVELS[value % 6],
        ];
    }
    let gray = (8 + (index - 232) * 10) as u8;
    [gray, gray, gray]
}

/// `#rrggbb` for a color, lowercase (the TS `rgb()` helper's output).
pub fn to_hex(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// Parse the CSS color forms fixtures use: `#rgb`, `#rgba`, `#rrggbb`,
/// `#rrggbbaa`, `rgb()/rgba()` with integer or percent channels, and the names
/// `transparent`, `black`, `white`. Anything else is `None`.
pub fn parse_css_color(input: &str) -> Option<Rgba> {
    let text = input.trim().to_ascii_lowercase();
    match text.as_str() {
        "transparent" => return Some([0, 0, 0, 0]),
        "black" => return Some([0, 0, 0, 255]),
        "white" => return Some([255, 255, 255, 255]),
        _ => {}
    }
    if let Some(hex) = text.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|v| v * 17);
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            3 => Some([nibble(0)?, nibble(1)?, nibble(2)?, 255]),
            4 => Some([nibble(0)?, nibble(1)?, nibble(2)?, nibble(3)?]),
            6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
            8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
            _ => None,
        };
    }
    let body = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = body
        .split([',', '/', ' '])
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let channel = |part: &str| -> Option<u8> {
        let value = match part.strip_suffix('%') {
            Some(percent) => percent.parse::<f32>().ok()? * 2.55,
            None => part.parse::<f32>().ok()?,
        };
        Some(value.clamp(0.0, 255.0).round() as u8)
    };
    let alpha = match parts.get(3) {
        None => 255,
        Some(part) => {
            let value = match part.strip_suffix('%') {
                Some(percent) => percent.parse::<f32>().ok()? / 100.0,
                None => part.parse::<f32>().ok()?,
            };
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        }
    };
    Some([
        channel(parts[0])?,
        channel(parts[1])?,
        channel(parts[2])?,
        alpha,
    ])
}
