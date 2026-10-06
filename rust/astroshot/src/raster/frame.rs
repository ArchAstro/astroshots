//! Terminal frame model: a grid of resolved cells.
//!
//! Port of the cell-resolution half of `terminal-html.ts` / `terminal-paint.ts`
//! (`cellStyle`, `terminalToHtml`, `terminalPlainText`, `ansiFrameToHtml`,
//! `createHeadlessTerminal`, `writeTerminal`), with `vt100` standing in for
//! `@xterm/headless`.

use super::colors::{Rgb, palette_color};
use vt100::Color;

/// One resolved terminal cell: text plus the computed style the HTML path put
/// in the `<span style=...>` attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledCell {
    /// Grapheme cluster, `" "` for an empty cell (`cell.getChars() || " "`).
    pub text: String,
    /// 1 for a normal cell, 2 for a wide cell, 0 for the trailing half of a
    /// wide cell (the TS skips `getWidth() === 0` cells).
    pub width: u8,
    /// Foreground after the inverse swap. Not replaced for `invisible`; the
    /// painter does that, as `styleAttribute` did.
    pub foreground: Rgb,
    /// Background after the inverse swap.
    pub background: Rgb,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub invisible: bool,
}

impl StyledCell {
    /// The cell used for positions outside the screen: a blank in the
    /// default colors (the TS `cell ? cellStyle(...) : {...defaults}` branch).
    pub fn blank(foreground: Rgb, background: Rgb) -> Self {
        Self {
            text: " ".to_string(),
            width: 1,
            foreground,
            background,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            strike: false,
            invisible: false,
        }
    }

    /// Color the glyph is drawn in: `color:${invisible ? background : foreground}`.
    pub fn text_color(&self) -> Rgb {
        if self.invisible {
            self.background
        } else {
            self.foreground
        }
    }
}

fn resolve(color: Color, default: Rgb) -> Rgb {
    match color {
        Color::Default => default,
        Color::Idx(index) => palette_color(index),
        Color::Rgb(r, g, b) => [r, g, b],
    }
}

/// Port of `cellStyle` for a `vt100` cell.
///
/// `vt100` 0.15 tracks bold, italic, underline and inverse only. `dim`,
/// `strike` and `invisible` (SGR 2, 9, 8) are never set by the parser; the
/// fields stay so other frame sources and tests can use them.
pub fn style_cell(
    cell: &vt100::Cell,
    default_foreground: Rgb,
    default_background: Rgb,
) -> StyledCell {
    let mut foreground = resolve(cell.fgcolor(), default_foreground);
    let mut background = resolve(cell.bgcolor(), default_background);
    if cell.inverse() {
        std::mem::swap(&mut foreground, &mut background);
    }
    let contents = cell.contents();
    StyledCell {
        text: if contents.is_empty() {
            " ".to_string()
        } else {
            contents
        },
        width: if cell.is_wide_continuation() {
            0
        } else if cell.is_wide() {
            2
        } else {
            1
        },
        foreground,
        background,
        bold: cell.bold(),
        dim: false,
        italic: cell.italic(),
        underline: cell.underline(),
        strike: false,
        invisible: false,
    }
}

/// A `cols x rows` grid of resolved cells, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalFrame {
    pub cols: u16,
    pub rows: u16,
    pub default_foreground: Rgb,
    pub default_background: Rgb,
    cells: Vec<StyledCell>,
}

impl TerminalFrame {
    /// Blank frame in the default colors.
    pub fn blank(cols: u16, rows: u16, default_foreground: Rgb, default_background: Rgb) -> Self {
        Self {
            cols,
            rows,
            default_foreground,
            default_background,
            cells: vec![
                StyledCell::blank(default_foreground, default_background);
                usize::from(cols) * usize::from(rows)
            ],
        }
    }

    /// Port of `terminalToHtml`'s cell walk: the visible `rows x cols` window
    /// of the screen (xterm `viewportY` is always 0 with `scrollback: 0`).
    pub fn from_screen(
        screen: &vt100::Screen,
        cols: u16,
        rows: u16,
        default_foreground: Rgb,
        default_background: Rgb,
    ) -> Self {
        let mut frame = Self::blank(cols, rows, default_foreground, default_background);
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = screen.cell(row, col) {
                    let index = frame.index(row, col);
                    frame.cells[index] = style_cell(cell, default_foreground, default_background);
                }
            }
        }
        frame
    }

    /// Port of `ansiFrameToHtml`: interpret a real ANSI frame at `cols x rows`.
    pub fn from_ansi(
        ansi: &[u8],
        cols: u16,
        rows: u16,
        default_foreground: Rgb,
        default_background: Rgb,
    ) -> Self {
        let mut terminal = HeadlessTerminal::new(cols, rows);
        terminal.write(b"\x1b[?7l");
        terminal.write(ansi);
        terminal.frame(default_foreground, default_background)
    }

    fn index(&self, row: u16, col: u16) -> usize {
        usize::from(row) * usize::from(self.cols) + usize::from(col)
    }

    pub fn cell(&self, row: u16, col: u16) -> Option<&StyledCell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.cells.get(self.index(row, col))
    }

    pub fn cell_mut(&mut self, row: u16, col: u16) -> Option<&mut StyledCell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let index = self.index(row, col);
        self.cells.get_mut(index)
    }

    /// Text of one row without trailing spaces (`translateToString(true)`),
    /// skipping the trailing half of wide cells.
    pub fn row_text(&self, row: u16) -> String {
        let mut text = String::new();
        for col in 0..self.cols {
            if let Some(cell) = self.cell(row, col)
                && cell.width != 0
            {
                text.push_str(&cell.text);
            }
        }
        text.trim_end().to_string()
    }
}

/// Port of `terminalPlainText`: rows joined by `\n`, trailing whitespace
/// trimmed.
pub fn terminal_plain_text(screen: &vt100::Screen, rows: u16) -> String {
    let (_, cols) = screen.size();
    let mut lines = Vec::with_capacity(usize::from(rows));
    for row in 0..rows {
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col)
                && !cell.is_wide_continuation()
            {
                let contents = cell.contents();
                line.push_str(if contents.is_empty() { " " } else { &contents });
            }
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n").trim_end().to_string()
}

/// Port of `createHeadlessTerminal` / `writeTerminal`: a `vt100` parser with
/// xterm's `convertEol: true` (a bare LF also returns the carriage) and no
/// scrollback.
pub struct HeadlessTerminal {
    parser: vt100::Parser,
}

impl HeadlessTerminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 0),
        }
    }

    /// Feed bytes; every `\n` also returns the carriage (`convertEol`). An
    /// extra `\r` before an existing `\r\n` changes nothing.
    pub fn write(&mut self, data: &[u8]) {
        let mut converted = Vec::with_capacity(data.len() + 8);
        for &byte in data {
            if byte == b'\n' {
                converted.push(b'\r');
            }
            converted.push(byte);
        }
        self.parser.process(&converted);
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Cursor `(col, row)` in the visible window (xterm `cursorX` / `cursorY`).
    pub fn cursor_position(&self) -> (u16, u16) {
        let (row, col) = self.screen().cursor_position();
        (col, row)
    }

    pub fn plain_text(&self) -> String {
        let (rows, _) = self.screen().size();
        terminal_plain_text(self.screen(), rows)
    }

    pub fn frame(&self, default_foreground: Rgb, default_background: Rgb) -> TerminalFrame {
        let (rows, cols) = self.screen().size();
        TerminalFrame::from_screen(
            self.screen(),
            cols,
            rows,
            default_foreground,
            default_background,
        )
    }
}
