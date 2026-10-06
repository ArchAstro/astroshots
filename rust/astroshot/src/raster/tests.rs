//! Cell-level and pixel-level tests. These replace `terminal-html.test.ts`,
//! whose assertions were on HTML strings (`color:#7c5cff`, escaped text, one
//! `tui-row` per row); the same facts are pinned on resolved cells.

use super::*;

const FG: Rgb = [0xff, 0xff, 0xff];
const BG: Rgb = [0x09, 0x0a, 0x12];

fn frame(ansi: &str, cols: u16, rows: u16) -> TerminalFrame {
    TerminalFrame::from_ansi(ansi.as_bytes(), cols, rows, FG, BG)
}

fn options(cols: u16, rows: u16) -> RasterOptions {
    let mut options = RasterOptions::new(cols, rows);
    options.foreground = [FG[0], FG[1], FG[2], 255];
    options
}

/// Cell origin in pixels for the default metrics at scale 2, padding 22:
/// origin 46, pitch 18, row height 39.6.
fn cell_origin(col: u32, row: u32) -> (u32, u32) {
    (46 + col * 18, (46.0 + row as f32 * 39.6).round() as u32)
}

/// Pixels of the content box (inside border and padding) for `cols x rows`.
fn content_pixels(image: &RgbaImage, cols: u32, rows: u32) -> Vec<(u32, u32)> {
    let height = (rows as f32 * 39.6).round() as u32;
    (46..46 + cols * 18)
        .flat_map(|x| (46..46 + height).map(move |y| (x, y)))
        .filter(|&(x, y)| x < image.width && y < image.height)
        .collect()
}

fn decode(png_bytes: &[u8]) -> (u32, u32) {
    let decoder = png::Decoder::new(png_bytes);
    let reader = decoder.read_info().unwrap();
    let info = reader.info();
    (info.width, info.height)
}

#[test]
fn preserves_terminal_colors_and_keeps_literal_text() {
    // terminal-html.test.ts: "preserves terminal colors and escapes fixture
    // content". HTML escaping has no equivalent; the text stays literal.
    let frame = frame("\u{1b}[38;2;124;92;255mPurple <frame>\u{1b}[0m", 24, 2);
    assert_eq!(frame.cell(0, 0).unwrap().foreground, [124, 92, 255]);
    assert_eq!(frame.cell(0, 13).unwrap().text, ">");
    assert_eq!(frame.cell(0, 13).unwrap().foreground, [124, 92, 255]);
    assert_eq!(frame.row_text(0), "Purple <frame>");
    // After the reset the rest of the row is default-colored.
    assert_eq!(frame.cell(0, 14).unwrap().foreground, FG);
}

#[test]
fn preserves_ink_style_truecolor_frames() {
    // terminal-html.test.ts: "preserves a real Ink component's truecolor
    // styling". Ink emits chalk truecolor SGR plus a trailing reset and
    // newline; the Ink render itself happens in the Node helper.
    let frame = frame("\u{1b}[38;2;124;92;255mBrand purple\u{1b}[39m\n", 24, 2);
    assert_eq!(frame.cell(0, 0).unwrap().foreground, [124, 92, 255]);
    assert_eq!(frame.row_text(0), "Brand purple");
}

#[test]
fn keeps_the_first_row_when_every_terminal_row_fills_the_exact_width() {
    let ansi = ["╭────────╮", "│ frame  │", "│ footer │", "╰────────╯"].join("\n");
    let frame = frame(&ansi, 10, 4);
    assert_eq!(frame.rows, 4);
    assert_eq!(frame.row_text(0), "╭────────╮");
    assert_eq!(frame.row_text(3), "╰────────╯");
    assert!((0..4).all(|row| frame.cell(row, 0).is_some()));
}

#[test]
fn palette_colors_follow_the_ts_tables() {
    assert_eq!(palette_color(0), [0x2b, 0x2f, 0x3a]);
    assert_eq!(palette_color(1), [0xf0, 0x72, 0x7a]);
    assert_eq!(palette_color(15), [0xff, 0xff, 0xff]);
    assert_eq!(palette_color(16), [0, 0, 0]);
    assert_eq!(palette_color(21), [0, 0, 255]);
    assert_eq!(palette_color(196), [255, 0, 0]);
    assert_eq!(palette_color(100), [135, 135, 0]);
    assert_eq!(palette_color(231), [255, 255, 255]);
    assert_eq!(palette_color(232), [8, 8, 8]);
    assert_eq!(palette_color(255), [238, 238, 238]);
    assert_eq!(to_hex(palette_color(1)), "#f0727a");
}

#[test]
fn resolves_palette_truecolor_and_defaults_per_cell() {
    let frame = frame(
        "\u{1b}[31ma\u{1b}[38;5;100mb\u{1b}[48;5;236mc\u{1b}[48;2;1;2;3md\u{1b}[0me",
        8,
        1,
    );
    let at = |col| frame.cell(0, col).unwrap();
    assert_eq!(at(0).foreground, palette_color(1));
    assert_eq!(at(0).background, BG);
    assert_eq!(at(1).foreground, [135, 135, 0]);
    assert_eq!(at(2).background, palette_color(236));
    assert_eq!(at(3).background, [1, 2, 3]);
    assert_eq!((at(4).foreground, at(4).background), (FG, BG));
    // Cells past the text are blanks in the default colors.
    assert_eq!(at(7).text, " ");
}

#[test]
fn applies_bold_italic_underline_and_inverse() {
    let frame = frame("\u{1b}[1;3;4mx\u{1b}[0m\u{1b}[7;31my\u{1b}[0mz", 4, 1);
    let x = frame.cell(0, 0).unwrap();
    assert!(x.bold && x.italic && x.underline);
    assert!(!x.dim && !x.strike && !x.invisible);
    let y = frame.cell(0, 1).unwrap();
    // Inverse swaps the resolved foreground and background.
    assert_eq!(y.foreground, BG);
    assert_eq!(y.background, palette_color(1));
    let z = frame.cell(0, 2).unwrap();
    assert!(!z.bold && !z.italic && !z.underline);
}

#[test]
fn inverse_of_default_colors_swaps_the_defaults() {
    let frame = frame("\u{1b}[7mx", 2, 1);
    let x = frame.cell(0, 0).unwrap();
    assert_eq!((x.foreground, x.background), (BG, FG));
}

#[test]
fn bare_line_feeds_return_the_carriage() {
    let frame = frame("ab\ncd", 6, 3);
    assert_eq!(frame.row_text(0), "ab");
    assert_eq!(frame.row_text(1), "cd");
    assert_eq!(frame.cell(1, 0).unwrap().text, "c");
}

#[test]
fn wide_cells_skip_their_trailing_half() {
    let frame = frame("界x", 4, 1);
    assert_eq!(frame.cell(0, 0).unwrap().width, 2);
    assert_eq!(frame.cell(0, 1).unwrap().width, 0);
    assert_eq!(frame.cell(0, 2).unwrap().text, "x");
    assert_eq!(frame.row_text(0), "界x");
}

#[test]
fn plain_text_trims_rows_and_trailing_blank_lines() {
    let mut terminal = HeadlessTerminal::new(10, 4);
    terminal.write(b"one  \ntwo\n");
    assert_eq!(terminal.plain_text(), "one\ntwo");
    assert_eq!(terminal_plain_text(terminal.screen(), 1), "one");
}

#[test]
fn sizes_follow_the_css_box_formula() {
    let defaults = RasterOptions::new(100, 30);
    // ceil(100 * 15 * .62 + 44), ceil(30 * 15 * 1.32 + 44)
    assert_eq!(defaults.css_size(), (974, 638));
    assert_eq!(defaults.pixel_size(), (1948, 1276));
    let movie = RasterOptions::movie(80, 24);
    assert_eq!(
        movie.css_size(),
        (
            ((80.0f32 * 14.0 * 0.62) + 32.0).ceil() as u32,
            ((24.0f32 * 14.0 * 1.35) + 32.0).ceil() as u32
        )
    );
    let mut one = RasterOptions::new(100, 30);
    one.scale = 1.0;
    assert_eq!(one.pixel_size(), (974, 638));
}

#[test]
fn png_dimensions_match_the_element_screenshot_size() {
    let options = options(24, 2);
    let bytes = render_png(&frame("hi", 24, 2), &options).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(decode(&bytes), options.pixel_size());
    let mut one = options.clone();
    one.scale = 1.0;
    let bytes = render_png(&frame("hi", 24, 2), &one).unwrap();
    assert_eq!(decode(&bytes), one.pixel_size());
}

#[test]
fn output_is_byte_identical_for_identical_input() {
    let ansi = "\u{1b}[1;38;2;124;92;255mBold\u{1b}[0m \u{1b}[3mitalic\u{1b}[0m \u{1b}[4munder\u{1b}[0m █▀▄ 界\n\u{1b}[48;5;236mbg\u{1b}[0m ╭──╮";
    let options = options(30, 3);
    let first = render_png(&frame(ansi, 30, 3), &options).unwrap();
    let second = render_png(&frame(ansi, 30, 3), &options).unwrap();
    assert_eq!(first, second);
    // A fresh rasterizer (cold glyph cache, fresh font system) agrees.
    let mut cold = Rasterizer::new();
    assert_eq!(
        first,
        cold.render_png(&frame(ansi, 30, 3), &options).unwrap()
    );
}

#[test]
fn box_corners_are_transparent_and_interior_is_the_background() {
    let options = options(24, 2);
    let image = render_rgba(&frame("", 24, 2), &options).unwrap();
    assert_eq!(image.pixel(0, 0)[3], 0, "rounded corner is transparent");
    let (width, _) = options.pixel_size();
    assert_eq!(image.pixel(width / 2, 30), [0x09, 0x0a, 0x12, 255]);
    // The 1px (2 device px) border is the border color over the background.
    let border = image.pixel(width / 2, 0);
    assert_eq!(border[3], 255);
    assert_ne!(border, [0x09, 0x0a, 0x12, 255]);
    assert!(border[2] > 0x12, "border tints toward rgb(185,168,255)");
}

#[test]
fn cell_backgrounds_land_on_their_grid_position() {
    let options = options(24, 2);
    let image = render_rgba(
        &frame("\u{1b}[48;2;255;0;0m \u{1b}[48;2;0;255;0m ", 24, 2),
        &options,
    )
    .unwrap();
    let (x0, y0) = cell_origin(0, 0);
    assert_eq!(image.pixel(x0 + 2, y0 + 2), [255, 0, 0, 255]);
    assert_eq!(image.pixel(x0 + 16, y0 + 30), [255, 0, 0, 255]);
    let (x1, _) = cell_origin(1, 0);
    assert_eq!(image.pixel(x1 + 2, y0 + 2), [0, 255, 0, 255]);
    let (x2, _) = cell_origin(2, 0);
    assert_eq!(image.pixel(x2 + 2, y0 + 2), [0x09, 0x0a, 0x12, 255]);
    // Second row starts below the first.
    let (_, y_row1) = cell_origin(0, 1);
    assert_eq!(image.pixel(x0 + 2, y_row1 + 2), [0x09, 0x0a, 0x12, 255]);
}

#[test]
fn full_block_paints_the_foreground_exactly() {
    let options = options(24, 2);
    let image = render_rgba(&frame("\u{1b}[38;2;124;92;255m█▀", 24, 2), &options).unwrap();
    let (x0, y0) = cell_origin(0, 0);
    assert_eq!(image.pixel(x0 + 9, y0 + 20), [124, 92, 255, 255]);
    // Upper half block: top is foreground, bottom is background.
    let (x1, _) = cell_origin(1, 0);
    assert_eq!(image.pixel(x1 + 9, y0 + 5), [124, 92, 255, 255]);
    assert_eq!(image.pixel(x1 + 9, y0 + 35), [0x09, 0x0a, 0x12, 255]);
}

#[test]
fn glyphs_are_drawn_in_the_cell_foreground_inside_their_cell() {
    let options = options(24, 2);
    let image = render_rgba(&frame("\u{1b}[38;2;255;0;0mM", 24, 2), &options).unwrap();
    let (x0, y0) = cell_origin(0, 0);
    let (x1, y1) = (x0 + 18, y0 + 40);
    let mut inked = 0;
    let mut strongest = 0u8;
    for (x, y) in content_pixels(&image, 24, 2) {
        let [r, g, b, _] = image.pixel(x, y);
        if (r, g, b) != (0x09, 0x0a, 0x12) {
            inked += 1;
            strongest = strongest.max(r);
            assert!(
                x >= x0 - 1 && x <= x1 && y >= y0 - 1 && y <= y1,
                "ink outside cell at {x},{y}"
            );
            // Red text over the dark background never gains green.
            assert!(g <= 0x0a + 1, "unexpected green {g}");
        }
    }
    assert!(inked > 40, "an M inks many pixels, got {inked}");
    assert_eq!(strongest, 255, "glyph cores reach the full foreground");
}

#[test]
fn spaces_draw_nothing_and_bold_inks_more_than_regular() {
    let options = options(8, 1);
    let count = |ansi: &str| {
        let image = render_rgba(&frame(ansi, 8, 1), &options).unwrap();
        content_pixels(&image, 8, 1)
            .into_iter()
            .filter(|&(x, y)| image.pixel(x, y) != [0x09, 0x0a, 0x12, 255])
            .map(|(x, y)| u64::from(image.pixel(x, y)[0]))
            .sum::<u64>()
    };
    assert_eq!(count("   "), 0);
    assert!(count("\u{1b}[1mHHH") > count("HHH"));
}

#[test]
fn underline_is_a_line_across_the_cell() {
    let options = options(8, 1);
    let plain = render_rgba(&frame(" ", 8, 1), &options).unwrap();
    let underlined = render_rgba(&frame("\u{1b}[4;38;2;0;255;0m ", 8, 1), &options).unwrap();
    let (x0, y0) = cell_origin(0, 0);
    let rows_with_green: Vec<u32> = (y0..y0 + 40)
        .filter(|&y| underlined.pixel(x0 + 9, y) == [0, 255, 0, 255])
        .collect();
    assert!(!rows_with_green.is_empty() && rows_with_green.len() <= 3);
    assert!((y0..y0 + 40).all(|y| plain.pixel(x0 + 9, y) == [0x09, 0x0a, 0x12, 255]));
    // The line spans the full cell width.
    let y = rows_with_green[0];
    assert_eq!(underlined.pixel(x0, y), [0, 255, 0, 255]);
    assert_eq!(underlined.pixel(x0 + 17, y), [0, 255, 0, 255]);
}

#[test]
fn dim_fades_the_cell_over_the_box_background() {
    let options = options(8, 1);
    let mut cells = frame("\u{1b}[48;2;200;100;50m ", 8, 1);
    cells.cell_mut(0, 0).unwrap().dim = true;
    let image = render_rgba(&cells, &options).unwrap();
    let (x0, y0) = cell_origin(0, 0);
    // 62% of (200,100,50) over (9,10,18).
    let expected = [
        (200.0f32 * 0.62 + 9.0 * 0.38).round() as u8,
        (100.0f32 * 0.62 + 10.0 * 0.38).round() as u8,
        (50.0f32 * 0.62 + 18.0 * 0.38).round() as u8,
        255,
    ];
    assert_eq!(image.pixel(x0 + 2, y0 + 2), expected);
}

#[test]
fn invisible_text_is_drawn_in_the_background_color() {
    let mut cells = frame("\u{1b}[38;2;255;0;0mM", 8, 1);
    assert_eq!(cells.cell(0, 0).unwrap().text_color(), [255, 0, 0]);
    cells.cell_mut(0, 0).unwrap().invisible = true;
    assert_eq!(cells.cell(0, 0).unwrap().text_color(), BG);
    let image = render_rgba(&cells, &options(8, 1)).unwrap();
    let hidden = content_pixels(&image, 8, 1)
        .into_iter()
        .all(|(x, y)| image.pixel(x, y) == [0x09, 0x0a, 0x12, 255]);
    assert!(hidden);
}

#[test]
fn cursor_draws_an_inverted_block() {
    let mut with_cursor = options(8, 1);
    with_cursor.cursor = Some(Cursor { row: 0, col: 2 });
    let image = render_rgba(&frame("", 8, 1), &with_cursor).unwrap();
    let (x2, y0) = cell_origin(2, 0);
    assert_eq!(image.pixel(x2 + 2, y0 + 2), [0xff, 0xff, 0xff, 255]);
    let (x3, _) = cell_origin(3, 0);
    assert_eq!(image.pixel(x3 + 2, y0 + 2), [0x09, 0x0a, 0x12, 255]);
    // The screen's cursor position is exposed for callers.
    let mut terminal = HeadlessTerminal::new(8, 1);
    terminal.write(b"ab");
    assert_eq!(
        Cursor::from_screen(terminal.screen()),
        Some(Cursor { row: 0, col: 2 })
    );
    terminal.write(b"\x1b[?25l");
    assert_eq!(Cursor::from_screen(terminal.screen()), None);
}

#[test]
fn overlays_are_stretched_over_the_cell_grid() {
    // A solid green 2x2 PNG covering cells (1..3, row 0).
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[0, 255, 0, 255].repeat(4))
            .unwrap();
    }
    let mut options = options(8, 2);
    options
        .overlays
        .push(Overlay::from_png(1.0, 0.0, 2.0, 1.0, &png_bytes).unwrap());
    let image = render_rgba(&frame("", 8, 2), &options).unwrap();
    let (x1, y0) = cell_origin(1, 0);
    assert_eq!(image.pixel(x1 + 10, y0 + 10), [0, 255, 0, 255]);
    assert_eq!(image.pixel(x1 + 30, y0 + 10), [0, 255, 0, 255]);
    let (x3, _) = cell_origin(3, 0);
    assert_eq!(image.pixel(x3 + 4, y0 + 10), [0x09, 0x0a, 0x12, 255]);
    assert!(Overlay::from_png(0.0, 0.0, 1.0, 1.0, b"not a png").is_err());
}

#[test]
fn invalid_options_are_rejected() {
    let cells = frame("", 4, 1);
    let mut bad = options(4, 1);
    bad.scale = 0.0;
    assert!(matches!(
        render_png(&cells, &bad),
        Err(RasterError::InvalidOption(_))
    ));
    let mut bad = options(4, 1);
    bad.padding = 5_000.0;
    assert!(render_png(&cells, &bad).is_err());
}

#[test]
fn parses_css_colors() {
    assert_eq!(parse_css_color("#090a12"), Some([9, 10, 18, 255]));
    assert_eq!(parse_css_color("#fff"), Some([255, 255, 255, 255]));
    assert_eq!(parse_css_color("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
    assert_eq!(parse_css_color("rgb(1, 2, 3)"), Some([1, 2, 3, 255]));
    assert_eq!(
        parse_css_color("rgba(255, 0, 0, .5)"),
        Some([255, 0, 0, 128])
    );
    assert_eq!(parse_css_color("transparent"), Some([0, 0, 0, 0]));
    assert_eq!(parse_css_color("papayawhip"), None);
    assert_eq!(parse_css_color("#12"), None);
    let opts = RasterOptions::new(1, 1)
        .with_css_colors("#fff", "#000")
        .unwrap();
    assert_eq!(opts.background, [0, 0, 0, 255]);
    assert!(
        RasterOptions::new(1, 1)
            .with_css_colors("nope", "#000")
            .is_err()
    );
}

#[test]
fn render_ansi_png_matches_frame_then_render() {
    let options = options(24, 2);
    let ansi = "\u{1b}[38;2;124;92;255mPurple <frame>\u{1b}[0m";
    let direct = render_ansi_png(ansi.as_bytes(), &options).unwrap();
    let staged = render_png(&frame(ansi, 24, 2), &options).unwrap();
    assert_eq!(direct, staged);
    let image = RgbaImage {
        width: 1,
        height: 1,
        data: vec![1, 2, 3, 4],
    };
    assert!(encode_png(&image).unwrap().starts_with(b"\x89PNG"));
}

#[test]
fn dim_hidden_and_strike_come_through_from_real_ansi() {
    let frame = TerminalFrame::from_ansi(
        b"\x1b[2mD\x1b[0m\x1b[8mH\x1b[0m\x1b[9mS\x1b[0mN",
        6,
        1,
        [0xe8, 0xe8, 0xf2],
        [0x09, 0x0a, 0x12],
    );
    let dim = frame.cell(0, 0).unwrap();
    assert!(dim.dim && !dim.strike && !dim.invisible);
    let hidden = frame.cell(0, 1).unwrap();
    assert!(hidden.invisible && !hidden.dim);
    assert_eq!(hidden.text_color(), hidden.background);
    let strike = frame.cell(0, 2).unwrap();
    assert!(strike.strike && !strike.dim);
    let plain = frame.cell(0, 3).unwrap();
    assert!(!plain.dim && !plain.strike && !plain.invisible);
}

#[test]
fn autowrap_off_overwrites_the_last_column() {
    // from_ansi sends ESC[?7l first, so a long row stays on one line.
    let frame = TerminalFrame::from_ansi(
        b"abcdefgh\nnext",
        4,
        2,
        [0xe8, 0xe8, 0xf2],
        [0x09, 0x0a, 0x12],
    );
    assert_eq!(frame.row_text(0), "abch");
    assert_eq!(frame.row_text(1), "next");
    // With autowrap on (plain terminal) the row wraps.
    let mut wrapped = HeadlessTerminal::new(4, 2);
    wrapped.write(b"abcdefgh");
    assert_eq!(wrapped.plain_text(), "abcd\nefgh");
}
