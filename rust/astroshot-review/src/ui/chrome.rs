//! Port of `packages/astroshot-review/src/ui/chrome.tsx`.
//!
//! Shared chrome. Inline Ink `<Text>` components are functions returning a
//! `Span`; block components are `Widget`s that render into the `Rect` they
//! are given (the TS `width` prop is `area.width`).
//!
//! Not ported as a component: `Line` (`<Box flexShrink={0} height={1}>`) is a
//! `Constraint::Length(1)` row in the parent's `Layout`.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Widget;

use super::theme::{THEME, truncate};
use super::{put_spans, text_width};
use astroshot_engine::review_data::model::ReviewState;

/// Background of the movie badge (`#3b2f6e`); not a theme token.
const MOVIE_BADGE_BG: Color = Color::Rgb(0x3b, 0x2f, 0x6e);

pub fn worktree_chip(label: &str) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::new().bg(THEME.selection).fg(THEME.purple),
    )
}

pub fn review_badge(state: ReviewState, stale: bool) -> Span<'static> {
    if state == ReviewState::Seen {
        return Span::styled("● Seen", Style::new().fg(THEME.blue));
    }
    let text = if stale {
        "● Unseen · changed"
    } else {
        "● Unseen"
    };
    Span::styled(text, Style::new().fg(THEME.amber))
}

/// `duration` is falsy in TS when `null` or empty.
pub fn movie_badge(duration: Option<&str>) -> Span<'static> {
    let text = match duration {
        Some(duration) if !duration.is_empty() => format!(" Movie · {duration} "),
        _ => " Movie ".to_string(),
    };
    Span::styled(text, Style::new().bg(MOVIE_BADGE_BG).fg(THEME.purple))
}

pub fn status_pill(label: Option<&str>) -> Option<Span<'static>> {
    let label = label.filter(|label| !label.is_empty())?;
    let color = match label {
        "Complete" => THEME.green,
        "Running" => THEME.amber,
        "Failed" => THEME.red,
        "Ready" => THEME.blue,
        _ => THEME.muted,
    };
    Some(Span::styled(label.to_string(), Style::new().fg(color)))
}

pub fn execution_pill(status: Option<&str>) -> Option<Span<'static>> {
    let status = status.filter(|status| !status.is_empty())?;
    let color = match status {
        "pass" => THEME.green,
        "fail" => THEME.red,
        "running" => THEME.amber,
        _ => THEME.muted,
    };
    Some(Span::styled(
        format!("· run {status}"),
        Style::new().fg(color),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHint {
    pub key: String,
    pub label: String,
}

impl KeyHint {
    pub fn new(key: &str, label: &str) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
        }
    }
}

/// One row of `key label` hints; hints that would overflow the row are
/// dropped (a later, shorter hint can still fit, as in TS).
pub struct HintBar<'a> {
    pub hints: &'a [KeyHint],
}

impl Widget for HintBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = area.width as usize;
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut used = 0;
        for hint in self.hints {
            // TS counts `.length`: key + space + label + 3 trailing spaces.
            let length = hint.key.chars().count() + 1 + hint.label.chars().count() + 3;
            if used + length > width {
                continue;
            }
            used += length;
            spans.push(Span::styled(
                hint.key.clone(),
                Style::new().fg(THEME.brand).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {}   ", hint.label),
                Style::new().fg(THEME.muted),
            ));
        }
        put_spans(buf, area, area.x, area.y, &spans);
    }
}

/// Centered one-row message on the selection background. Renders nothing for
/// an empty message (TS: `null`).
pub struct Toast<'a> {
    pub message: Option<&'a str>,
}

impl Widget for Toast<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(message) = self.message.filter(|message| !message.is_empty()) else {
            return;
        };
        let text = format!(
            " {} ",
            truncate(message, (area.width as usize).saturating_sub(6).max(8))
        );
        let x = area.x + (area.width as usize).saturating_sub(text_width(&text)) as u16 / 2;
        let style = Style::new()
            .bg(THEME.selection)
            .fg(THEME.text)
            .add_modifier(Modifier::BOLD);
        put_spans(buf, area, x, area.y, &[Span::styled(text, style)]);
    }
}

pub fn section_label(text: &str) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::new().fg(THEME.muted).add_modifier(Modifier::BOLD),
    )
}

/// `─` repeated `width` times; `color` defaults to `THEME.faint`.
pub fn rule(width: usize, color: Option<Color>) -> Span<'static> {
    Span::styled(
        "─".repeat(width),
        Style::new().fg(color.unwrap_or(THEME.faint)),
    )
}

const META_LABEL_WIDTH: u16 = 10;

/// `label` in a fixed 10-column muted gutter, then `value`, cut from the
/// start (`…tail`) when it does not fit (Ink `wrap="truncate-start"`).
pub struct MetaRow<'a> {
    pub label: &'a str,
    pub value: &'a str,
}

impl Widget for MetaRow<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [label_area, value_area] =
            Layout::horizontal([Constraint::Length(META_LABEL_WIDTH), Constraint::Fill(1)])
                .areas(area);
        put_spans(
            buf,
            label_area,
            label_area.x,
            label_area.y,
            &[Span::styled(self.label, Style::new().fg(THEME.muted))],
        );
        let value = truncate_start(self.value, value_area.width as usize);
        put_spans(
            buf,
            value_area,
            value_area.x,
            value_area.y,
            &[Span::raw(value)],
        );
    }
}

/// Keep the end of `text`, replacing the cut head with `…`, within `width` columns.
fn truncate_start(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 1; // the ellipsis
    for c in text.chars().rev() {
        let w = text_width(c.encode_utf8(&mut [0; 4]));
        if used + w > width {
            break;
        }
        used += w;
        tail.push(c);
    }
    tail.reverse();
    format!("…{}", tail.into_iter().collect::<String>())
}

/// Ink `<Text wrap="wrap">`: `wrap-ansi` with `{trim: false, hard: true}`.
///
/// Spaces are kept, so a row can end in a space, and a row that follows an
/// exactly full one starts with the space that separated the two words. A
/// word wider than the line is broken; it starts on the current row unless a
/// fresh row saves a break.
pub(crate) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let mut rows = vec![String::new()];
        for (index, word) in paragraph.split(' ').enumerate() {
            let mut row_width = text_width(&rows[rows.len() - 1]);
            if index != 0 {
                if row_width >= width {
                    rows.push(String::new());
                    row_width = 0;
                }
                let last = rows.len() - 1;
                rows[last].push(' ');
                row_width += 1;
            }
            let word_width = text_width(word);
            if word_width > width {
                let remaining = width - row_width.min(width);
                let breaks_starting_this_row = 1 + (word_width - remaining - 1) / width;
                let breaks_starting_next_row = (word_width - 1) / width;
                if breaks_starting_next_row < breaks_starting_this_row {
                    rows.push(String::new());
                }
                wrap_word(&mut rows, word, width);
                continue;
            }
            if row_width + word_width > width && row_width > 0 && word_width > 0 {
                rows.push(String::new());
            }
            let last = rows.len() - 1;
            rows[last].push_str(word);
        }
        lines.append(&mut rows);
    }
    lines
}

/// Append `word` to the last row character by character, opening a new row
/// whenever the current one is full.
fn wrap_word(rows: &mut Vec<String>, word: &str, width: usize) {
    let mut visible = text_width(&rows[rows.len() - 1]);
    let count = word.chars().count();
    for (index, c) in word.chars().enumerate() {
        let w = text_width(c.encode_utf8(&mut [0; 4]));
        if visible + w <= width {
            let last = rows.len() - 1;
            rows[last].push(c);
        } else {
            rows.push(c.to_string());
            visible = 0;
        }
        visible += w;
        if visible == width && index + 1 < count {
            rows.push(String::new());
            visible = 0;
        }
    }
}

/// Title, wrapped body, and optional action, centered in the area both ways
/// with 2 columns of horizontal padding.
///
/// Yoga places a `<Text>` at the floor of a fractional position and a `<Box>`
/// at the nearest cell (half rounds up). The title is a `<Text>`; the body
/// and the action sit in boxes. So when the spare rows are odd the body and
/// action land one row lower than the title would suggest, and when the spare
/// columns are odd the action sits one column further right.
pub struct EmptyState<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub action: Option<&'a str>,
}

impl Widget for EmptyState<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = area.width as usize;
        let inner = width.saturating_sub(4);
        let body_width = inner.min(60);
        let body_lines = wrap_words(self.body, body_width);
        // Measured with trailing spaces, as Ink measures the wrapped text.
        let block_width = body_lines
            .iter()
            .map(|line| text_width(line))
            .max()
            .unwrap_or(0);
        let action = self.action.filter(|action| !action.is_empty());

        let total = 1 + body_lines.len() + if action.is_some() { 2 } else { 0 };
        let spare = (area.height as usize).saturating_sub(total);
        let half_row = (spare % 2) as u16;
        let mut y = area.y + (spare / 2) as u16;
        let text_x = |content: usize| (2 + inner.saturating_sub(content) / 2) as u16;
        let box_x = |content: usize| (2 + inner.saturating_sub(content).div_ceil(2)) as u16;

        let title = Span::styled(self.title, Style::new().add_modifier(Modifier::BOLD));
        put_spans(
            buf,
            area,
            area.x + text_x(text_width(self.title)),
            y,
            &[title],
        );
        y += 1 + half_row;

        let body_x =
            area.x + box_x(body_width) + (body_width.saturating_sub(block_width) / 2) as u16;
        let body_style = Style::new().fg(THEME.muted);
        for line in &body_lines {
            put_spans(
                buf,
                area,
                body_x,
                y,
                &[Span::styled(line.clone(), body_style)],
            );
            y += 1;
        }

        if let Some(action) = action {
            y += 1; // marginTop
            let span = Span::styled(action, Style::new().fg(THEME.brand));
            put_spans(buf, area, area.x + box_x(text_width(action)), y, &[span]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::*;
    use ratatui::style::Color;

    fn inline(span: Span<'static>) -> ratatui::buffer::Buffer {
        draw(30, 1, |buf, area| {
            put_spans(buf, area, 0, 0, &[span]);
        })
    }

    #[test]
    fn worktree_chip_pads_label_on_selection_background() {
        let buf = inline(worktree_chip("main"));
        assert_eq!(row(&buf, 0), " main");
        assert_eq!(bg_at(&buf, 0, 0), THEME.selection);
        assert_eq!(fg_at(&buf, 1, 0), THEME.purple);
        assert_eq!(bg_at(&buf, 5, 0), THEME.selection);
        assert_eq!(bg_at(&buf, 6, 0), Color::Reset);
    }

    #[test]
    fn review_badge_text_and_color_per_state() {
        let seen = inline(review_badge(ReviewState::Seen, false));
        assert_eq!(row(&seen, 0), "● Seen");
        assert_eq!(fg_at(&seen, 0, 0), THEME.blue);
        let unseen = inline(review_badge(ReviewState::Pending, false));
        assert_eq!(row(&unseen, 0), "● Unseen");
        assert_eq!(fg_at(&unseen, 0, 0), THEME.amber);
        let changed = inline(review_badge(ReviewState::Pending, true));
        assert_eq!(row(&changed, 0), "● Unseen · changed");
        // A seen shot ignores `stale`, like TS.
        assert_eq!(
            row(&inline(review_badge(ReviewState::Seen, true)), 0),
            "● Seen"
        );
    }

    #[test]
    fn movie_badge_shows_duration_when_present() {
        let with = inline(movie_badge(Some("0:12")));
        assert_eq!(row(&with, 0), " Movie · 0:12");
        assert_eq!(bg_at(&with, 13, 0), Color::Rgb(0x3b, 0x2f, 0x6e));
        assert_eq!(bg_at(&with, 14, 0), Color::Reset);
        assert_eq!(bg_at(&with, 0, 0), Color::Rgb(0x3b, 0x2f, 0x6e));
        assert_eq!(fg_at(&with, 0, 0), THEME.purple);
        assert_eq!(row(&inline(movie_badge(None)), 0), " Movie");
        assert_eq!(row(&inline(movie_badge(Some(""))), 0), " Movie");
    }

    #[test]
    fn status_pill_color_per_label_and_hidden_when_empty() {
        let cases = [
            ("Complete", THEME.green),
            ("Running", THEME.amber),
            ("Failed", THEME.red),
            ("Ready", THEME.blue),
            ("Other", THEME.muted),
        ];
        for (label, color) in cases {
            let buf = inline(status_pill(Some(label)).unwrap());
            assert_eq!(row(&buf, 0), label);
            assert_eq!(fg_at(&buf, 0, 0), color, "{label}");
        }
        assert!(status_pill(None).is_none());
        assert!(status_pill(Some("")).is_none());
    }

    #[test]
    fn execution_pill_color_per_status_and_hidden_when_empty() {
        let cases = [
            ("pass", THEME.green),
            ("fail", THEME.red),
            ("running", THEME.amber),
            ("skipped", THEME.muted),
        ];
        for (status, color) in cases {
            let buf = inline(execution_pill(Some(status)).unwrap());
            assert_eq!(row(&buf, 0), format!("· run {status}"));
            assert_eq!(fg_at(&buf, 0, 0), color, "{status}");
        }
        assert!(execution_pill(None).is_none());
    }

    #[test]
    fn hint_bar_renders_bold_keys_and_muted_labels() {
        let hints = [KeyHint::new("q", "quit"), KeyHint::new("?", "help")];
        let buf = render(HintBar { hints: &hints }, 30, 1);
        assert_eq!(row(&buf, 0), "q quit   ? help");
        assert_eq!(fg_at(&buf, 0, 0), THEME.brand);
        assert_eq!(modifier_at(&buf, 0, 0), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 2, 0), THEME.muted);
        assert_eq!(modifier_at(&buf, 2, 0), Modifier::empty());
        assert_eq!(fg_at(&buf, 9, 0), THEME.brand);
    }

    #[test]
    fn hint_bar_drops_hints_that_do_not_fit_but_keeps_later_short_ones() {
        // lengths: "a one" 1+1+3+3=8, "b a-very-long-label" 1+1+17+3=22, "c x" 1+1+1+3=6
        let hints = [
            KeyHint::new("a", "one"),
            KeyHint::new("b", "a-very-long-label"),
            KeyHint::new("c", "x"),
        ];
        let buf = render(HintBar { hints: &hints }, 16, 1);
        assert_eq!(row(&buf, 0), "a one   c x");
        // Exactly full width fits; one column less drops the last hint.
        assert_eq!(
            row(&render(HintBar { hints: &hints }, 14, 1), 0),
            "a one   c x"
        );
        assert_eq!(row(&render(HintBar { hints: &hints }, 13, 1), 0), "a one");
        assert_eq!(row(&render(HintBar { hints: &hints }, 5, 1), 0), "");
    }

    #[test]
    fn toast_is_centered_truncated_and_styled() {
        let buf = render(
            Toast {
                message: Some("Saved"),
            },
            20,
            1,
        );
        assert_eq!(row_raw(&buf, 0), "       Saved        ");
        assert_eq!(bg_at(&buf, 6, 0), THEME.selection);
        assert_eq!(bg_at(&buf, 5, 0), Color::Reset);
        assert_eq!(fg_at(&buf, 7, 0), THEME.text);
        assert_eq!(modifier_at(&buf, 7, 0), Modifier::BOLD);

        // width 20 -> text budget 14, plus a space each side.
        let long = render(
            Toast {
                message: Some("0123456789abcdefghij"),
            },
            20,
            1,
        );
        assert_eq!(row_raw(&long, 0), "   0123456789abc…   ");
        assert_eq!(row(&render(Toast { message: None }, 20, 1), 0), "");
    }

    #[test]
    fn toast_budget_never_drops_below_eight_columns() {
        let buf = render(
            Toast {
                message: Some("abcdefghijkl"),
            },
            10,
            1,
        );
        assert_eq!(row(&buf, 0), " abcdefg…");
    }

    #[test]
    fn section_label_is_bold_muted_and_rule_is_faint_by_default() {
        let label = inline(section_label("Files"));
        assert_eq!(row(&label, 0), "Files");
        assert_eq!(fg_at(&label, 0, 0), THEME.muted);
        assert_eq!(modifier_at(&label, 0, 0), Modifier::BOLD);

        let default_rule = inline(rule(5, None));
        assert_eq!(row(&default_rule, 0), "─────");
        assert_eq!(fg_at(&default_rule, 0, 0), THEME.faint);
        assert_eq!(fg_at(&inline(rule(2, Some(THEME.red))), 0, 0), THEME.red);
        assert_eq!(row(&inline(rule(0, None)), 0), "");
    }

    #[test]
    fn meta_row_uses_a_ten_column_label_gutter() {
        let buf = render(
            MetaRow {
                label: "Path",
                value: "/a/b/c.png",
            },
            30,
            1,
        );
        assert_eq!(row(&buf, 0), "Path      /a/b/c.png");
        assert_eq!(fg_at(&buf, 0, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 10, 0), Color::Reset);
    }

    #[test]
    fn meta_row_truncates_the_value_from_the_start() {
        let buf = render(
            MetaRow {
                label: "Path",
                value: "/Users/me/very/long/shot.png",
            },
            24,
            1,
        );
        // 14 value columns: ellipsis plus the last 13 characters.
        assert_eq!(row(&buf, 0), "Path      …long/shot.png");
        assert_eq!(truncate_start("abcdef", 3), "…ef");
        assert_eq!(truncate_start("abc", 3), "abc");
        assert_eq!(truncate_start("abc", 0), "");
    }

    #[test]
    fn empty_state_centers_title_wrapped_body_and_action() {
        let buf = render(
            EmptyState {
                title: "No shots",
                body: "Capture one with astroshot to see it here",
                action: Some("press r to rescan"),
            },
            30,
            9,
        );
        // body width = 26: "Capture one with astroshot" fills the row, so the next
        // starts with the separating space. The action box has 9 spare columns
        // and rounds its half column up.
        assert_eq!(
            rows(&buf),
            [
                "",
                "",
                "           No shots",
                "  Capture one with astroshot",
                "   to see it here",
                "",
                "       press r to rescan",
                "",
                "",
            ]
        );
        assert_eq!(modifier_at(&buf, 11, 2), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 2, 3), THEME.muted);
        assert_eq!(fg_at(&buf, 7, 6), THEME.brand);
    }

    #[test]
    fn empty_state_without_action_is_vertically_centered_and_caps_body_at_60() {
        let body = "word ".repeat(30);
        let buf = render(
            EmptyState {
                title: "T",
                body: body.trim(),
                action: None,
            },
            100,
            8,
        );
        let lines = rows(&buf);
        assert_eq!(lines[2].trim(), "T");
        // 30 words * 5 - 1 = 149 columns wrap to 60-column lines.
        assert_eq!(lines[3].trim(), "word ".repeat(12).trim());
        assert_eq!(lines[4].trim(), "word ".repeat(12).trim());
        assert_eq!(lines[5].trim(), "word ".repeat(6).trim());
        assert_eq!(lines[6], "");
    }

    #[test]
    fn wrap_words_breaks_overlong_words() {
        assert_eq!(
            wrap_words("abcdefghij kl", 4),
            ["abcd", "efgh", "ij ", "kl"]
        );
        assert_eq!(wrap_words("a\nb", 4), ["a", "b"]);
    }

    #[test]
    fn wrap_words_keeps_spaces_the_way_ink_does() {
        // A full row pushes the separating space onto the next row.
        assert_eq!(
            wrap_words(
                "Went to the page and looked around for a sign up button",
                40
            ),
            [
                "Went to the page and looked around for a",
                " sign up button"
            ]
        );
        assert_eq!(wrap_words("abcd efgh", 4), ["abcd", " ", "efgh"]);
        assert_eq!(wrap_words("a  b   c", 3), ["a  ", "b  ", " c"]);
        // A long word starts on the current row unless a fresh row saves a break.
        assert_eq!(
            wrap_words(
                "see https://example.com/a/very/long/url/that/does/not/fit ok",
                20
            ),
            [
                "see https://example.",
                "com/a/very/long/url/",
                "that/does/not/fit ok"
            ]
        );
        assert_eq!(
            wrap_words("ab abcdefghij kl", 4),
            ["ab ", "abcd", "efgh", "ij ", "kl"]
        );
        assert_eq!(wrap_words("", 5), [""]);
    }

    #[test]
    fn empty_state_puts_body_and_action_a_row_lower_when_spare_rows_are_odd() {
        let state = || EmptyState {
            title: "You’re all caught up",
            body: "Every friction log has been seen.",
            action: Some("u View history"),
        };
        // 4 content rows in 7: the title floors to row 1, the boxes round to 3 and 5.
        assert_eq!(
            rows(&render(state(), 40, 7)),
            [
                "",
                "          You’re all caught up",
                "",
                "   Every friction log has been seen.",
                "",
                "             u View history",
                "",
            ]
        );
        assert_eq!(
            rows(&render(state(), 40, 8)),
            [
                "",
                "",
                "          You’re all caught up",
                "   Every friction log has been seen.",
                "",
                "             u View history",
                "",
                "",
            ]
        );
    }
}
