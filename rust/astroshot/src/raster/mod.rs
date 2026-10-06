//! Native terminal-frame rasterization (PORTING.md decision 2).
//!
//! Replaces the TS path "terminal -> HTML -> Chromium screenshot":
//! `packages/tui-shot/src/terminal-html.ts`, the `captureTerminalHtml` page in
//! `shot.ts`, and `packages/movie-harness/src/terminal-paint.ts`. An `alacritty_terminal`
//! screen is resolved to cells ([`TerminalFrame`]), glyphs are shaped and
//! rasterized with `cosmic-text` from a bundled JetBrains Mono, and everything
//! is composited with `tiny-skia`.
//!
//! The output is the pixels of the `[data-tui-shot]` element the TS
//! screenshotted: a rounded box of `css_size * scale` pixels with a 1px
//! border, the box background, padding, and the cell grid; transparent outside
//! the rounded corners (`omitBackground`). The 16px body margin and the
//! box-shadow are outside that element and are not part of the image.
//!
//! Known gaps against the Chromium output:
//! - Font: JetBrains Mono only; `fontFamily` is not honored. Glyphs for
//!   scripts it lacks (CJK, emoji, most symbols) are skipped, not drawn as
//!   tofu. Wide cells reserve two columns but draw only if the font has the
//!   glyph.
//! - Block elements (U+2580-259F) are drawn as exact cell fractions; box
//!   drawing lines come from the font and may leave sub-pixel gaps at other
//!   line heights.

mod colors;
mod frame;
mod render;

pub use colors::{ANSI_16, Rgb, Rgba, palette_color, parse_css_color, to_hex};
pub use frame::{
    HeadlessTerminal, Screen, StyledCell, TerminalFrame, style_cell, terminal_plain_text,
};
pub use render::Rasterizer;

use std::cell::RefCell;

/// Horizontal advance of one cell in ems. This is the advance width of the
/// bundled JetBrains Mono (600/1000 em). The TS box width uses 0.62 em per
/// column ([`RasterOptions::css_size`]); glyph pitch is the real advance.
pub const CELL_ADVANCE_EM: f32 = 0.6;

#[derive(Debug, thiserror::Error)]
pub enum RasterError {
    #[error("invalid raster option: {0}")]
    InvalidOption(String),
    #[error("invalid CSS color {0:?}")]
    InvalidColor(String),
    #[error("could not decode overlay image: {0}")]
    Overlay(String),
    #[error("could not encode PNG: {0}")]
    Encode(String),
}

/// An image painted over the cell grid (kitty graphics). Port of the
/// `<img class="tui-graphic">` from `overlaysToHtml`: `col`/`cols` in cells,
/// `row`/`rows` in line heights, stretched to fill (`object-fit: fill`).
#[derive(Clone, Debug)]
pub struct Overlay {
    pub col: f32,
    pub row: f32,
    pub cols: f32,
    pub rows: f32,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA8, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Overlay {
    /// Decode PNG bytes (what `GraphicsOverlay.dataUrl` carried).
    pub fn from_png(
        col: f32,
        row: f32,
        cols: f32,
        rows: f32,
        png: &[u8],
    ) -> Result<Self, RasterError> {
        let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .map_err(|error| RasterError::Overlay(error.to_string()))?
            .to_rgba8();
        Ok(Self {
            col,
            row,
            cols,
            rows,
            width: decoded.width(),
            height: decoded.height(),
            rgba: decoded.into_raw(),
        })
    }
}

/// A block cursor drawn by swapping the cell's foreground and background.
/// The TS paths never draw a cursor; this is opt-in and off by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
}

impl Cursor {
    /// The screen's cursor, or `None` when the program hid it.
    pub fn from_screen(screen: &Screen) -> Option<Self> {
        let (row, col, visible) = frame::cursor_state(screen);
        visible.then_some(Self { row, col })
    }
}

/// Mirrors `TerminalCaptureRequest` / `TerminalPaintOptions`, minus
/// `fontFamily` (the font is bundled) and the output path.
#[derive(Clone, Debug)]
pub struct RasterOptions {
    pub cols: u16,
    pub rows: u16,
    pub foreground: Rgba,
    pub background: Rgba,
    pub font_size: f32,
    pub line_height: f32,
    pub padding: f32,
    pub border_radius: f32,
    /// Device scale factor (`deviceScaleFactor`).
    pub scale: f32,
    pub cursor: Option<Cursor>,
    pub overlays: Vec<Overlay>,
}

impl RasterOptions {
    /// tui-shot defaults (`shot.ts`): `#090a12` on `#e8e8f2`, 15px font,
    /// line height 1.32, padding 22, radius 12, scale 2.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            foreground: [0xe8, 0xe8, 0xf2, 255],
            background: [0x09, 0x0a, 0x12, 255],
            font_size: 15.0,
            line_height: 1.32,
            padding: 22.0,
            border_radius: 12.0,
            scale: 2.0,
            cursor: None,
            overlays: Vec::new(),
        }
    }

    /// movie-harness PTY defaults (`recordPtyMovie`): 14px, line height 1.35,
    /// padding 16.
    pub fn movie(cols: u16, rows: u16) -> Self {
        Self {
            font_size: 14.0,
            line_height: 1.35,
            padding: 16.0,
            ..Self::new(cols, rows)
        }
    }

    /// Set the default colors from CSS strings (fixture `foreground` /
    /// `background`).
    pub fn with_css_colors(
        mut self,
        foreground: &str,
        background: &str,
    ) -> Result<Self, RasterError> {
        self.foreground = parse_css_color(foreground)
            .ok_or_else(|| RasterError::InvalidColor(foreground.to_string()))?;
        self.background = parse_css_color(background)
            .ok_or_else(|| RasterError::InvalidColor(background.to_string()))?;
        Ok(self)
    }

    /// Default foreground as opaque RGB, for [`TerminalFrame`] construction.
    pub fn foreground_rgb(&self) -> Rgb {
        [self.foreground[0], self.foreground[1], self.foreground[2]]
    }

    pub fn background_rgb(&self) -> Rgb {
        [self.background[0], self.background[1], self.background[2]]
    }

    /// `cssWidth` / `cssHeight` from `terminalDocument` and `captureTerminalHtml`:
    /// `ceil(cols * fontSize * 0.62 + padding * 2)` and
    /// `ceil(rows * fontSize * lineHeight + padding * 2)`.
    ///
    /// The movie session size is `css_size + 32` (viewport with the 16px body
    /// margin); the frames themselves are [`RasterOptions::pixel_size`].
    pub fn css_size(&self) -> (u32, u32) {
        let width = f32::from(self.cols) * self.font_size * 0.62 + self.padding * 2.0;
        let height = f32::from(self.rows) * self.font_size * self.line_height + self.padding * 2.0;
        (width.ceil() as u32, height.ceil() as u32)
    }

    /// Size of the rendered image: `css_size * scale`, rounded.
    pub fn pixel_size(&self) -> (u32, u32) {
        let (width, height) = self.css_size();
        (
            (width as f32 * self.scale).round() as u32,
            (height as f32 * self.scale).round() as u32,
        )
    }

    pub(crate) fn validate(&self) -> Result<(), RasterError> {
        let positive = |name: &str, value: f32, max: f32| {
            if value.is_finite() && value > 0.0 && value <= max {
                Ok(())
            } else {
                Err(RasterError::InvalidOption(format!(
                    "{name} must be greater than 0 and at most {max}"
                )))
            }
        };
        let non_negative = |name: &str, value: f32| {
            if value.is_finite() && (0.0..=1_000.0).contains(&value) {
                Ok(())
            } else {
                Err(RasterError::InvalidOption(format!(
                    "{name} must be between 0 and 1000"
                )))
            }
        };
        if self.cols == 0 || self.rows == 0 {
            return Err(RasterError::InvalidOption(
                "cols and rows must be at least 1".to_string(),
            ));
        }
        positive("fontSize", self.font_size, 200.0)?;
        positive("lineHeight", self.line_height, 10.0)?;
        positive("scale", self.scale, 8.0)?;
        non_negative("padding", self.padding)?;
        non_negative("borderRadius", self.border_radius)
    }
}

/// Straight-alpha RGBA8 image, `width * height * 4` bytes, row-major. The
/// buffer API for the movie path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        [
            self.data[at],
            self.data[at + 1],
            self.data[at + 2],
            self.data[at + 3],
        ]
    }
}

thread_local! {
    static SHARED: RefCell<Option<Rasterizer>> = const { RefCell::new(None) };
}

fn with_shared<T>(run: impl FnOnce(&mut Rasterizer) -> T) -> T {
    SHARED.with(|slot| {
        let mut slot = slot.borrow_mut();
        run(slot.get_or_insert_with(Rasterizer::new))
    })
}

/// Rasterize a frame to straight-alpha RGBA. Replaces
/// `page.setContent(html)` + `locator("[data-tui-shot]").screenshot()` in the
/// movie path (`movie-harness/src/sources/pty.ts` `sample`, and the truecolor
/// demo loop). Uses a per-thread [`Rasterizer`] so glyphs stay cached across
/// frames; hold your own `Rasterizer` to control that.
pub fn render_rgba(
    frame: &TerminalFrame,
    options: &RasterOptions,
) -> Result<RgbaImage, RasterError> {
    with_shared(|rasterizer| rasterizer.render_rgba(frame, options))
}

/// Rasterize a frame to PNG bytes. Replaces `captureTerminalHtml` in
/// `tui-shot/src/shot.ts` (used by `renderTuiShot` and `pty-shot.ts`).
pub fn render_png(frame: &TerminalFrame, options: &RasterOptions) -> Result<Vec<u8>, RasterError> {
    with_shared(|rasterizer| rasterizer.render_png(frame, options))
}

/// Interpret an ANSI frame at `cols x rows` (the `ansiFrameToHtml` step) and
/// render it to PNG in one call; the Ink shot path.
pub fn render_ansi_png(ansi: &[u8], options: &RasterOptions) -> Result<Vec<u8>, RasterError> {
    let frame = TerminalFrame::from_ansi(
        ansi,
        options.cols,
        options.rows,
        options.foreground_rgb(),
        options.background_rgb(),
    );
    render_png(&frame, options)
}

/// Encode straight-alpha RGBA8 as PNG (for callers that composite first).
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, RasterError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&image.data))
        .map_err(|error| RasterError::Encode(error.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests;
