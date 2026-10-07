//! Painting: box, cell backgrounds, glyphs, decorations, overlays.

use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight,
    fontdb,
};
use tiny_skia::{
    FillRule, FilterQuality, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

use super::colors::{Rgb, Rgba};
use super::{CELL_ADVANCE_EM, Overlay, RasterError, RasterOptions, RgbaImage, TerminalFrame};

const FONT_FAMILY: &str = "JetBrains Mono";
const FONT_REGULAR: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf");
const FONT_ITALIC: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Italic.ttf");
const FONT_BOLD_ITALIC: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-BoldItalic.ttf");

/// `opacity: .62` for dim cells.
const DIM_OPACITY: f32 = 0.62;
/// Border from `[data-tui-shot]`: `1px solid rgba(185, 168, 255, .18)`.
const BORDER_COLOR: Rgba = [185, 168, 255, 46];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    text: String,
    bold: bool,
    italic: bool,
    size_bits: u64,
    line_bits: u64,
}

/// One rasterized glyph, positioned relative to the top-left of its cell.
struct GlyphBitmap {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    /// Alpha mask (`color == false`, 1 byte per pixel) or straight RGBA.
    data: Vec<u8>,
    color: bool,
}

/// Owns the font system and glyph cache. Creating one parses the bundled
/// fonts; keep it for the length of a movie.
pub struct Rasterizer {
    font_system: FontSystem,
    swash: SwashCache,
    glyphs: HashMap<GlyphKey, std::rc::Rc<Vec<GlyphBitmap>>>,
}

impl Default for Rasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rasterizer {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        for data in [FONT_REGULAR, FONT_BOLD, FONT_ITALIC, FONT_BOLD_ITALIC] {
            db.load_font_data(data.to_vec());
        }
        db.set_monospace_family(FONT_FAMILY);
        db.set_sans_serif_family(FONT_FAMILY);
        db.set_serif_family(FONT_FAMILY);
        // A fixed locale and only the bundled fonts keep output identical
        // across machines.
        let font_system = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
        Self {
            font_system,
            swash: SwashCache::new(),
            glyphs: HashMap::new(),
        }
    }

    pub fn render_png(
        &mut self,
        frame: &TerminalFrame,
        options: &RasterOptions,
    ) -> Result<Vec<u8>, RasterError> {
        super::encode_png(&self.render_rgba(frame, options)?)
    }

    pub fn render_rgba(
        &mut self,
        frame: &TerminalFrame,
        options: &RasterOptions,
    ) -> Result<RgbaImage, RasterError> {
        options.validate()?;
        let (width, height) = options.pixel_size();
        let mut pixmap = Pixmap::new(width, height)
            .ok_or_else(|| RasterError::InvalidOption("image size is empty".to_string()))?;
        let scale = options.scale;
        let (css_width, css_height) = options.css_size();
        // Geometry is computed in f64 from the JS-number options and narrowed
        // to f32 only where tiny-skia and cosmic-text take it.
        let box_width = f64::from(css_width) * scale;
        let box_height = f64::from(css_height) * scale;

        // Box: background under the border (background-clip: border-box), then
        // the 1px border ring.
        let box_path = rounded_rect(
            0.0,
            0.0,
            box_width as f32,
            box_height as f32,
            (options.border_radius * scale) as f32,
        );
        pixmap.fill_path(
            &box_path,
            &paint(options.background),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        let half = (0.5 * scale) as f32;
        let ring = rounded_rect(
            half,
            half,
            (box_width - scale) as f32,
            (box_height - scale) as f32,
            ((options.border_radius - 0.5).max(0.0) * scale) as f32,
        );
        let stroke = Stroke {
            width: scale as f32,
            ..Stroke::default()
        };
        pixmap.stroke_path(
            &ring,
            &paint(BORDER_COLOR),
            &stroke,
            Transform::identity(),
            None,
        );

        let parent_background = options.background_rgb();
        let cursor = options.cursor;
        let layout = Layout::new(options);

        for row in 0..frame.rows.min(options.rows) {
            for col in 0..frame.cols.min(options.cols) {
                let Some(cell) = frame.cell(row, col) else {
                    continue;
                };
                if cell.width == 0 {
                    continue;
                }
                let mut cell = cell.clone();
                if cursor.is_some_and(|c| c.row == row && c.col == col) {
                    std::mem::swap(&mut cell.foreground, &mut cell.background);
                    cell.invisible = false;
                }
                self.paint_cell(
                    &mut pixmap,
                    &layout,
                    row,
                    col,
                    &cell,
                    parent_background,
                    options,
                );
            }
        }

        for overlay in &options.overlays {
            paint_overlay(&mut pixmap, &layout, overlay)?;
        }

        let data = pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let color = pixel.demultiply();
                [color.red(), color.green(), color.blue(), color.alpha()]
            })
            .collect();
        Ok(RgbaImage {
            width,
            height,
            data,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_cell(
        &mut self,
        pixmap: &mut Pixmap,
        layout: &Layout,
        row: u16,
        col: u16,
        cell: &super::StyledCell,
        parent_background: Rgb,
        options: &RasterOptions,
    ) {
        // `opacity: .62` fades the whole span, background included, over the
        // box background behind it.
        let fade = |color: Rgb| -> Rgb {
            if cell.dim {
                mix(color, parent_background, DIM_OPACITY)
            } else {
                color
            }
        };
        let background = fade(cell.background);
        let foreground = fade(cell.text_color());

        let x0 = layout.edge_x(col);
        let x1 = layout.edge_x(col + u16::from(cell.width));
        let y0 = layout.edge_y(row);
        let y1 = layout.edge_y(row + 1);

        if background != options.background_rgb() || cell.dim {
            fill(
                pixmap,
                x0,
                y0,
                x1,
                y1,
                [background[0], background[1], background[2], 255],
            );
        }

        let first = cell.text.chars().next().unwrap_or(' ');
        let single = cell.text.chars().count() == 1;
        if single && let Some((shapes, alpha)) = block_element(first) {
            let w = f64::from(x1 - x0);
            let h = f64::from(y1 - y0);
            for (l, t, r, b) in shapes {
                fill(
                    pixmap,
                    x0 + (l * w).round() as i32,
                    y0 + (t * h).round() as i32,
                    x0 + (r * w).round() as i32,
                    y0 + (b * h).round() as i32,
                    [foreground[0], foreground[1], foreground[2], alpha],
                );
            }
            return;
        }

        if !cell.invisible && !cell.text.trim().is_empty() {
            let glyphs = self.glyphs_for(&cell.text, cell.bold, cell.italic, layout);
            for glyph in glyphs.iter() {
                draw_glyph(pixmap, glyph, x0, y0, foreground);
            }
        }

        // Text decorations take the text color, as CSS does.
        let thickness = (layout.font_px / 15.0).round().max(1.0) as i32;
        let baseline = y0 + layout.baseline();
        if cell.underline {
            let top = baseline + (layout.font_px * 0.1).round() as i32;
            fill(pixmap, x0, top, x1, top + thickness, opaque(foreground));
        }
        if cell.strike {
            let top = baseline - (layout.font_px * 0.28).round() as i32;
            fill(pixmap, x0, top, x1, top + thickness, opaque(foreground));
        }
    }

    fn glyphs_for(
        &mut self,
        text: &str,
        bold: bool,
        italic: bool,
        layout: &Layout,
    ) -> std::rc::Rc<Vec<GlyphBitmap>> {
        let key = GlyphKey {
            text: text.to_string(),
            bold,
            italic,
            size_bits: layout.font_px.to_bits(),
            line_bits: layout.row_height.to_bits(),
        };
        if let Some(found) = self.glyphs.get(&key) {
            return found.clone();
        }
        let attrs = Attrs::new()
            .family(Family::Name(FONT_FAMILY))
            .weight(if bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if italic { Style::Italic } else { Style::Normal });
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(layout.font_px as f32, layout.row_height as f32),
        );
        buffer.set_size(&mut self.font_system, None, None);
        buffer.set_text(&mut self.font_system, text, attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut bitmaps = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                // Glyph id 0 is `.notdef`: the font has no glyph, skip tofu.
                if glyph.glyph_id == 0 {
                    continue;
                }
                let physical = glyph.physical((0.0, run.line_y), 1.0);
                let Some(image) = self
                    .swash
                    .get_image(&mut self.font_system, physical.cache_key)
                    .clone()
                else {
                    continue;
                };
                if image.placement.width == 0 || image.placement.height == 0 {
                    continue;
                }
                let color = match image.content {
                    SwashContent::Mask => false,
                    SwashContent::Color => true,
                    SwashContent::SubpixelMask => continue,
                };
                bitmaps.push(GlyphBitmap {
                    left: physical.x + image.placement.left,
                    top: physical.y - image.placement.top,
                    width: image.placement.width,
                    height: image.placement.height,
                    data: image.data,
                    color,
                });
            }
        }
        let shared = std::rc::Rc::new(bitmaps);
        self.glyphs.insert(key, shared.clone());
        shared
    }
}

/// Cell grid in device pixels, in f64. The content box starts inside the 1px
/// border and the padding; a row is `lineHeight` em tall (`.tui-row`).
pub(super) struct Layout {
    origin: f64,
    pitch: f64,
    row_height: f64,
    font_px: f64,
}

impl Layout {
    pub(super) fn new(options: &RasterOptions) -> Self {
        let font_px = options.font_size * options.scale;
        Self {
            origin: (1.0 + options.padding) * options.scale,
            pitch: CELL_ADVANCE_EM * font_px,
            row_height: options.line_height * font_px,
            font_px,
        }
    }

    pub(super) fn edge_x(&self, col: u16) -> i32 {
        (self.origin + f64::from(col) * self.pitch).round() as i32
    }

    pub(super) fn edge_y(&self, row: u16) -> i32 {
        (self.origin + f64::from(row) * self.row_height).round() as i32
    }

    /// Baseline offset from the top of a row: font ascent plus half-leading
    /// (JetBrains Mono: ascent 1.02 em, descent 0.30 em).
    pub(super) fn baseline(&self) -> i32 {
        let content = 1.32 * self.font_px;
        ((self.row_height - content) / 2.0 + 1.02 * self.font_px).round() as i32
    }

    /// Device-pixel rectangle `(left, top, width, height)` of an overlay
    /// placed in cell units (`left: {col}ch`-equivalent, `top: {row *
    /// lineHeight}em`).
    pub(super) fn overlay_rect(&self, overlay: &Overlay) -> (f64, f64, f64, f64) {
        (
            self.origin + overlay.col * self.pitch,
            self.origin + overlay.row * self.row_height,
            overlay.cols * self.pitch,
            overlay.rows * self.row_height,
        )
    }
}

fn opaque(color: Rgb) -> Rgba {
    [color[0], color[1], color[2], 255]
}

fn mix(top: Rgb, bottom: Rgb, top_weight: f32) -> Rgb {
    let blend = |a: u8, b: u8| {
        (f32::from(a) * top_weight + f32::from(b) * (1.0 - top_weight)).round() as u8
    };
    [
        blend(top[0], bottom[0]),
        blend(top[1], bottom[1]),
        blend(top[2], bottom[2]),
    ]
}

fn paint(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint.anti_alias = true;
    paint
}

fn fill(pixmap: &mut Pixmap, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgba) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let Some(rect) = Rect::from_ltrb(x0 as f32, y0 as f32, x1 as f32, y1 as f32) else {
        return;
    };
    let mut paint = paint(color);
    paint.anti_alias = false;
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Path {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = r * 0.552_284_8;
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x + w - r, y);
    b.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    b.line_to(x + w, y + h - r);
    b.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    b.line_to(x + r, y + h);
    b.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    b.line_to(x, y + r);
    b.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    b.close();
    b.finish().expect("rounded rect path has points")
}

fn draw_glyph(pixmap: &mut Pixmap, glyph: &GlyphBitmap, cell_x: i32, cell_y: i32, color: Rgb) {
    let size = glyph.width as usize * glyph.height as usize;
    let mut data = Vec::with_capacity(size * 4);
    if glyph.color {
        for px in glyph.data.chunks_exact(4) {
            let a = u32::from(px[3]);
            data.extend([
                (u32::from(px[0]) * a / 255) as u8,
                (u32::from(px[1]) * a / 255) as u8,
                (u32::from(px[2]) * a / 255) as u8,
                px[3],
            ]);
        }
    } else {
        for &alpha in &glyph.data {
            let a = u32::from(alpha);
            data.extend([
                (u32::from(color[0]) * a / 255) as u8,
                (u32::from(color[1]) * a / 255) as u8,
                (u32::from(color[2]) * a / 255) as u8,
                alpha,
            ]);
        }
    }
    let Some(size) = tiny_skia::IntSize::from_wh(glyph.width, glyph.height) else {
        return;
    };
    let Some(mask) = Pixmap::from_vec(data, size) else {
        return;
    };
    pixmap.draw_pixmap(
        cell_x + glyph.left,
        cell_y + glyph.top,
        mask.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

fn paint_overlay(
    pixmap: &mut Pixmap,
    layout: &Layout,
    overlay: &Overlay,
) -> Result<(), RasterError> {
    let size = tiny_skia::IntSize::from_wh(overlay.width, overlay.height)
        .ok_or_else(|| RasterError::Overlay("overlay image is empty".to_string()))?;
    let mut data = Vec::with_capacity(overlay.rgba.len());
    for px in overlay.rgba.chunks_exact(4) {
        let a = u32::from(px[3]);
        data.extend([
            (u32::from(px[0]) * a / 255) as u8,
            (u32::from(px[1]) * a / 255) as u8,
            (u32::from(px[2]) * a / 255) as u8,
            px[3],
        ]);
    }
    let source = Pixmap::from_vec(data, size).ok_or_else(|| {
        RasterError::Overlay("overlay pixel data has the wrong length".to_string())
    })?;
    let (left, top, width, height) = layout.overlay_rect(overlay);
    let transform = Transform::from_scale(
        (width / f64::from(overlay.width)) as f32,
        (height / f64::from(overlay.height)) as f32,
    )
    .post_translate(left as f32, top as f32);
    let paint = PixmapPaint {
        quality: FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    pixmap.draw_pixmap(0, 0, source.as_ref(), &paint, transform, None);
    Ok(())
}

type Shape = (f64, f64, f64, f64);

/// Block Elements (U+2580-259F) as fractions of the cell, plus alpha. Drawn
/// exactly so adjacent blocks tile without font-metric gaps.
fn block_element(ch: char) -> Option<(Vec<Shape>, u8)> {
    let code = ch as u32;
    let quadrant = |mask: u8| -> Vec<Shape> {
        let mut shapes = Vec::new();
        if mask & 1 != 0 {
            shapes.push((0.0, 0.0, 0.5, 0.5));
        }
        if mask & 2 != 0 {
            shapes.push((0.5, 0.0, 1.0, 0.5));
        }
        if mask & 4 != 0 {
            shapes.push((0.0, 0.5, 0.5, 1.0));
        }
        if mask & 8 != 0 {
            shapes.push((0.5, 0.5, 1.0, 1.0));
        }
        shapes
    };
    let shapes = match code {
        0x2580 => vec![(0.0, 0.0, 1.0, 0.5)],
        0x2581..=0x2588 => vec![(0.0, 1.0 - f64::from(code - 0x2580) / 8.0, 1.0, 1.0)],
        0x2589..=0x258F => vec![(0.0, 0.0, f64::from(0x2590 - code) / 8.0, 1.0)],
        0x2590 => vec![(0.5, 0.0, 1.0, 1.0)],
        0x2591..=0x2593 => {
            let alpha = [64, 128, 191][(code - 0x2591) as usize];
            return Some((vec![(0.0, 0.0, 1.0, 1.0)], alpha));
        }
        0x2594 => vec![(0.0, 0.0, 1.0, 0.125)],
        0x2595 => vec![(0.875, 0.0, 1.0, 1.0)],
        0x2596 => quadrant(4),
        0x2597 => quadrant(8),
        0x2598 => quadrant(1),
        0x2599 => quadrant(1 | 4 | 8),
        0x259A => quadrant(1 | 8),
        0x259B => quadrant(1 | 2 | 4),
        0x259C => quadrant(1 | 2 | 8),
        0x259D => quadrant(2),
        0x259E => quadrant(2 | 4),
        0x259F => quadrant(2 | 4 | 8),
        _ => return None,
    };
    Some((shapes, 255))
}
