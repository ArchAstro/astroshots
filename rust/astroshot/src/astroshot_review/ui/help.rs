//! Port of `packages/astroshot-review/src/ui/help.tsx`.
//!
//! The keyboard overlay: a heading, then the key sections flowed into one
//! column (width < 100) or two (width >= 100), like the Ink `flexWrap` row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Widget;

use super::put_spans;
use super::theme::THEME;

pub struct Section {
    pub title: &'static str,
    pub keys: &'static [(&'static str, &'static str)],
}

pub const SECTIONS: [Section; 4] = [
    Section {
        title: "Everywhere",
        keys: &[
            ("1 / 2", "Shots · Friction Logs"),
            ("tab", "next tab"),
            (",", "settings"),
            ("r", "rescan"),
            ("?", "this help"),
            ("q", "quit"),
        ],
    },
    Section {
        title: "Stream",
        keys: &[
            ("↑↓ j k", "move"),
            ("⏎", "open detail"),
            ("f", "full-screen review"),
            ("u", "Unseen ⇄ History"),
            ("m", "Movies only"),
            ("s", "mark Seen"),
            ("S", "mark all visible Seen"),
            ("A", "mark this worktree Seen"),
            ("c", "send feedback"),
            ("z", "collapse worktree"),
            ("o", "reveal in Finder"),
            ("O", "open movie in player"),
            ("y", "copy image"),
        ],
    },
    Section {
        title: "Detail / Review",
        keys: &[
            ("← →", "older · newer"),
            ("+ / -", "zoom in / out"),
            ("0", "reset zoom"),
            ("p", "play in tray"),
            ("space", "play / pause"),
            (", .", "seek −5s / +5s"),
            ("[ ]", "previous / next chapter"),
            ("esc", "back / close"),
        ],
    },
    Section {
        title: "Friction Logs",
        keys: &[
            ("⏎", "open log · open step"),
            ("[ ]", "switch run · switch image"),
            ("p", "toggle prompt"),
            ("← →", "previous / next step"),
        ],
    },
];

/// Key column width: `key.padEnd(10)`.
const KEY_WIDTH: usize = 10;

pub struct HelpOverlay;

impl Widget for HelpOverlay {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // paddingX 2, paddingY 1.
        let left = area.x + 2;
        let mut y = area.y + 1;
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let width = area.width as usize;
        let columns = if width >= 100 { 2 } else { 1 };
        let column_width = width.saturating_sub(4) / columns;

        // Each section is `title + keys` tall plus marginBottom 1; a row of
        // sections is as tall as its tallest member.
        let sections_height: usize = SECTIONS
            .chunks(columns)
            .map(|row| row.iter().map(|s| s.keys.len() + 2).max().unwrap_or(0))
            .sum();
        // Two text rows + marginTop 1 + the sections.
        let content = 3 + sections_height;
        let inner = usize::from(area.height).saturating_sub(2);
        let subtitle = Span::styled("Press ? or esc to close", Style::new().fg(THEME.muted));
        if content <= inner {
            put_spans(buf, area, left, y, &[Span::styled("Keyboard", bold)]);
            put_spans(buf, area, left, y + 1, &[subtitle]);
            y += 3;
        } else {
            // The column overflows, so Yoga shrinks its three children in
            // proportion to their heights. Both text rows end up shorter than
            // one row and are floored onto the first row (the subtitle paints
            // over the title); the sections keep their own layout and start
            // at the rounded sum of what is left above them.
            let text_height = 1.0 - (content - inner) as f64 / (2 + sections_height) as f64;
            put_spans(buf, area, left, y, &[subtitle]);
            y += (2.0 * text_height + 1.0 + 0.5).floor() as u16;
        }

        for row in SECTIONS.chunks(columns) {
            let mut row_height = 0;
            for (index, section) in row.iter().enumerate() {
                let x = left + (index * column_width) as u16;
                let column = Rect::new(
                    x,
                    area.y,
                    (column_width as u16).min(area.right().saturating_sub(x)),
                    area.height,
                );
                put_spans(
                    buf,
                    column,
                    x,
                    y,
                    &[Span::styled(
                        section.title,
                        Style::new().fg(THEME.brand).add_modifier(Modifier::BOLD),
                    )],
                );
                for (offset, (key, label)) in section.keys.iter().enumerate() {
                    let pad = KEY_WIDTH.saturating_sub(key.chars().count());
                    put_spans(
                        buf,
                        column,
                        x,
                        y + 1 + offset as u16,
                        &[
                            Span::styled(
                                format!("{key}{}", " ".repeat(pad)),
                                Style::new().fg(THEME.blue),
                            ),
                            Span::styled(*label, Style::new().fg(THEME.text)),
                        ],
                    );
                }
                row_height = row_height.max(section.keys.len() + 2);
            }
            y += row_height as u16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::ui::testing::*;

    #[test]
    fn single_column_stacks_every_section_with_padded_keys() {
        let buf = render(HelpOverlay, 60, 44);
        let lines = rows(&buf);
        assert_eq!(lines[0], "");
        assert_eq!(lines[1], "  Keyboard");
        assert_eq!(lines[2], "  Press ? or esc to close");
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], "  Everywhere");
        assert_eq!(lines[5], "  1 / 2     Shots · Friction Logs");
        assert_eq!(lines[9], "  ?         this help");
        assert_eq!(lines[10], "  q         quit");
        assert_eq!(lines[11], "");
        assert_eq!(lines[12], "  Stream");
        assert_eq!(lines[13], "  ↑↓ j k    move");
        // Each section is title + keys + one blank row (marginBottom).
        assert_eq!(lines[25], "  y         copy image");
        assert_eq!(lines[26], "");
        assert_eq!(lines[27], "  Detail / Review");
        assert_eq!(lines[28], "  ← →       older · newer");
        assert_eq!(lines[37], "  Friction Logs");
        assert_eq!(lines[38], "  ⏎         open log · open step");
    }

    #[test]
    fn wide_terminal_flows_two_sections_per_row() {
        let buf = render(HelpOverlay, 100, 30);
        let lines = rows(&buf);
        // column width = (100 - 4) / 2 = 48
        assert_eq!(lines[4], format!("  {:<48}Stream", "Everywhere"));
        assert_eq!(
            lines[5],
            format!("  {:<48}↑↓ j k    move", "1 / 2     Shots · Friction Logs")
        );
        // Row 1 is as tall as Stream (13 keys): Detail starts at 4 + 15.
        assert_eq!(
            lines[19],
            format!("  {:<48}Friction Logs", "Detail / Review")
        );
        assert_eq!(
            lines[20],
            format!(
                "  {:<48}⏎         open log · open step",
                "← →       older · newer"
            )
        );
    }

    #[test]
    fn styles_match_the_ink_component() {
        let buf = render(HelpOverlay, 60, 44);
        assert_eq!(modifier_at(&buf, 2, 1), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 2, 2), THEME.muted);
        assert_eq!(fg_at(&buf, 2, 4), THEME.brand);
        assert_eq!(modifier_at(&buf, 2, 4), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 2, 5), THEME.blue);
        assert_eq!(fg_at(&buf, 12, 5), THEME.text);
    }

    // Rows from the real Ink `<App>` help overlay at 80x12 and 100x30 (body
    // heights 9 and 27), where the content is taller than the box.
    #[test]
    fn overflowing_content_shrinks_the_title_rows_like_yoga() {
        let lines = rows(&render(HelpOverlay, 80, 9));
        assert_eq!(
            lines[..5],
            [
                "",
                "  Press ? or esc to close",
                "  Everywhere",
                "  1 / 2     Shots · Friction Logs",
                "  tab       next tab",
            ]
        );

        let buf = render(HelpOverlay, 100, 27);
        let lines = rows(&buf);
        assert_eq!(lines[0], "");
        assert_eq!(lines[1], "  Press ? or esc to close");
        assert_eq!(fg_at(&buf, 2, 1), THEME.muted);
        assert_eq!(modifier_at(&buf, 2, 1), Modifier::empty());
        assert_eq!(lines[2], "");
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], format!("  {:<48}Stream", "Everywhere"));
        assert_eq!(
            lines[19],
            format!("  {:<48}Friction Logs", "Detail / Review")
        );
        assert_eq!(lines[26], "  [ ]       previous / next chapter");
    }

    #[test]
    fn small_areas_clip_instead_of_panicking() {
        let buf = render(HelpOverlay, 12, 5);
        assert_eq!(rows(&buf)[1], "  Press ? or");
        assert_eq!(rows(&buf)[2], "  Everywhe");
    }
}
