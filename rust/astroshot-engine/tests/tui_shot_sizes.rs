//! Terminal PNG sizes against the TS implementation.
//!
//! `tests/fixtures/terminal_sizes.json` holds one row per real TS capture:
//! `captureTerminalHtml` (packages/tui-shot/src/shot.ts) laid the
//! `[data-tui-shot]` box out in Chromium and screenshotted it; `css` is the
//! box size the TS computed and `png` the size read from the PNG header.
//! `tests/fixtures/terminal_sizes.gen.mjs` regenerates it. The table covers
//! rows 1..=100 at the tui-shot defaults (15px / 1.32 / padding 22), the
//! `interactive-pty.yaml` grid at scale 1, the movie PTY defaults (14px /
//! 1.35 / padding 16) and the truecolor-demo font (16px), columns 1..=300,
//! and fractional scales and font metrics.

use astroshot_engine::raster::{RasterOptions, TerminalFrame, render_png};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SizeCase {
    cols: u16,
    rows: u16,
    font_size: f64,
    line_height: f64,
    padding: f64,
    scale: f64,
    css: (u32, u32),
    png: (u32, u32),
}

fn table() -> Vec<SizeCase> {
    serde_json::from_str(include_str!("fixtures/terminal_sizes.json")).unwrap()
}

fn options(case: &SizeCase) -> RasterOptions {
    let mut options = RasterOptions::new(case.cols, case.rows);
    options.font_size = case.font_size;
    options.line_height = case.line_height;
    options.padding = case.padding;
    options.scale = case.scale;
    options
}

#[test]
fn css_and_png_sizes_match_every_ts_capture() {
    let table = table();
    // Rows 1..=100 at the tui-shot defaults and at the movie defaults.
    for (font_size, line_height) in [(15.0, 1.32), (14.0, 1.35)] {
        for rows in 1..=100u16 {
            assert!(
                table.iter().any(|case| case.rows == rows
                    && case.font_size == font_size
                    && case.line_height == line_height),
                "fixture has no {font_size}px/{line_height} case with {rows} rows"
            );
        }
    }
    let mut wrong = Vec::new();
    for case in &table {
        let options = options(case);
        if options.css_size() != case.css || options.pixel_size() != case.png {
            wrong.push(format!(
                "{case:?}: css {:?} png {:?}",
                options.css_size(),
                options.pixel_size()
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} sizes differ from TS:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn rendered_png_headers_carry_the_ts_sizes() {
    // Rendering every case is slow; the row counts where f32 arithmetic was
    // off by a pixel, plus the fractional scales, go through the rasterizer.
    let f32_was_wrong = [5, 10, 20, 25, 40, 45, 50, 80, 85, 90, 95, 100];
    let mut rendered = 0;
    for case in table() {
        let fractional = case.scale.fract() != 0.0;
        let default_width = case.cols == 52 || case.cols == 80;
        if !(fractional || default_width && f32_was_wrong.contains(&case.rows)) {
            continue;
        }
        let options = options(&case);
        let frame = TerminalFrame::from_ansi(
            b"size",
            case.cols,
            case.rows,
            options.foreground_rgb(),
            options.background_rgb(),
        );
        let png = render_png(&frame, &options).unwrap();
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((width, height), case.png, "{case:?}");
        rendered += 1;
    }
    assert_eq!(rendered, 24 + 50);
}
