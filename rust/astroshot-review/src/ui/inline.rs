//! Row helpers shared by the stream and friction ports. They stand for three
//! Ink constructs that `chrome` and `ui::put_spans` do not cover:
//!
//! - `<Box height={1} justifyContent="space-between">` -> [`space_between`]
//! - `<Text wrap="truncate">` with nested styled runs -> [`put_truncated`]
//! - `<Box backgroundColor>` -> [`fill_bg`]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use super::put_spans;
use super::text_width;

/// Total terminal columns of `spans`.
pub(crate) fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// Write `spans` at (`x`, `y`) in at most `max` columns. Text that does not fit
/// is cut and ended with `…` in the style of the span that was cut (Ink
/// `wrap="truncate"`).
pub(crate) fn put_truncated(
    buf: &mut Buffer,
    area: Rect,
    x: u16,
    y: u16,
    spans: &[Span<'_>],
    max: usize,
) -> u16 {
    if spans_width(spans) <= max {
        return put_spans(buf, area, x, y, spans);
    }
    if max == 0 {
        return x;
    }
    let mut budget = max - 1;
    let mut kept: Vec<Span<'static>> = Vec::new();
    let mut ellipsis = Style::new();
    for span in spans {
        ellipsis = span.style;
        let width = span.width();
        if width <= budget {
            kept.push(Span::styled(span.content.to_string(), span.style));
            budget -= width;
            continue;
        }
        let mut cut = String::new();
        let mut used = 0;
        for c in span.content.chars() {
            let w = text_width(c.encode_utf8(&mut [0; 4]));
            if used + w > budget {
                break;
            }
            used += w;
            cut.push(c);
        }
        if !cut.is_empty() {
            kept.push(Span::styled(cut, span.style));
        }
        break;
    }
    kept.push(Span::styled("…", ellipsis));
    put_spans(buf, area, x, y, &kept)
}

/// One row, `left` from the start and `right` flush to the end of `area`.
/// `left` shrinks (truncated) when the two do not fit.
pub(crate) fn space_between(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    left: &[Span<'_>],
    right: &[Span<'_>],
) {
    let right_width = spans_width(right).min(usize::from(area.width));
    let left_max = usize::from(area.width) - right_width;
    put_truncated(buf, area, area.x, y, left, left_max);
    put_spans(buf, area, area.right() - right_width as u16, y, right);
}

/// Paint `color` behind every cell of `area` (Ink `backgroundColor`).
pub(crate) fn fill_bg(buf: &mut Buffer, area: Rect, color: Color) {
    buf.set_style(area.intersection(buf.area), Style::new().bg(color));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::{bg_at, draw, fg_at, modifier_at, row};
    use crate::ui::theme::THEME;
    use ratatui::style::Modifier;

    fn fg(color: Color) -> Style {
        Style::new().fg(color)
    }

    #[test]
    fn put_truncated_writes_text_that_fits_unchanged() {
        let buf = draw(12, 1, |buf, area| {
            let end = put_truncated(buf, area, 1, 0, &[Span::raw("abc"), Span::raw("de")], 5);
            assert_eq!(end, 6);
        });
        assert_eq!(row(&buf, 0), " abcde");
    }

    #[test]
    fn put_truncated_ends_with_an_ellipsis_in_the_cut_span_style() {
        // Ink renders these two rows the same way in a 12-column box.
        let buf = draw(12, 2, |buf, area| {
            let spans = [
                Span::styled(" abc def ", fg(THEME.muted)),
                Span::styled("· 3 unseen and more", fg(THEME.amber)),
            ];
            put_truncated(buf, area, 0, 0, &spans, 12);
            let bold = Style::new().add_modifier(Modifier::BOLD);
            let spans = [
                Span::styled("01", fg(THEME.purple).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled("A long title here", bold),
            ];
            put_truncated(buf, area, 0, 1, &spans, 12);
        });
        assert_eq!(row(&buf, 0), " abc def · …");
        assert_eq!(fg_at(&buf, 1, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 9, 0), THEME.amber);
        assert_eq!(fg_at(&buf, 11, 0), THEME.amber);
        assert_eq!(row(&buf, 1), "01  A long …");
        assert_eq!(modifier_at(&buf, 11, 1), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 11, 1), Color::Reset);
    }

    #[test]
    fn put_truncated_writes_nothing_in_zero_columns() {
        let buf = draw(4, 1, |buf, area| {
            assert_eq!(put_truncated(buf, area, 2, 0, &[Span::raw("abc")], 0), 2);
        });
        assert_eq!(row(&buf, 0), "");
    }

    #[test]
    fn space_between_pins_the_right_side_and_cuts_the_left() {
        let buf = draw(12, 3, |buf, area| {
            let inner = Rect::new(1, 0, 10, 3);
            assert_eq!(area.width, 12);
            space_between(buf, inner, 0, &[Span::raw("ab")], &[Span::raw("xyz")]);
            space_between(
                buf,
                inner,
                1,
                &[Span::raw("abcdefghij")],
                &[Span::raw("xyz")],
            );
            space_between(buf, inner, 2, &[Span::raw("ab")], &[]);
        });
        assert_eq!(row(&buf, 0), " ab     xyz");
        assert_eq!(row(&buf, 1), " abcdef…xyz");
        assert_eq!(row(&buf, 2), " ab");
    }

    #[test]
    fn fill_bg_paints_only_the_part_of_the_area_inside_the_buffer() {
        let buf = draw(4, 2, |buf, _| {
            fill_bg(buf, Rect::new(2, 1, 10, 10), THEME.selection);
        });
        assert_eq!(bg_at(&buf, 1, 1), Color::Reset);
        assert_eq!(bg_at(&buf, 2, 1), THEME.selection);
        assert_eq!(bg_at(&buf, 3, 1), THEME.selection);
        assert_eq!(bg_at(&buf, 2, 0), Color::Reset);
    }
}
