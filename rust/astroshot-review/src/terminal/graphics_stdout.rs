//! Port of `packages/astroshot-review/src/terminal/graphics-stdout.ts`.
//!
//! The stdout the UI renders into. It forwards everything to the real stream
//! and splices kitty placements into each frame so pictures and text arrive
//! in the same synchronized update.
//!
//! The TS wraps a `NodeJS.WriteStream` in a `Proxy` that overrides `write`;
//! Rust wraps any `io::Write` in [`GraphicsStdout`], which implements `Write`.

use std::io::{self, Write};
use std::sync::LazyLock;

use regex::Regex;

use super::image_layer::ImageLayer;

const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
const END_SYNC: &str = "\x1b[?2026l";

static ANSI_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b_[^\x1b]*\x1b\\|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-_]",
    )
    .expect("valid ANSI pattern")
});

/// `String.prototype.trim` whitespace (WhiteSpace and LineTerminator).
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | '\u{20}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

pub fn has_printable_text(chunk: &str) -> bool {
    !ANSI_PATTERN
        .replace_all(chunk, "")
        .trim_matches(is_js_whitespace)
        .is_empty()
}

pub fn splice_graphics(chunk: &str, graphics: &str) -> String {
    if graphics.is_empty() {
        return chunk.to_string();
    }
    match chunk.strip_suffix(END_SYNC) {
        Some(body) => format!("{body}{graphics}{END_SYNC}"),
        None => format!("{chunk}{graphics}"),
    }
}

/// Wraps the real output stream; see the module docs.
pub struct GraphicsStdout<W: Write> {
    real: W,
    layer: ImageLayer,
}

pub fn create_graphics_stdout<W: Write>(real: W, layer: ImageLayer) -> GraphicsStdout<W> {
    GraphicsStdout { real, layer }
}

impl<W: Write> GraphicsStdout<W> {
    /// The TS `write` override: one chunk of text in, one chunk out.
    pub fn write_str(&mut self, chunk: &str) -> io::Result<()> {
        let mut text = chunk.to_string();
        if text.contains(ENTER_ALT_SCREEN) {
            // The UI enters the alternate screen without homing the cursor.
            // Frames must start at the top-left so layout coordinates map to
            // screen cells.
            text = text.replacen(
                ENTER_ALT_SCREEN,
                &format!("{ENTER_ALT_SCREEN}\x1b[H\x1b[2J"),
                1,
            );
            self.layer.invalidate();
        }
        if has_printable_text(&text) {
            text = splice_graphics(&text, &self.layer.render());
        }
        self.real.write_all(text.as_bytes())
    }

    pub fn into_inner(self) -> W {
        self.real
    }
}

impl<W: Write> Write for GraphicsStdout<W> {
    /// Bytes are decoded as UTF-8 (lossy), like `Buffer.toString("utf8")`.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_str(&String::from_utf8_lossy(buf))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.real.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_frames_with_visible_text_and_ignores_pure_control_writes() {
        assert!(!has_printable_text("\x1b[2K\x1b[1A\x1b[G"));
        assert!(!has_printable_text("\x1b[?2026h"));
        assert!(has_printable_text("\x1b[2K hello \x1b[0m"));
        assert!(!has_printable_text("\x1b_Ga=p,i=1\x1b\\"));
    }

    #[test]
    fn keeps_placements_inside_the_synchronized_update_block() {
        assert_eq!(
            splice_graphics("frame\x1b[?2026l", "<g>"),
            "frame<g>\x1b[?2026l"
        );
        assert_eq!(splice_graphics("frame", "<g>"), "frame<g>");
        assert_eq!(splice_graphics("frame", ""), "frame");
    }
}
