//! Port of `packages/astroshot-review/src/ui/text-input.tsx`.
//!
//! Single-line composer used for feedback. While active it owns the keyboard:
//! the screen routes every key (and bracketed paste) to [`TextInputState`]
//! and acts on the returned [`Outcome`] (`onSubmit` / `onCancel` in Ink).
//!
//! Divergences: the cursor and slicing count `char`s (TS: UTF-16 units), and
//! horizontal scrolling uses the box's real content width (`width - 4`; TS used
//! `width - 2`, which let the caret wrap out of view on long text).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, StatefulWidget, Widget};

use super::theme::THEME;

/// What the screen should do after an input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Keep the composer open (the event may have edited it).
    Pending,
    /// `onSubmit(value)`.
    Submit(String),
    /// `onCancel()`.
    Cancel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInputState {
    value: Vec<char>,
    /// Caret position as a `char` index into `value`, `0..=value.len()`.
    cursor: usize,
}

impl TextInputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn value(&self) -> String {
        self.value.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn insert(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        let count = chars.len();
        self.value.splice(self.cursor..self.cursor, chars);
        self.cursor += count;
    }

    /// A paste (or a fast PTY) delivers several characters at once, possibly
    /// ending in a newline. Insert the printable part, then submit if asked.
    /// A single character plus a newline submits without inserting it, as in TS.
    pub fn handle_paste(&mut self, input: &str) -> Outcome {
        let pasted: String = input
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n'))
            .collect();
        let wants_submit = input.contains(['\r', '\n']);
        if pasted.chars().count() > 1 {
            self.insert(&pasted);
            return if wants_submit {
                Outcome::Submit(self.value())
            } else {
                Outcome::Pending
            };
        }
        if wants_submit {
            return Outcome::Submit(self.value());
        }
        self.insert(&pasted);
        Outcome::Pending
    }

    pub fn handle(&mut self, key: KeyEvent) -> Outcome {
        if key.kind == KeyEventKind::Release {
            return Outcome::Pending;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let meta = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => Outcome::Cancel,
            KeyCode::Enter => Outcome::Submit(self.value()),
            KeyCode::Backspace | KeyCode::Delete => {
                if self.cursor > 0 {
                    self.value.remove(self.cursor - 1);
                    self.cursor -= 1;
                }
                Outcome::Pending
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                Outcome::Pending
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.value.len());
                Outcome::Pending
            }
            KeyCode::Char('a') if ctrl => {
                self.cursor = 0;
                Outcome::Pending
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.value.len();
                Outcome::Pending
            }
            KeyCode::Char('u') if ctrl => {
                self.value.clear();
                self.cursor = 0;
                Outcome::Pending
            }
            KeyCode::Char(c) if !ctrl && !meta => {
                self.insert(c.encode_utf8(&mut [0; 4]));
                Outcome::Pending
            }
            // Other ctrl/alt chords, tab, up/down, and everything else.
            _ => Outcome::Pending,
        }
    }
}

/// Props of the Ink component; the width is the `Rect` it renders into.
/// Needs 4 rows: a rounded 3-row box and the hint line.
pub struct TextInput<'a> {
    pub placeholder: &'a str,
    pub submit_label: Option<&'a str>,
}

impl StatefulWidget for TextInput<'_> {
    type State = TextInputState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut TextInputState) {
        // TS scrolls by `width - 2`, but the box's content is `width - 4` wide
        // (border + padding), so its caret could wrap out of view. Scroll by the
        // real content width instead; identical output while the text fits.
        let inner_width = (area.width as usize).saturating_sub(4).max(4);
        let inverse = Style::new().add_modifier(Modifier::REVERSED);

        let content: Line<'static> = if state.value.is_empty() {
            let placeholder: String = self.placeholder.chars().take(inner_width - 1).collect();
            Line::from(vec![
                Span::styled(" ", inverse),
                Span::styled(placeholder, Style::new().fg(THEME.muted)),
            ])
        } else {
            // Keep the caret visible by scrolling the text horizontally.
            let start = (state.cursor + 1).saturating_sub(inner_width);
            let end = (start + inner_width).min(state.value.len());
            let visible = &state.value[start..end];
            let caret = state.cursor - start;
            let before: String = visible[..caret.min(visible.len())].iter().collect();
            let at = visible.get(caret).copied().unwrap_or(' ');
            let after: String = visible.iter().skip(caret + 1).collect();
            Line::from(vec![
                Span::raw(before),
                Span::styled(at.to_string(), inverse),
                Span::raw(after),
            ])
        };

        let boxed = Rect::new(area.x, area.y, area.width, area.height.min(3));
        Paragraph::new(content)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(THEME.blue))
                    .padding(Padding::horizontal(1)),
            )
            .render(boxed, buf);

        if area.height > 3 {
            let hint = format!("  ⏎ {}  ·  esc cancel", self.submit_label.unwrap_or("send"));
            Paragraph::new(Span::styled(hint, Style::new().fg(THEME.muted)))
                .render(Rect::new(area.x, area.y + 3, area.width, 1), buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::ui::testing::*;
    use crossterm::event::KeyEventState;
    use ratatui::style::Color;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn press(state: &mut TextInputState, code: KeyCode) -> Outcome {
        state.handle(key(code, KeyModifiers::NONE))
    }

    fn ctrl(state: &mut TextInputState, c: char) -> Outcome {
        state.handle(key(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn type_text(state: &mut TextInputState, text: &str) {
        for c in text.chars() {
            assert_eq!(press(state, KeyCode::Char(c)), Outcome::Pending);
        }
    }

    fn typed(text: &str) -> TextInputState {
        let mut state = TextInputState::new();
        type_text(&mut state, text);
        state
    }

    // ---- editing --------------------------------------------------------

    #[test]
    fn typing_inserts_at_the_cursor_and_advances_it() {
        let mut state = typed("helo");
        press(&mut state, KeyCode::Left);
        type_text(&mut state, "l");
        assert_eq!(state.value(), "hello");
        assert_eq!(state.cursor(), 4);
    }

    #[test]
    fn backspace_and_delete_both_remove_the_char_before_the_cursor() {
        let mut state = typed("abc");
        press(&mut state, KeyCode::Backspace);
        assert_eq!((state.value().as_str(), state.cursor()), ("ab", 2));
        press(&mut state, KeyCode::Delete);
        assert_eq!((state.value().as_str(), state.cursor()), ("a", 1));
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Backspace);
        assert_eq!((state.value().as_str(), state.cursor()), ("a", 0));
    }

    #[test]
    fn backspace_removes_mid_string_chars() {
        let mut state = typed("abcd");
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Backspace);
        assert_eq!((state.value().as_str(), state.cursor()), ("acd", 1));
    }

    #[test]
    fn arrows_move_within_bounds() {
        let mut state = typed("ab");
        press(&mut state, KeyCode::Right);
        assert_eq!(state.cursor(), 2);
        for _ in 0..5 {
            press(&mut state, KeyCode::Left);
        }
        assert_eq!(state.cursor(), 0);
        press(&mut state, KeyCode::Right);
        assert_eq!(state.cursor(), 1);
    }

    #[test]
    fn ctrl_a_e_u_jump_and_clear() {
        let mut state = typed("hello");
        ctrl(&mut state, 'a');
        assert_eq!(state.cursor(), 0);
        ctrl(&mut state, 'e');
        assert_eq!(state.cursor(), 5);
        ctrl(&mut state, 'u');
        assert_eq!((state.value().as_str(), state.cursor()), ("", 0));
    }

    #[test]
    fn other_chords_and_navigation_keys_are_ignored() {
        let mut state = typed("ab");
        ctrl(&mut state, 'x');
        state.handle(key(KeyCode::Char('x'), KeyModifiers::ALT));
        press(&mut state, KeyCode::Tab);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Home);
        assert_eq!((state.value().as_str(), state.cursor()), ("ab", 2));
        // Key release events (Windows / kitty protocol) never edit.
        let mut release = key(KeyCode::Char('z'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(state.handle(release), Outcome::Pending);
        assert_eq!(state.value(), "ab");
    }

    #[test]
    fn shifted_characters_insert() {
        let mut state = TextInputState::new();
        state.handle(key(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert_eq!(state.value(), "A");
    }

    #[test]
    fn multibyte_characters_count_as_one_position() {
        let mut state = typed("é⏎");
        press(&mut state, KeyCode::Backspace);
        assert_eq!((state.value().as_str(), state.cursor()), ("é", 1));
    }

    // ---- submit / cancel -------------------------------------------------

    #[test]
    fn enter_submits_the_current_value() {
        let mut state = typed("ship it");
        assert_eq!(
            press(&mut state, KeyCode::Enter),
            Outcome::Submit("ship it".into())
        );
        // Empty submits too; the caller decides what that means.
        assert_eq!(
            press(&mut TextInputState::new(), KeyCode::Enter),
            Outcome::Submit(String::new())
        );
    }

    #[test]
    fn escape_cancels() {
        let mut state = typed("draft");
        assert_eq!(press(&mut state, KeyCode::Esc), Outcome::Cancel);
    }

    // ---- paste ------------------------------------------------------------

    #[test]
    fn multi_character_paste_inserts_at_the_cursor() {
        let mut state = typed("ad");
        press(&mut state, KeyCode::Left);
        assert_eq!(state.handle_paste("bc"), Outcome::Pending);
        assert_eq!((state.value().as_str(), state.cursor()), ("abcd", 3));
    }

    #[test]
    fn paste_ending_in_a_newline_inserts_then_submits() {
        let mut state = typed("x");
        assert_eq!(state.handle_paste("yz\r\n"), Outcome::Submit("xyz".into()));
        assert_eq!(state.value(), "xyz");
    }

    #[test]
    fn paste_strips_embedded_newlines() {
        let mut state = TextInputState::new();
        assert_eq!(state.handle_paste("a\nb\nc"), Outcome::Submit("abc".into()));
    }

    #[test]
    fn single_character_plus_newline_submits_without_inserting_it() {
        // Faithful to text-input.tsx: `pasted.length > 1` is false for "a\n".
        let mut state = typed("hi");
        assert_eq!(state.handle_paste("a\n"), Outcome::Submit("hi".into()));
        assert_eq!(state.value(), "hi");
        assert_eq!(state.handle_paste("\n"), Outcome::Submit("hi".into()));
    }

    #[test]
    fn single_character_paste_inserts_and_empty_paste_is_ignored() {
        let mut state = TextInputState::new();
        assert_eq!(state.handle_paste("q"), Outcome::Pending);
        assert_eq!(state.handle_paste(""), Outcome::Pending);
        assert_eq!(state.value(), "q");
    }

    // ---- rendering --------------------------------------------------------

    fn view<'a>() -> TextInput<'a> {
        TextInput {
            placeholder: "Share feedback…",
            submit_label: Some("Send Feedback"),
        }
    }

    #[test]
    fn empty_input_shows_caret_block_and_placeholder() {
        let mut state = TextInputState::new();
        let buf = render_stateful(view(), &mut state, 30, 4);
        assert_eq!(
            rows(&buf),
            [
                "╭────────────────────────────╮",
                "│  Share feedback…           │",
                "╰────────────────────────────╯",
                "  ⏎ Send Feedback  ·  esc cancel",
            ]
            .map(|row| row
                .chars()
                .take(30)
                .collect::<String>()
                .trim_end()
                .to_string())
        );
        // Caret cell is inverse; placeholder is muted; border is blue.
        assert_eq!(modifier_at(&buf, 2, 1), Modifier::REVERSED);
        assert_eq!(fg_at(&buf, 3, 1), THEME.muted);
        assert_eq!(fg_at(&buf, 0, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 2, 3), THEME.muted);
    }

    #[test]
    fn submit_label_defaults_to_send() {
        let mut state = TextInputState::new();
        let input = TextInput {
            placeholder: "",
            submit_label: None,
        };
        let buf = render_stateful(input, &mut state, 30, 4);
        assert_eq!(row(&buf, 3), "  ⏎ send  ·  esc cancel");
    }

    #[test]
    fn typed_text_renders_with_an_inverse_caret_cell() {
        let mut state = typed("hello");
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Left);
        let buf = render_stateful(view(), &mut state, 30, 4);
        assert_eq!(row(&buf, 1), "│ hello                      │");
        // The caret sits on the 'l' at index 3, drawn at column 2 + 3.
        assert_eq!(buf[(5, 1)].symbol(), "l");
        assert_eq!(modifier_at(&buf, 5, 1), Modifier::REVERSED);
        assert_eq!(modifier_at(&buf, 4, 1), Modifier::empty());
        assert_eq!(modifier_at(&buf, 6, 1), Modifier::empty());
    }

    #[test]
    fn caret_at_end_of_text_is_an_inverse_space() {
        let mut state = typed("hi");
        let buf = render_stateful(view(), &mut state, 30, 4);
        assert_eq!(buf[(4, 1)].symbol(), " ");
        assert_eq!(modifier_at(&buf, 4, 1), Modifier::REVERSED);
        assert_eq!(bg_at(&buf, 4, 1), Color::Reset);
    }

    #[test]
    fn long_text_scrolls_horizontally_to_keep_the_caret_visible() {
        // width 12 -> 8 content columns: the last 7 chars plus the caret cell.
        let mut state = typed("0123456789abcdef");
        let buf = render_stateful(view(), &mut state, 12, 4);
        assert_eq!(row(&buf, 1), "│ 9abcdef  │");
        assert_eq!(modifier_at(&buf, 9, 1), Modifier::REVERSED);
        ctrl(&mut state, 'a');
        let buf = render_stateful(view(), &mut state, 12, 4);
        assert_eq!(row(&buf, 1), "│ 01234567 │");
        assert_eq!(modifier_at(&buf, 2, 1), Modifier::REVERSED);
    }
}
