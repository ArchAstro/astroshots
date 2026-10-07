//! Cell colors and glyphs of real Ink output, through the Node helper and the
//! native rasterizer. Brings the weaker ports of
//! `packages/tui-shot/src/terminal-html.test.ts` (and the truecolor case of
//! `packages/movie-harness/src/session.test.ts`) up to the originals, which
//! rendered a real Ink component with chalk truecolor and asserted the HTML
//! (`color:#7c5cff`, the text, the escaped `<`/`>`). There is no HTML here;
//! the same facts are asserted on the cells the rasterizer paints from, and
//! on the pixels.
//!
//! The Ink cases need `node` >=22 and the workspace `node_modules`; they are
//! skipped without Node.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use astroshot::node_helper::{NodeHelper, find_node};
use astroshot::raster::{
    HeadlessTerminal, RasterOptions, TerminalFrame, parse_css_color, render_rgba,
};

const PURPLE: [u8; 3] = [0x7c, 0x5c, 0xff];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const BACKGROUND: [u8; 3] = [0x09, 0x0a, 0x12];

fn own_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ink")
        .join(name)
}

/// Pixels inside the cell box of `(row, col)` for the tui-shot defaults at
/// scale 2: origin (1 + 22) * 2, pitch 0.6 * 30, row height 1.32 * 30.
fn cell_pixels(row: u32, col: u32) -> impl Iterator<Item = (u32, u32)> {
    let top = (46.0 + f64::from(row) * 39.6).round() as u32;
    let bottom = (46.0 + f64::from(row + 1) * 39.6).round() as u32;
    (46 + col * 18..46 + (col + 1) * 18).flat_map(move |x| (top..bottom).map(move |y| (x, y)))
}

#[tokio::test]
async fn preserves_a_real_ink_components_truecolor_styling() {
    if let Err(error) = find_node() {
        common::skip(&error);
        return;
    }
    // Real Ink and chalk render the fixture in Node; the helper raises
    // chalk to truecolor for the render, as `renderInkFrame` does.
    let mut helper = NodeHelper::spawn()
        .await
        .expect("node >=22 and rust/node-helper/helper.mjs")
        .with_request_timeout(Duration::from_secs(90));
    let reply = helper
        .ink_render(&own_fixture("truecolor.tsx"), None, None)
        .await
        .unwrap();
    helper.shutdown().await.unwrap();
    assert_eq!((reply.cols, reply.rows), (24, 4));
    assert!(
        reply.ansi.contains("\u{1b}[38;2;124;92;255m"),
        "{:?}",
        reply.ansi
    );

    // The fixture's own colors: #ffffff on #090a12, as in the TS test.
    let mut options = RasterOptions::new(24, 4);
    options.foreground = parse_css_color(reply.foreground.as_deref().unwrap()).unwrap();
    options.background = parse_css_color(reply.background.as_deref().unwrap()).unwrap();
    let frame = TerminalFrame::from_ansi(
        reply.ansi.as_bytes(),
        24,
        4,
        options.foreground_rgb(),
        options.background_rgb(),
    );

    // `color:#7c5cff`: every border cell is truecolor purple on the default
    // background, and nothing else is.
    let horizontal = "─".repeat(22);
    assert_eq!(frame.row_text(0), format!("╭{horizontal}╮"));
    assert_eq!(frame.row_text(3), format!("╰{horizontal}╯"));
    let literal = "<a> & \"b\" 'c'";
    assert_eq!(frame.row_text(1), format!("│{:<22}│", "Brand purple"));
    assert_eq!(frame.row_text(2), format!("│{literal:<22}│"));
    for row in 0..4u16 {
        for col in 0..24u16 {
            let cell = frame.cell(row, col).unwrap();
            let border = row == 0 || row == 3 || col == 0 || col == 23;
            assert_eq!(cell.background, BACKGROUND, "cell {row},{col}");
            if border {
                assert_eq!(cell.foreground, PURPLE, "border cell {row},{col}");
            } else if row == 2 {
                assert_eq!(cell.foreground, WHITE, "cell {row},{col}");
            }
        }
    }

    // The characters the HTML path escaped (`&lt;`, `&gt;`, `&amp;`,
    // `&quot;`) are those glyph cells, one character each.
    for (index, character) in literal.chars().enumerate() {
        let cell = frame.cell(2, index as u16 + 1).unwrap();
        assert_eq!(cell.text, character.to_string(), "column {index}");
    }

    // Pixels: border cells are painted in exactly #7c5cff and never in the
    // default white; the literal row is painted white and never purple.
    let image = render_rgba(&frame, &options).unwrap();
    let count = |row: u32, col: u32, color: [u8; 3]| {
        cell_pixels(row, col)
            .filter(|&(x, y)| image.pixel(x, y)[..3] == color)
            .count()
    };
    for (row, col) in
        (0..24)
            .flat_map(|col| [(0, col), (3, col)])
            .chain([(1, 0), (1, 23), (2, 0), (2, 23)])
    {
        assert!(count(row, col, PURPLE) > 0, "border cell {row},{col}");
        assert_eq!(count(row, col, WHITE), 0, "border cell {row},{col}");
    }
    for (index, character) in literal.chars().enumerate() {
        let col = index as u32 + 1;
        if character == ' ' {
            assert_eq!(count(2, col, WHITE), 0, "blank column {col}");
            continue;
        }
        assert!(count(2, col, WHITE) > 0, "glyph {character:?} is drawn");
        assert_eq!(count(2, col, PURPLE), 0, "column {col}");
    }
}

/// terminal-html.test.ts "preserves terminal colors and escapes fixture
/// content": `<`, `>`, `&` and quotes are ordinary glyph cells, each drawn
/// with a different shape, in the SGR color.
#[test]
fn html_special_characters_render_as_their_own_glyph_cells() {
    // The five characters, then `<` and `>` again.
    let text = "<>&\"'<>";
    let ansi = format!("\u{1b}[38;2;124;92;255mPurple <frame>\u{1b}[0m\n{text}");
    let mut options = RasterOptions::new(24, 2);
    options.foreground = [0xff, 0xff, 0xff, 255];
    let frame = TerminalFrame::from_ansi(ansi.as_bytes(), 24, 2, WHITE, BACKGROUND);

    assert_eq!(frame.row_text(0), "Purple <frame>");
    for col in 0..14 {
        assert_eq!(
            frame.cell(0, col).unwrap().foreground,
            PURPLE,
            "column {col}"
        );
    }
    assert_eq!(frame.cell(0, 7).unwrap().text, "<");
    assert_eq!(frame.cell(0, 13).unwrap().text, ">");
    assert_eq!(frame.cell(0, 14).unwrap().foreground, WHITE);
    assert_eq!(frame.row_text(1), text);
    for (col, character) in text.chars().enumerate() {
        assert_eq!(
            frame.cell(1, col as u16).unwrap().text,
            character.to_string()
        );
    }

    let image = render_rgba(&frame, &options).unwrap();
    let ink = |row: u32, col: u32| -> Vec<bool> {
        cell_pixels(row, col)
            .map(|(x, y)| image.pixel(x, y)[..3] != BACKGROUND)
            .collect()
    };
    let shapes: Vec<Vec<bool>> = (0..7).map(|col| ink(1, col)).collect();
    for (index, shape) in shapes[..5].iter().enumerate() {
        assert!(shape.iter().any(|&on| on), "glyph {index} is drawn");
        for (offset, other) in shapes[index + 1..5].iter().enumerate() {
            assert!(
                shape != other,
                "glyphs {index} and {} differ",
                index + 1 + offset
            );
        }
    }
    // The same character draws the same glyph wherever it is on the row.
    assert!(shapes[5] == shapes[0], "both `<` cells match");
    assert!(shapes[6] == shapes[1], "both `>` cells match");
    // The purple `<` and `>` of the first row are drawn, in purple.
    for col in [7, 13] {
        let purple = cell_pixels(0, col)
            .filter(|&(x, y)| image.pixel(x, y)[..3] == PURPLE)
            .count();
        assert!(purple > 0, "column {col}");
    }
}

/// movie-harness session.test.ts "preserves SGR truecolor in HTML": the HTML
/// held one `color:#7c5cff` run with the text "Purple frame" on the default
/// background, and default-colored cells after it.
#[test]
fn movie_terminal_keeps_sgr_truecolor_on_every_cell_of_the_run() {
    let mut terminal = HeadlessTerminal::new(24, 2);
    terminal.write(b"\x1b[38;2;124;92;255mPurple frame\x1b[0m");
    let frame = terminal.frame(WHITE, BACKGROUND);
    assert_eq!(frame.row_text(0), "Purple frame");
    for col in 0..12 {
        let cell = frame.cell(0, col).unwrap();
        assert_eq!(
            (cell.foreground, cell.background),
            (PURPLE, BACKGROUND),
            "column {col}"
        );
    }
    for col in 12..24 {
        let cell = frame.cell(0, col).unwrap();
        assert_eq!(
            (cell.foreground, cell.background),
            (WHITE, BACKGROUND),
            "column {col}"
        );
    }
}
