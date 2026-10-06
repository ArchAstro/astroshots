//! Port of `packages/astroshot-review/src/ui` (Ink -> ratatui).
//!
//! # UI convention (every component port follows this)
//!
//! 1. **Components are plain values that render into a ratatui `Buffer`.**
//!    - A block component (owns rows) is a struct implementing
//!      `Widget` (`StatefulWidget` when it has state). It draws into the
//!      `Rect` it is given and never reads the terminal size itself.
//!    - An inline component (an Ink `<Text>` run) is a function returning a
//!      `Span` (or `Vec<Span>`) that the parent places in a `Line`.
//! 2. **Props become struct fields** (borrowed: `&'a str`, `&'a [T]`). Ink's
//!    `width`/`height` props are the `Rect` the widget renders into.
//! 3. **Colors come from `theme::THEME`**; never inline a `Color::Rgb` that
//!    already has a token. `<Text bold|inverse>` is `Modifier::BOLD|REVERSED`.
//! 4. **Stateful input is a state struct plus a view.** The state owns the
//!    data and exposes `handle(KeyEvent) -> Outcome` (and `handle_paste`);
//!    the view is a `StatefulWidget` over that state. The screen routes
//!    crossterm events to `handle` and acts on the returned `Outcome`; Ink's
//!    `onSubmit`/`onCancel` callbacks become `Outcome::{Submit, Cancel}`.
//! 5. **Ink `<Box>` flex layout maps to `Layout` constraints** (`Length` for
//!    fixed/`flexShrink={0}` rows, `Min`/`Fill` for `flexGrow`). Centering
//!    and flex-wrap that `Layout` cannot express are computed explicitly with
//!    the same floor rounding Yoga uses. A one-row `<Box height={1}>` is
//!    `Constraint::Length(1)`.
//! 6. **Widths are terminal columns.** Measure with [`text_width`]
//!    (unicode-width via ratatui); ported TS code that counted `.length`
//!    (UTF-16 units) counts `char`s, as `theme::truncate` already does.
//! 7. **Tests render into `ratatui::backend::TestBackend`** through the
//!    helpers in `testing` and assert exact row text plus cell styles.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Span;

pub mod chrome;
pub mod context;
pub mod detail;
pub mod help;
pub mod hooks;
pub mod movie_player;
pub mod picture;
pub mod selectors;
pub mod settings;
pub mod system;
pub mod text_input;
pub mod theme;

#[cfg(test)]
pub(crate) mod testing;

/// Terminal columns `text` occupies.
pub fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

/// Write `spans` left to right starting at (`x`, `y`), clipped to `area`.
/// Returns the column after the last cell written.
pub(crate) fn put_spans(buf: &mut Buffer, area: Rect, x: u16, y: u16, spans: &[Span<'_>]) -> u16 {
    if y < area.top() || y >= area.bottom() {
        return x;
    }
    let mut x = x.max(area.left());
    for span in spans {
        if x >= area.right() {
            break;
        }
        let (next, _) = buf.set_span(x, y, span, area.right() - x);
        x = next;
    }
    x
}
