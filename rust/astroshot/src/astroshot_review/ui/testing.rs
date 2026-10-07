//! `TestBackend` helpers shared by the UI component tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::{StatefulWidget, Widget};

/// Render `widget` into a `width` x `height` terminal and return the buffer.
pub fn render<W: Widget>(widget: W, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| frame.render_widget(widget, frame.area()))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Same as [`render`] for a `StatefulWidget`.
pub fn render_stateful<W: StatefulWidget>(
    widget: W,
    state: &mut W::State,
    width: u16,
    height: u16,
) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| frame.render_stateful_widget(widget, frame.area(), state))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Render an arbitrary closure (for inline spans placed in a `Line`).
pub fn draw(width: u16, height: u16, f: impl FnOnce(&mut Buffer, Rect)) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    f(&mut buf, Rect::new(0, 0, width, height));
    buf
}

/// Row `y` as text, trailing blanks kept (so centering is assertable).
pub fn row_raw(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

/// Row `y` as text with trailing blanks trimmed.
pub fn row(buf: &Buffer, y: u16) -> String {
    row_raw(buf, y).trim_end().to_string()
}

pub fn rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| row(buf, y)).collect()
}

pub fn fg_at(buf: &Buffer, x: u16, y: u16) -> Color {
    buf[(x, y)].fg
}

pub fn bg_at(buf: &Buffer, x: u16, y: u16) -> Color {
    buf[(x, y)].bg
}

pub fn modifier_at(buf: &Buffer, x: u16, y: u16) -> Modifier {
    buf[(x, y)].modifier
}
