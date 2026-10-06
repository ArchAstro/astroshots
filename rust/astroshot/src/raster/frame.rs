//! Terminal frame model: a grid of resolved cells.
//!
//! Port of the cell-resolution half of `terminal-html.ts` / `terminal-paint.ts`
//! (`cellStyle`, `terminalToHtml`, `terminalPlainText`, `ansiFrameToHtml`,
//! `createHeadlessTerminal`, `writeTerminal`), with `alacritty_terminal`
//! standing in for `@xterm/headless`.

use super::colors::{Rgb, palette_color};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

/// The emulator screen: an `alacritty_terminal` term with no event sink.
pub type Screen = Term<VoidListener>;

struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

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
        Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        Color::Indexed(index) => palette_color(index),
        Color::Named(named) => match named {
            NamedColor::Foreground | NamedColor::Background | NamedColor::Cursor => default,
            NamedColor::DimBlack => palette_color(0),
            NamedColor::DimRed => palette_color(1),
            NamedColor::DimGreen => palette_color(2),
            NamedColor::DimYellow => palette_color(3),
            NamedColor::DimBlue => palette_color(4),
            NamedColor::DimMagenta => palette_color(5),
            NamedColor::DimCyan => palette_color(6),
            NamedColor::DimWhite => palette_color(7),
            // BrightForeground / DimForeground: no palette entry, use the default.
            other => match u8::try_from(other as usize) {
                Ok(index) if index < 16 => palette_color(index),
                _ => default,
            },
        },
    }
}

/// Port of `cellStyle` for an `alacritty_terminal` cell. Dim (SGR 2), strike
/// (9) and hidden (8) come straight from the cell flags.
pub fn style_cell(cell: &Cell, default_foreground: Rgb, default_background: Rgb) -> StyledCell {
    let mut foreground = resolve(cell.fg, default_foreground);
    let mut background = resolve(cell.bg, default_background);
    if cell.flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut foreground, &mut background);
    }
    let mut text = String::new();
    text.push(cell.c);
    for &mark in cell.zerowidth().unwrap_or(&[]) {
        text.push(mark);
    }
    if text.chars().all(|c| c == '\0') {
        text = " ".to_string();
    }
    StyledCell {
        text,
        width: if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            0
        } else if cell.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        },
        foreground,
        background,
        bold: cell.flags.contains(Flags::BOLD),
        dim: cell.flags.contains(Flags::DIM),
        italic: cell.flags.contains(Flags::ITALIC),
        underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
        strike: cell.flags.contains(Flags::STRIKEOUT),
        invisible: cell.flags.contains(Flags::HIDDEN),
    }
}

fn grid_cell(screen: &Screen, row: u16, col: u16) -> Option<&Cell> {
    let grid = screen.grid();
    if usize::from(row) >= grid.screen_lines() || usize::from(col) >= grid.columns() {
        return None;
    }
    Some(&grid[Point::new(Line(i32::from(row)), Column(usize::from(col)))])
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
        screen: &Screen,
        cols: u16,
        rows: u16,
        default_foreground: Rgb,
        default_background: Rgb,
    ) -> Self {
        let mut frame = Self::blank(cols, rows, default_foreground, default_background);
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = grid_cell(screen, row, col) {
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
pub fn terminal_plain_text(screen: &Screen, rows: u16) -> String {
    let cols = u16::try_from(screen.columns()).unwrap_or(u16::MAX);
    let mut lines = Vec::with_capacity(usize::from(rows));
    for row in 0..rows {
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = grid_cell(screen, row, col)
                && !cell.flags.contains(Flags::WIDE_CHAR_SPACER)
            {
                let styled = style_cell(cell, [0; 3], [0; 3]);
                line.push_str(&styled.text);
            }
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n").trim_end().to_string()
}

/// Port of `createHeadlessTerminal` / `writeTerminal`: an `alacritty_terminal`
/// term with xterm's `convertEol: true` (a bare LF also returns the carriage)
/// and no scrollback.
pub struct HeadlessTerminal {
    term: Screen,
    processor: Processor,
}

impl HeadlessTerminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        let config = Config {
            scrolling_history: 0,
            ..Config::default()
        };
        let size = GridSize {
            cols: usize::from(cols.max(1)),
            rows: usize::from(rows.max(1)),
        };
        Self {
            term: Term::new(config, &size, VoidListener),
            processor: Processor::new(),
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
        self.processor.advance(&mut self.term, &converted);
    }

    pub fn screen(&self) -> &Screen {
        &self.term
    }

    pub fn plain_text(&self) -> String {
        let rows = u16::try_from(self.term.screen_lines()).unwrap_or(u16::MAX);
        terminal_plain_text(&self.term, rows)
    }

    pub fn frame(&self, default_foreground: Rgb, default_background: Rgb) -> TerminalFrame {
        let rows = u16::try_from(self.term.screen_lines()).unwrap_or(u16::MAX);
        let cols = u16::try_from(self.term.columns()).unwrap_or(u16::MAX);
        TerminalFrame::from_screen(
            &self.term,
            cols,
            rows,
            default_foreground,
            default_background,
        )
    }
}

/// Cursor position and visibility of a screen, for [`super::Cursor`].
pub(super) fn cursor_state(screen: &Screen) -> (u16, u16, bool) {
    let point = screen.grid().cursor.point;
    let row = u16::try_from(point.line.0.max(0)).unwrap_or(u16::MAX);
    let col = u16::try_from(point.column.0).unwrap_or(u16::MAX);
    (row, col, screen.mode().contains(TermMode::SHOW_CURSOR))
}
