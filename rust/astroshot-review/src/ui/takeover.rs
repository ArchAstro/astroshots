//! Port of `packages/astroshot-review/src/ui/takeover.tsx`.
//!
//! Full-screen review: header, image stage, and the feedback rail.
//! Pages over run siblings, oldest → newest.
//!
//! # Mapping
//!
//! - `<ReviewTakeover>` is the [`ReviewTakeover`] `StatefulWidget`; its Ink
//!   `width`/`height` props are the `Rect` it renders into. The remaining
//!   props are [`ReviewTakeoverProps`].
//! - [`ReviewTakeoverState`] owns what the Ink children kept in hooks: the
//!   stage's [`PictureState`], the [`MoviePlayerState`] and the composer's
//!   [`TextInputState`]. The picture and the player replace each other (the
//!   one not rendered is dropped, which unregisters it from the image layer)
//!   and the composer text is dropped when the screen renders with
//!   `composer: false`.
//! - `playback` / `onPlayback` are the [`Playback`] handle the state was
//!   built with; the parent keeps a clone and patches it.
//! - The component binds no keys itself. While the composer is open the
//!   parent routes every key and paste to [`ReviewTakeoverState::handle`] /
//!   [`ReviewTakeoverState::handle_paste`]; `onComposerSubmit` /
//!   `onComposerCancel` are the returned [`Outcome`]. Paging, pan, zoom,
//!   playback and `c s esc` belong to `app.tsx`.
//!
//! # Layout
//!
//! Checked row for row against frames captured from the Ink component.
//!
//! - Rows 0-2 are the header (title and paging, metadata, rule). The stage
//!   and the rail share the next `max(4, height - 4)` rows; the last row of
//!   the screen stays blank.
//! - The rail is a flex column: every row is fixed except the comment list
//!   (`flexGrow={1} flexShrink={100}`) and, while it is open, the composer
//!   (Ink's default `flexShrink` of 1). [`flex_sizes`] reproduces Yoga's two
//!   distribution passes and [`round_grid`] its pixel-grid rounding, so a
//!   rail that does not fit lays out as it does in Ink:
//!   - the comment list gives up rows first and is clipped;
//!   - the composer then loses a sliver of a row, which makes Yoga place its
//!     hint line on the box's bottom border (`  ⏎ Send Feedback  ·  esc
//!     cancel───╯`) and leave the row below it blank;
//!   - once the comment list is gone the composer's box shrinks to its two
//!     border rows and the input text is painted over the lower one.
//! - The composer's hint line wraps (`  ⏎ Send Feedback  ·  esc` / `cancel`)
//!   when the rail is narrower than 34 columns, making the composer 5 rows.
//!
//! Divergences, all in layouts Ink itself breaks:
//!
//! - Rail rows that do not fit: Ink applies only the innermost
//!   `overflow="hidden"`, so rows that have a clip of their own (`<Line>`,
//!   comment bodies) escape the box that should cut them off. Clipped
//!   comment bodies show through the action rows and the composer
//!   (`s Seen over`, `│u Share feedback…the river bank │`), and rail rows
//!   past the stage land on the blank last row. Here the comment list and
//!   the rail are clipped as their `overflow="hidden"` asks; the rows that
//!   fit are where Ink puts them.
//! - Header row when the title and the paging text do not fit (under about
//!   45 columns, or a long sequence): Ink shrinks both and wraps `esc close`
//!   out of view. Here the paging text keeps its width and the title is cut
//!   to the room that is left.
//! - Width below 39: the stage keeps its 10-column minimum and the rail its
//!   28, and Ink paints past the right edge. Here everything is clipped to
//!   the screen, as is a stage taller than a screen under 8 rows.

use std::sync::Arc;

use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{StatefulWidget, Widget};

use crate::ui::chrome::{review_badge, rule, section_label};
use crate::ui::context::AppServices;
use crate::ui::detail::{
    CommentList, DetailPosition, Spans, bold, fg, first_wrapped_row, ink_wrap, spans_width, styled,
    truncate_end, truthy,
};
use crate::ui::hooks::Wake;
use crate::ui::movie_player::{MovieBackend, MoviePlayer, MoviePlayerState, Playback, ProgressBar};
use crate::ui::picture::{Picture, PictureProps, PictureState};
use crate::ui::put_spans;
use crate::ui::selectors::review_state_of;
use crate::ui::text_input::{Outcome, TextInput, TextInputState};
use crate::ui::theme::{DateInput, THEME, abbreviated_date_time, truncate};
use astroshot_engine::review_data::model::Shot;

/// Width of the feedback rail on terminals at least 100 columns wide.
pub const RAIL_WIDTH: u16 = 42;

const STALE_NOTE: &str = "A newer image was captured since this was seen.";
const COMPOSER_PLACEHOLDER: &str = "Share feedback…";
const COMPOSER_SUBMIT_LABEL: &str = "Send Feedback";

/// Props of `<ReviewTakeover>` other than its size, playback and callbacks.
#[derive(Debug, Clone)]
pub struct ReviewTakeoverProps<'a> {
    pub shot: &'a Shot,
    /// 1-based index among the run siblings (0 when the shot is not listed).
    pub position: DetailPosition,
    pub composer: bool,
    /// The parent's player switch (`ui.inlinePlayer`), not `playback.playing`.
    pub playing: bool,
    pub busy: bool,
    pub error: Option<&'a str>,
    /// 1 shows the whole image; above magnifies (crops in).
    pub zoom: f64,
    /// Pan center as a fraction of the image, [0,1].
    pub pan_x: f64,
    pub pan_y: f64,
}

impl<'a> ReviewTakeoverProps<'a> {
    /// No composer, no player, whole image.
    pub fn new(shot: &'a Shot, position: DetailPosition) -> Self {
        Self {
            shot,
            position,
            composer: false,
            playing: false,
            busy: false,
            error: None,
            zoom: 1.0,
            pan_x: 0.5,
            pan_y: 0.5,
        }
    }
}

/// The screen's column and row split, in cells. `rail_width` and
/// `stage_width` keep their TS minimums, so they can add up to more than
/// `width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakeoverLayout {
    pub rail_width: u16,
    pub stage_width: u16,
    pub stage_height: u16,
}

impl TakeoverLayout {
    pub fn new(width: u16, height: u16) -> Self {
        let rail_width = if width >= 100 {
            RAIL_WIDTH
        } else {
            ((f64::from(width) * 0.38).floor() as u16).max(28)
        };
        // headerHeight 3 + footerHeight 1.
        Self {
            rail_width,
            stage_width: width.saturating_sub(rail_width + 1).max(10),
            stage_height: height.saturating_sub(4).max(4),
        }
    }
}

// ---- Yoga flex column ------------------------------------------------------

/// One child of a flex column: its flex basis, `flexShrink`, and minimum size.
#[derive(Debug, Clone, Copy)]
struct FlexItem {
    basis: f64,
    shrink: f64,
    min: f64,
}

/// Yoga compares layout floats with this tolerance.
const YOGA_EPSILON: f64 = 0.0001;

/// Yoga's main-axis sizes for a column that is `free` rows short (`free < 0`).
///
/// A port of `distributeFreeSpaceFirstPass` / `SecondPass` for shrinking:
/// the first pass takes children that would fall under their minimum out of
/// the distribution, the second shares what is left. As in Yoga, when the
/// first pass accounts for the whole shortfall the second shrinks nothing and
/// every child keeps its basis.
fn flex_sizes(items: &[FlexItem], free: f64) -> Vec<f64> {
    let mut total: f64 = items.iter().map(|item| -item.shrink * item.basis).sum();
    let mut remaining = free;
    let mut delta = 0.0;
    for item in items {
        let scaled = -item.shrink * item.basis;
        if remaining >= 0.0 || scaled == 0.0 {
            continue;
        }
        let base = item.basis + remaining / total * scaled;
        let bound = base.max(item.min);
        if (base - bound).abs() > YOGA_EPSILON {
            delta += bound - item.basis;
            total -= scaled;
        }
    }
    remaining -= delta;
    items
        .iter()
        .map(|item| {
            let scaled = -item.shrink * item.basis;
            if remaining >= 0.0 || scaled == 0.0 {
                return item.basis;
            }
            let size = if total == 0.0 {
                item.basis + scaled
            } else {
                item.basis + remaining / total * scaled
            };
            size.max(item.min)
        })
        .collect()
}

/// Yoga's `roundValueToPixelGrid` at scale 1: half rounds up. `floor` is the
/// rounding it uses for the position of a text node.
fn round_grid(value: f64, floor: bool) -> i64 {
    let whole = value.floor();
    let fraction = value - whole;
    let up = if fraction < YOGA_EPSILON {
        false
    } else if (fraction - 1.0).abs() < YOGA_EPSILON {
        true
    } else {
        !floor && fraction > 0.5 - YOGA_EPSILON
    };
    whole as i64 + i64::from(up)
}

// ---- state -----------------------------------------------------------------

/// Hook state of the screen's children.
pub struct ReviewTakeoverState {
    services: AppServices,
    wake: Wake,
    playback: Playback,
    backend: Option<Arc<dyn MovieBackend>>,
    picture: Option<PictureState>,
    player: Option<MoviePlayerState>,
    composer: TextInputState,
}

impl ReviewTakeoverState {
    /// `playback` is the parent's handle; the player patches it.
    pub fn new(services: &AppServices, playback: Playback, wake: Wake) -> Self {
        Self {
            services: services.clone(),
            wake,
            playback,
            backend: None,
            picture: None,
            player: None,
            composer: TextInputState::new(),
        }
    }

    /// Same, with the movie backend injected (tests).
    pub fn with_backend(
        services: &AppServices,
        playback: Playback,
        wake: Wake,
        backend: Arc<dyn MovieBackend>,
    ) -> Self {
        Self {
            backend: Some(backend),
            ..Self::new(services, playback, wake)
        }
    }

    pub fn playback(&self) -> &Playback {
        &self.playback
    }

    /// The composer's text so far.
    pub fn composer_value(&self) -> String {
        self.composer.value()
    }

    /// Drop the composer text (the `<TextInput>` unmounted). Rendering with
    /// `composer: false` does the same.
    pub fn reset_composer(&mut self) {
        self.composer = TextInputState::new();
    }

    /// A key while the composer is open: `Submit` is `onComposerSubmit(text)`,
    /// `Cancel` is `onComposerCancel()`.
    pub fn handle(&mut self, key: KeyEvent) -> Outcome {
        self.composer.handle(key)
    }

    /// A bracketed paste while the composer is open.
    pub fn handle_paste(&mut self, input: &str) -> Outcome {
        self.composer.handle_paste(input)
    }

    /// The still on the stage, while it is mounted.
    pub fn picture(&self) -> Option<&PictureState> {
        self.picture.as_ref()
    }

    /// The movie player, while it is mounted.
    pub fn player(&self) -> Option<&MoviePlayerState> {
        self.player.as_ref()
    }
}

// ---- rail ------------------------------------------------------------------

/// Writer over the rail's content column. Rows are offsets from the top of
/// the stage and may fall outside it; everything is clipped to `clip`.
struct Rail<'b> {
    buf: &'b mut Buffer,
    /// The on-screen part of the content column.
    clip: Rect,
    /// Column and row of the content column's top-left cell.
    x: u16,
    y: u16,
    /// `railInner`: the TS layout width of the content column.
    inner: usize,
}

impl Rail<'_> {
    fn row_y(&self, row: i64) -> Option<u16> {
        let y = i64::from(self.y) + row;
        (y >= i64::from(self.clip.top()) && y < i64::from(self.clip.bottom())).then_some(y as u16)
    }

    fn put(&mut self, row: i64, offset: usize, spans: &[Span<'_>]) {
        let x = usize::from(self.x) + offset;
        if let Some(y) = self.row_y(row)
            && x < usize::from(self.clip.right())
        {
            put_spans(self.buf, self.clip, x as u16, y, spans);
        }
    }

    /// One row with `justifyContent="space-between"`.
    fn between(&mut self, row: i64, left: &[Span<'_>], right: &[Span<'_>]) {
        self.put(row, 0, left);
        let start = self
            .inner
            .saturating_sub(spans_width(right))
            .max(spans_width(left));
        self.put(row, start, right);
    }

    /// Copy `columns` of row `from` of `source` (a buffer at the origin, as
    /// wide as the content column) onto rail row `row`.
    fn blit(&mut self, source: &Buffer, from: u16, row: i64, columns: std::ops::Range<u16>) {
        let Some(y) = self.row_y(row) else {
            return;
        };
        for column in columns {
            let x = self.x.saturating_add(column);
            if x >= self.clip.left() && x < self.clip.right() && column < source.area.width {
                self.buf[(x, y)] = source[(column, from)].clone();
            }
        }
    }
}

/// Columns the `<TextInput>` text occupies inside its box (caret included).
fn composer_text_width(state: &TextInputState, width: u16) -> u16 {
    let inner = usize::from(width).saturating_sub(4).max(4);
    let length = state.value().chars().count();
    let text = if length == 0 {
        1 + COMPOSER_PLACEHOLDER.chars().count().min(inner - 1)
    } else {
        let start = (state.cursor() + 1).saturating_sub(inner);
        let visible = length.min(start + inner) - start;
        visible.max(state.cursor() - start + 1)
    };
    text.min(usize::from(u16::MAX)) as u16
}

/// `<ReviewTakeover>`: renders into the `Rect` it is given (the Ink `width`/`height`).
pub struct ReviewTakeover<'a> {
    pub props: ReviewTakeoverProps<'a>,
}

impl<'a> ReviewTakeover<'a> {
    pub fn new(props: ReviewTakeoverProps<'a>) -> Self {
        Self { props }
    }
}

impl ReviewTakeover<'_> {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let props = &self.props;
        let shot = props.shot;
        let width = usize::from(area.width);
        let inner = width.saturating_sub(2);
        // paddingX={1}
        let clip = Rect::new(
            area.x + 1,
            area.y,
            area.width.saturating_sub(2),
            area.height,
        );
        if clip.is_empty() {
            return;
        }
        let muted = fg(THEME.muted);
        let blue = fg(THEME.blue);

        let mut title = vec![styled(
            truncate(&shot.title, width.saturating_sub(40).max(8)),
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if let Some(sequence) = truthy(shot.sequence.as_deref()) {
            title.push(styled("  ", muted));
            title.push(styled(format!(" {sequence} "), muted.bg(THEME.selection)));
        }
        let paging = [
            styled("‹", blue),
            styled(
                format!(" {} / {} ", props.position.index, props.position.count),
                muted,
            ),
            styled("›", blue),
            styled("   esc close", muted),
        ];
        let paging_width = spans_width(&paging);
        let title = truncate_end(title, inner.saturating_sub(paging_width), Style::new());
        let title_end = put_spans(buf, clip, clip.x, area.y, &title);
        let paging_x = (usize::from(clip.x) + inner.saturating_sub(paging_width))
            .max(usize::from(title_end))
            .min(usize::from(u16::MAX)) as u16;
        put_spans(buf, clip, paging_x, area.y, &paging);

        let when = abbreviated_date_time(DateInput::Millis(shot.captured_at));
        let meta = [
            shot.worktree.as_str(),
            shot.feature.as_str(),
            shot.url.as_deref().unwrap_or(""),
            when.as_str(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ");
        put_spans(
            buf,
            clip,
            clip.x,
            area.y.saturating_add(1),
            &[styled(truncate(&meta, inner), muted)],
        );
        put_spans(
            buf,
            clip,
            clip.x,
            area.y.saturating_add(2),
            &[rule(inner, None)],
        );
    }
}

impl StatefulWidget for ReviewTakeover<'_> {
    type State = ReviewTakeoverState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut ReviewTakeoverState) {
        let props = &self.props;
        let shot = props.shot;
        if !props.composer {
            // The `<TextInput>` is unmounted; its text goes with it.
            state.reset_composer();
        }
        if area.width == 0 || area.height == 0 {
            return;
        }

        let layout = TakeoverLayout::new(area.width, area.height);
        let stage_height = layout.stage_height;
        let can_play = shot.is_movie && shot.video_path.is_some();
        let playback = state.playback.snapshot();
        let show_player = can_play && (props.playing || playback.position_ms > 0.0);
        let review_state = review_state_of(shot);
        let stale = shot.review.as_ref().is_some_and(|review| review.is_stale);
        let rail_inner = usize::from(layout.rail_width - 2);
        let image_height = stage_height - u16::from(show_player);

        self.render_header(area, buf);

        // Stage: the player and its progress bar, or the picture.
        let stage_y = area.y.saturating_add(3);
        let image = Rect::new(area.x, stage_y, layout.stage_width, image_height).intersection(area);
        let on_screen = stage_y < area.bottom() && !image.is_empty();
        if show_player {
            state.picture = None;
            let (services, wake, backend) = (&state.services, &state.wake, &state.backend);
            let handle = &state.playback;
            let player = state.player.get_or_insert_with(|| match backend {
                Some(backend) => MoviePlayerState::with_backend(
                    services,
                    handle.clone(),
                    wake.clone(),
                    backend.clone(),
                ),
                None => MoviePlayerState::new(services, handle.clone(), wake.clone()),
            });
            if on_screen {
                MoviePlayer {
                    video_path: shot.video_path.as_deref().expect("can_play checked it"),
                    poster_path: &shot.path,
                    poster_version: shot.mtime_ms,
                }
                .render(image, buf, player);
            }
            // `width={stageWidth - 2}`, centered in the stage column.
            let bar = Rect::new(
                area.x.saturating_add(1),
                stage_y.saturating_add(image_height),
                layout.stage_width - 2,
                1,
            )
            .intersection(area);
            if bar.y < area.bottom() && !bar.is_empty() {
                ProgressBar {
                    position_ms: playback.position_ms,
                    duration_ms: playback.duration_ms.or(shot.duration_ms),
                    chapters: &shot.chapters,
                    playing: playback.playing,
                }
                .render(bar, buf);
            }
        } else {
            state.player = None;
            let (services, wake) = (&state.services, &state.wake);
            let picture = state
                .picture
                .get_or_insert_with(|| PictureState::new(services, wake.clone()));
            if on_screen {
                Picture::new(PictureProps {
                    src: Some(&shot.path),
                    version: shot.mtime_ms,
                    label: Some(if shot.is_movie {
                        "▶ movie poster"
                    } else {
                        "Screenshot unavailable"
                    }),
                    max_upscale: 8.0,
                    zoom: props.zoom,
                    pan_x: props.pan_x,
                    pan_y: props.pan_y,
                    ..PictureProps::default()
                })
                .render(image, buf, picture);
            }
        }

        // Divider between the stage and the rail.
        let divider_x = area.x.saturating_add(layout.stage_width);
        let stage_bottom = stage_y.saturating_add(stage_height).min(area.bottom());
        if divider_x < area.right() {
            for y in stage_y..stage_bottom {
                buf[(divider_x, y)]
                    .set_symbol("│")
                    .set_style(fg(THEME.faint));
            }
        }

        // Rail: paddingX={1}, clipped to the stage.
        let rail_x = divider_x.saturating_add(2);
        let clip = Rect::new(rail_x, stage_y, rail_inner as u16, stage_height).intersection(area);
        if stage_y >= area.bottom() || rail_x >= area.right() || clip.is_empty() {
            return;
        }
        let mut rail = Rail {
            buf,
            clip,
            x: rail_x,
            y: stage_y,
            inner: rail_inner,
        };
        let muted = fg(THEME.muted);

        let mut row: i64 = 0;
        rail.between(
            row,
            &[styled("Review", Style::new().add_modifier(Modifier::BOLD))],
            &[review_badge(review_state, stale)],
        );
        row += 1;
        if stale {
            for line in ink_wrap(STALE_NOTE, rail_inner).into_iter().take(2) {
                rail.put(row, 0, &[styled(line, fg(THEME.amber))]);
                row += 1;
            }
            // The box is two rows whatever the note wraps to.
            row = 3;
        }
        let description = if shot.description.is_empty() {
            &shot.file_name
        } else {
            &shot.description
        };
        // Reserved from the raw length; the wrapped text is clipped to it.
        let description_rows = description.chars().count().div_ceil(rail_inner).clamp(1, 3);
        let wrapped = ink_wrap(&truncate(description, rail_inner * 3), rail_inner);
        for (index, line) in wrapped.into_iter().take(description_rows).enumerate() {
            rail.put(row + index as i64, 0, &[styled(line, muted)]);
        }
        row += description_rows as i64;
        rail.put(row, 0, &[rule(rail_inner, None)]);
        row += 1;
        let comment_count = shot
            .review
            .as_ref()
            .map_or(0, |review| review.comments.len());
        rail.between(
            row,
            &[section_label("FEEDBACK")],
            &[styled(comment_count.to_string(), muted)],
        );
        row += 1;

        // The flexible part: the comment list and, when open, the composer.
        let comments = CommentList {
            shot,
            // `Math.max(2, Math.floor((stageHeight - 12) / 3))`
            max: usize::from(stage_height.saturating_sub(12) / 3).max(2),
        };
        let comments_basis = f64::from(comments.height(rail_inner as u16));
        let error = truthy(props.error);
        let hint = format!("  ⏎ {COMPOSER_SUBMIT_LABEL}  ·  esc cancel");
        let hint_lines = ink_wrap(&hint, rail_inner);
        let composer_basis = 3.0 + hint_lines.len() as f64;
        let status_rows = i64::from(props.busy) + i64::from(error.is_some());
        let above = row as f64;
        let fixed_below = 1.0 + status_rows as f64 + if props.composer { 0.0 } else { 2.0 };
        let mut items = vec![FlexItem {
            basis: comments_basis,
            shrink: 100.0,
            min: 0.0,
        }];
        if props.composer {
            items.push(FlexItem {
                basis: composer_basis,
                shrink: 1.0,
                min: 0.0,
            });
        }
        let basis_total: f64 = items.iter().map(|item| item.basis).sum();
        let free = f64::from(stage_height) - above - fixed_below - basis_total;
        let sizes = if free >= 0.0 {
            // `flexGrow={1}`: the comment list takes the rows that are left.
            let mut sizes: Vec<f64> = items.iter().map(|item| item.basis).collect();
            sizes[0] += free;
            sizes
        } else {
            flex_sizes(&items, free)
        };

        let comments_end = above + sizes[0];
        let comment_rows = round_grid(comments_end, false) - row;
        let width = rail_inner as u16;
        if comment_rows > 0 {
            // Laid out at the rail's width even where the screen cuts it off.
            let rows = comment_rows.min(i64::from(stage_height)) as u16;
            let mut scratch = Buffer::empty(Rect::new(0, 0, width, rows));
            comments.render(scratch.area, &mut scratch);
            for index in 0..rows {
                rail.blit(&scratch, index, row + i64::from(index), 0..width);
            }
        }
        let mut at = comments_end;
        rail.put(round_grid(at, false), 0, &[rule(rail_inner, None)]);
        at += 1.0;
        if props.busy {
            rail.put(
                round_grid(at, false),
                0,
                &first_wrapped_row(vec![styled("Saving review…", muted)], rail_inner),
            );
            at += 1.0;
        }
        if let Some(error) = error {
            rail.put(
                round_grid(at, false),
                0,
                &first_wrapped_row(
                    vec![styled(truncate(error, rail_inner), fg(THEME.red))],
                    rail_inner,
                ),
            );
            at += 1.0;
        }
        let top = round_grid(at, false);

        if !props.composer {
            let plain: Spans = vec![
                styled(" c ", bold(THEME.blue)),
                Span::raw("Send Feedback"),
                styled("   s ", bold(THEME.green)),
                Span::raw("Seen"),
            ];
            rail.put(top, 0, &first_wrapped_row(plain, rail_inner));
            let keys = if props.zoom > 1.0 {
                "← ↑ → ↓ pan · +/- zoom · 0 reset · esc close"
            } else if can_play {
                "← → page · space play · +/- zoom · [ ] chapter"
            } else {
                "← → page · +/- zoom · c send · s seen"
            };
            rail.put(
                top + 1,
                0,
                &truncate_end(vec![styled(keys, muted)], rail_inner, muted),
            );
            return;
        }

        // Composer: a bordered box (3 rows, never under its 2 border rows)
        // and the hint text, sharing whatever height the rail left them.
        let parts = [
            FlexItem {
                basis: 3.0,
                shrink: 1.0,
                min: 2.0,
            },
            FlexItem {
                basis: hint_lines.len() as f64,
                shrink: 1.0,
                min: 0.0,
            },
        ];
        let composer_free = sizes[1] - composer_basis;
        let part_sizes = if composer_free >= 0.0 {
            vec![parts[0].basis, parts[1].basis]
        } else {
            flex_sizes(&parts, composer_free)
        };
        let box_rows = round_grid(at + part_sizes[0], false) - top;
        let mut scratch = Buffer::empty(Rect::new(0, 0, width, 3));
        TextInput {
            placeholder: COMPOSER_PLACEHOLDER,
            submit_label: Some(COMPOSER_SUBMIT_LABEL),
        }
        .render(scratch.area, &mut scratch, &mut state.composer);
        rail.blit(&scratch, 0, top, 0..width);
        if box_rows >= 3 {
            rail.blit(&scratch, 1, top + 1, 0..width);
            rail.blit(&scratch, 2, top + 2, 0..width);
        } else {
            // No content row is left: the text lands on the bottom border.
            rail.blit(&scratch, 2, top + 1, 0..width);
            let text = composer_text_width(&state.composer, width);
            rail.blit(&scratch, 1, top + 1, 2..2u16.saturating_add(text));
        }
        // A text node's position rounds down, so a box that lost any part of
        // a row pulls the hint up onto its last row.
        let hint_top = top + round_grid(part_sizes[0], true);
        for (index, line) in hint_lines.into_iter().enumerate() {
            rail.put(hint_top + index as i64, 0, &[styled(line, muted)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::probe::GraphicsProtocol;
    use crate::ui::context::test_support::{FakeService, services};
    use crate::ui::movie_player::{FrameSource, PlaybackPatch, PlaybackState, ProbeFuture};
    use crate::ui::testing::{bg_at, fg_at, modifier_at, render_stateful, rows};
    use crate::video::ffmpeg::FramePlayerOptions;
    use astroshot_engine::review_data::model::{
        Chapter, ReviewComment, ReviewSnapshot, ReviewState,
    };
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::style::Color;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The expected rows below are frames captured from the Ink
    // `<ReviewTakeover>` (graphics "none") at the same size and props. Rows
    // that show a date are built with `meta_row` / `reviewer_row`, since the
    // component formats it in the local timezone. Where Ink painted rail rows
    // below the stage (see the module docs) the expected row is blank.

    // ---- fixtures --------------------------------------------------------

    const CAPTURED_AT: f64 = 1_786_460_040_123.0;

    fn still() -> Shot {
        Shot {
            id: "/w/.astroshot/feat/0001-home.png".into(),
            path: "/w/.astroshot/feat/0001-home.png".into(),
            file_name: "0001-home.png".into(),
            worktree_path: "/w".into(),
            worktree: "firstlanding-wt8".into(),
            worktree_short: "wt8".into(),
            feature: "feat".into(),
            feature_dir: "/w/.astroshot/feat".into(),
            sequence: Some("0001".into()),
            slug: "home".into(),
            title: "Home".into(),
            description: "The home page after login".into(),
            url: None,
            run_id: None,
            status: None,
            captured_at: CAPTURED_AT,
            mtime_ms: 1.0,
            is_movie: false,
            video_file_name: None,
            video_path: None,
            duration_ms: None,
            source: None,
            chapters: Vec::new(),
            review: None,
        }
    }

    fn chapter(slug: Option<&str>, title: Option<&str>, t_ms: Option<f64>) -> Chapter {
        Chapter {
            slug: slug.map(str::to_string),
            title: title.map(str::to_string),
            t_ms,
        }
    }

    fn movie() -> Shot {
        Shot {
            is_movie: true,
            video_file_name: Some("0001-home.webm".into()),
            video_path: Some("/w/.astroshot/feat/0001-home.webm".into()),
            duration_ms: Some(12_000.0),
            source: Some("browser".into()),
            chapters: vec![
                chapter(Some("a"), Some("Open"), Some(0.0)),
                chapter(Some("b"), None, Some(1_500.0)),
                chapter(None, None, Some(65_000.0)),
                chapter(None, Some("x"), Some(70_000.0)),
                chapter(None, Some("y"), None),
            ],
            ..still()
        }
    }

    const CREATED_AT: &str = "2026-08-11T14:54:00Z";

    fn review(bodies: &[&str], state: ReviewState, is_stale: bool) -> Option<ReviewSnapshot> {
        Some(ReviewSnapshot {
            state,
            decision: None,
            hash_matches: true,
            is_stale,
            comments: bodies
                .iter()
                .enumerate()
                .map(|(index, body)| ReviewComment {
                    id: format!("c{index}"),
                    body: body.to_string(),
                    created_at: CREATED_AT.into(),
                })
                .collect(),
            reviewed_at: None,
        })
    }

    const FOX: &str = "The quick brown fox jumps over the lazy dog and keeps running through the forest until it reaches the river bank where it stops to drink some water and rest for a while before going on again and again and again";
    const LONG_TITLE: &str = "A very long title that goes on and on and on and on and on and on and on and on and on and on and on and on";
    const LONG_DESCRIPTION: &str = "A much longer description of this particular frame that certainly needs more than a single rail row to be shown in full, and then some more words to go past three";
    const LONG_ERROR: &str = "Could not write review.json: EACCES permission denied, open /w/.astroshot/feat/review.json";

    /// `text` cut to `width` characters, the last one being `…`.
    fn cut(text: &str, width: usize) -> String {
        if text.chars().count() <= width {
            return text.to_string();
        }
        let head: String = text.chars().take(width - 1).collect();
        format!("{head}…")
    }

    /// The metadata row: `prefix` and the capture time in this machine's timezone.
    fn meta_row(prefix: &str, width: usize) -> String {
        let when = abbreviated_date_time(DateInput::Millis(CAPTURED_AT));
        format!(" {}", cut(&format!("{prefix} · {when}"), width - 2))
    }

    /// A comment header row in the rail of a `width` x `height` screen, with
    /// `left` on the stage.
    fn reviewer_row(left: &str, width: u16, height: u16) -> String {
        let layout = TakeoverLayout::new(width, height);
        let when = abbreviated_date_time(DateInput::Text(CREATED_AT));
        format!(
            "{left:<stage$}│ {}",
            cut(
                &format!("Reviewer · {when}"),
                usize::from(layout.rail_width - 2)
            ),
            stage = usize::from(layout.stage_width),
        )
    }

    struct NoMovie;

    impl MovieBackend for NoMovie {
        fn probe(&self, _video_path: &str) -> ProbeFuture {
            Box::pin(async { None })
        }

        fn frame_player(&self, _options: FramePlayerOptions) -> Box<dyn FrameSource> {
            unreachable!("nothing decodes without kitty graphics")
        }
    }

    struct Harness {
        state: ReviewTakeoverState,
        wakes: Arc<AtomicUsize>,
    }

    fn harness() -> Harness {
        let services = services(GraphicsProtocol::None, FakeService::new(false));
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let wake: Wake = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let playback = Playback::new(PlaybackState::default(), wake.clone());
        Harness {
            state: ReviewTakeoverState::with_backend(&services, playback, wake, Arc::new(NoMovie)),
            wakes,
        }
    }

    fn props(shot: &Shot) -> ReviewTakeoverProps<'_> {
        ReviewTakeoverProps::new(shot, DetailPosition { index: 2, count: 7 })
    }

    impl Harness {
        fn draw(&mut self, props: ReviewTakeoverProps<'_>, width: u16, height: u16) -> Buffer {
            render_stateful(ReviewTakeover::new(props), &mut self.state, width, height)
        }

        fn rows(&mut self, props: ReviewTakeoverProps<'_>, width: u16, height: u16) -> Vec<String> {
            rows(&self.draw(props, width, height))
        }

        /// The parent's `ui.playback`.
        fn playback(&self, playing: bool, position_ms: f64, duration_ms: Option<f64>) {
            self.state.playback().patch(&PlaybackPatch {
                playing: Some(playing),
                position_ms: Some(position_ms),
                duration_ms: Some(duration_ms),
                ..PlaybackPatch::default()
            });
        }
    }

    fn press(state: &mut ReviewTakeoverState, code: KeyCode) -> Outcome {
        state.handle(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(state: &mut ReviewTakeoverState, text: &str) {
        for c in text.chars() {
            assert_eq!(press(state, KeyCode::Char(c)), Outcome::Pending);
        }
    }

    // ---- layout arithmetic -------------------------------------------------

    #[test]
    fn layout_splits_the_screen_like_the_ts_arithmetic() {
        let layout = |width, height| {
            let l = TakeoverLayout::new(width, height);
            (l.rail_width, l.stage_width, l.stage_height)
        };
        assert_eq!(layout(120, 30), (42, 77, 26));
        assert_eq!(layout(100, 20), (42, 57, 16));
        // Under 100 columns the rail is 38% of the width, at least 28.
        assert_eq!(layout(99, 20), (37, 61, 16));
        assert_eq!(layout(80, 24), (30, 49, 20));
        assert_eq!(layout(74, 24), (28, 45, 20));
        assert_eq!(layout(60, 8), (28, 31, 4));
        // The stage keeps 10 columns and 4 rows however small the screen.
        assert_eq!(layout(30, 6), (28, 10, 4));
        assert_eq!(layout(0, 0), (28, 10, 4));
    }

    fn sizes(items: &[(f64, f64, f64)], free: f64) -> Vec<f64> {
        let items: Vec<FlexItem> = items
            .iter()
            .map(|&(basis, shrink, min)| FlexItem { basis, shrink, min })
            .collect();
        flex_sizes(&items, free)
            .into_iter()
            .map(|size| (size * 1000.0).round() / 1000.0)
            .collect()
    }

    #[test]
    fn flex_sizes_shrinks_in_proportion_to_shrink_times_basis() {
        // Comment list (100) against the composer (1): the list takes nearly all of it.
        assert_eq!(
            sizes(&[(9.0, 100.0, 0.0), (4.0, 1.0, 0.0)], -4.0),
            [5.018, 3.982]
        );
        // Box and hint inside the composer.
        assert_eq!(
            sizes(&[(3.0, 1.0, 2.0), (1.0, 1.0, 0.0)], -1.0),
            [2.25, 0.75]
        );
        // A lone shrinkable child absorbs the shortfall, down to its minimum.
        assert_eq!(sizes(&[(5.0, 100.0, 0.0)], -2.0), [3.0]);
        assert_eq!(sizes(&[(5.0, 100.0, 0.0)], -9.0), [0.0]);
    }

    #[test]
    fn flex_sizes_freezes_children_at_their_minimum_like_yoga() {
        // The list hits zero in the first pass; the composer takes the rest.
        assert_eq!(
            sizes(&[(1.0, 100.0, 0.0), (4.0, 1.0, 0.0)], -2.0),
            [0.0, 3.0]
        );
        assert_eq!(
            sizes(&[(1.0, 100.0, 0.0), (4.0, 1.0, 0.0)], -4.0),
            [0.0, 1.0]
        );
        // The box cannot go under its two border rows; the hint gets nothing.
        assert_eq!(sizes(&[(3.0, 1.0, 2.0), (1.0, 1.0, 0.0)], -3.0), [2.0, 0.0]);
        // Yoga quirk: when the first pass accounts for the whole shortfall,
        // the second pass shrinks nothing and everything keeps its basis.
        assert_eq!(
            sizes(&[(1.0, 100.0, 0.0), (4.0, 1.0, 0.0)], -5.0),
            [1.0, 4.0]
        );
        assert_eq!(sizes(&[(3.0, 1.0, 2.0), (1.0, 1.0, 0.0)], -2.0), [3.0, 1.0]);
    }

    #[test]
    fn round_grid_rounds_half_up_and_floors_text_positions() {
        assert_eq!(round_grid(2.0, false), 2);
        assert_eq!(round_grid(2.25, false), 2);
        assert_eq!(round_grid(2.5, false), 3);
        assert_eq!(round_grid(2.75, false), 3);
        assert_eq!(round_grid(2.75, true), 2);
        assert_eq!(round_grid(2.987, true), 2);
        // Within Yoga's tolerance of a whole number either way.
        assert_eq!(round_grid(2.99999, true), 3);
        assert_eq!(round_grid(3.00001, false), 3);
    }

    // ---- frames ------------------------------------------------------------

    #[tokio::test]
    async fn still_shot_renders_the_ink_frame() {
        let shot = still();
        let mut h = harness();
        let lines = h.rows(props(&shot), 100, 20);
        assert_eq!(
            lines,
            [
                " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                &meta_row("firstlanding-wt8 · feat", 100),
                " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                "                                                         │ Review                          ● Unseen",
                "                                                         │ The home page after login",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │ FEEDBACK                               0",
                "                                                         │ No comments yet · leave concise, action…",
                "                                                         │",
                "                                                         │",
                "                 Screenshot unavailable                  │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │  c Send Feedback   s Seen",
                "                                                         │ ← → page · +/- zoom · c send · s seen",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn rail_is_38_percent_of_a_terminal_narrower_than_100_columns() {
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(props(&shot), 80, 24);
            assert_eq!(
                lines,
                [
                    " Home   0001                                              ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 80),
                    " ──────────────────────────────────────────────────────────────────────────────",
                    "                                                 │ Review              ● Unseen",
                    "                                                 │ The home page after login",
                    "                                                 │ ────────────────────────────",
                    "                                                 │ FEEDBACK                   0",
                    "                                                 │ No comments yet · leave con…",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "             Screenshot unavailable              │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │ ────────────────────────────",
                    "                                                 │  c Send Feedback   s Seen",
                    "                                                 │ ← → page · +/- zoom · c sen…",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(props(&shot), 72, 16);
            assert_eq!(
                lines,
                [
                    " Home   0001                                      ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 72),
                    " ──────────────────────────────────────────────────────────────────────",
                    "                                           │ Review            ● Unseen",
                    "                                           │ The home page after login",
                    "                                           │ ──────────────────────────",
                    "                                           │ FEEDBACK                 0",
                    "                                           │ No comments yet · leave c…",
                    "          Screenshot unavailable           │",
                    "                                           │",
                    "                                           │",
                    "                                           │",
                    "                                           │ ──────────────────────────",
                    "                                           │  c Send Feedback   s Seen",
                    "                                           │ ← → page · +/- zoom · c s…",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(props(&shot), 40, 12);
            assert_eq!(
                lines,
                [
                    " Home   0001      ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 40),
                    " ──────────────────────────────────────",
                    "           │ Review            ● Unseen",
                    "           │ The home page after login",
                    "           │ ──────────────────────────",
                    "Screenshot…│ FEEDBACK                 0",
                    "           │ No comments yet · leave c…",
                    "           │ ──────────────────────────",
                    "           │  c Send Feedback   s Seen",
                    "           │ ← → page · +/- zoom · c s…",
                    "",
                ]
            );
        }
    }

    #[tokio::test]
    async fn long_title_url_and_description_are_cut_to_their_rows() {
        let shot = Shot {
            title: LONG_TITLE.into(),
            url: Some("https://x.test/a".into()),
            description: LONG_DESCRIPTION.into(),
            review: review(&[], ReviewState::Seen, false),
            ..still()
        };
        let mut h = harness();
        let lines = h.rows(props(&shot), 99, 20);
        assert_eq!(
            lines,
            [
                " A very long title that goes on and on and on and on and on…   0001          ‹ 2 / 7 ›   esc close",
                &meta_row("firstlanding-wt8 · feat · https://x.test/a", 99),
                " ─────────────────────────────────────────────────────────────────────────────────────────────────",
                "                                                             │ Review                       ● Seen",
                "                                                             │ A much longer description of this",
                "                                                             │ particular frame that certainly",
                "                                                             │ needs more than a single rail row",
                "                                                             │ ───────────────────────────────────",
                "                                                             │ FEEDBACK                          0",
                "                                                             │ No comments yet · leave concise, a…",
                "                   Screenshot unavailable                    │",
                "                                                             │",
                "                                                             │",
                "                                                             │",
                "                                                             │",
                "                                                             │",
                "                                                             │ ───────────────────────────────────",
                "                                                             │  c Send Feedback   s Seen",
                "                                                             │ ← → page · +/- zoom · c send · s s…",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn empty_strings_count_as_missing_and_a_stale_review_adds_its_note() {
        let shot = Shot {
            sequence: Some(String::new()),
            worktree: String::new(),
            url: Some(String::new()),
            description: String::new(),
            review: review(&[], ReviewState::Pending, true),
            ..still()
        };
        let mut h = harness();
        let lines = h.rows(props(&shot), 100, 20);
        assert_eq!(
            lines,
            [
                " Home                                                                         ‹ 2 / 7 ›   esc close",
                &meta_row("feat", 100),
                " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                "                                                         │ Review                ● Unseen · changed",
                "                                                         │ A newer image was captured since this",
                "                                                         │ was seen.",
                "                                                         │ 0001-home.png",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │ FEEDBACK                               0",
                "                                                         │ No comments yet · leave concise, action…",
                "                 Screenshot unavailable                  │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │  c Send Feedback   s Seen",
                "                                                         │ ← → page · +/- zoom · c send · s seen",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn stale_note_and_description_wrap_like_ink_in_a_narrow_rail() {
        {
            let shot = Shot {
                description: LONG_DESCRIPTION.into(),
                review: review(&["one"], ReviewState::Pending, true),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(props(&shot), 80, 24);
            assert_eq!(
                lines,
                [
                    " Home   0001                                              ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 80),
                    " ──────────────────────────────────────────────────────────────────────────────",
                    "                                                 │ Review    ● Unseen · changed",
                    "                                                 │ A newer image was captured",
                    "                                                 │ since this was seen.",
                    "                                                 │ A much longer description of",
                    "                                                 │  this particular frame that",
                    "                                                 │ certainly needs more than a…",
                    "                                                 │ ────────────────────────────",
                    "                                                 │ FEEDBACK                   1",
                    &reviewer_row("", 80, 24),
                    "             Screenshot unavailable              │ one",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │ ────────────────────────────",
                    "                                                 │  c Send Feedback   s Seen",
                    "                                                 │ ← → page · +/- zoom · c sen…",
                    "",
                ]
            );
        }
        {
            let shot = Shot {
                review: review(&["one"], ReviewState::Pending, true),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(props(&shot), 60, 16);
            assert_eq!(
                lines,
                [
                    " Home   0001                          ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 60),
                    " ──────────────────────────────────────────────────────────",
                    "                               │ Review  ● Unseen · changed",
                    "                               │ A newer image was captured",
                    "                               │  since this was seen.",
                    "                               │ The home page after login",
                    "                               │ ──────────────────────────",
                    "    Screenshot unavailable     │ FEEDBACK                 1",
                    &reviewer_row("", 60, 16),
                    "                               │ one",
                    "                               │",
                    "                               │ ──────────────────────────",
                    "                               │  c Send Feedback   s Seen",
                    "                               │ ← → page · +/- zoom · c s…",
                    "",
                ]
            );
        }
    }

    #[tokio::test]
    async fn hint_row_names_the_keys_for_the_zoom_and_shot_kind() {
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    zoom: 2.0,
                    ..props(&shot)
                },
                100,
                20,
            );
            assert_eq!(
                lines[10],
                "                 Screenshot unavailable                  │"
            );
            assert_eq!(
                lines[18],
                "                                                         │ ← ↑ → ↓ pan · +/- zoom · 0 reset · esc …"
            );
        }
        {
            let shot = movie();
            let mut h = harness();
            let lines = h.rows(props(&shot), 100, 20);
            assert_eq!(
                lines[10],
                "                     ▶ movie poster                      │"
            );
            assert_eq!(
                lines[18],
                "                                                         │ ← → page · space play · +/- zoom · [ ] …"
            );
        }
        {
            let shot = movie();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    zoom: 1.5,
                    ..props(&shot)
                },
                100,
                20,
            );
            assert_eq!(
                lines[10],
                "                     ▶ movie poster                      │"
            );
            assert_eq!(
                lines[18],
                "                                                         │ ← ↑ → ↓ pan · +/- zoom · 0 reset · esc …"
            );
        }
        {
            let shot = Shot {
                video_path: None,
                ..movie()
            };
            let mut h = harness();
            h.playback(true, 3_000.0, None);
            let lines = h.rows(
                ReviewTakeoverProps {
                    playing: true,
                    ..props(&shot)
                },
                100,
                20,
            );
            assert_eq!(
                lines[10],
                "                     ▶ movie poster                      │"
            );
            assert_eq!(
                lines[18],
                "                                                         │ ← → page · +/- zoom · c send · s seen"
            );
        }
    }

    #[tokio::test]
    async fn playing_movie_shows_the_player_and_the_progress_bar() {
        let shot = movie();
        let mut h = harness();
        h.playback(true, 3_000.0, None);
        let lines = h.rows(
            ReviewTakeoverProps {
                playing: true,
                ..props(&shot)
            },
            100,
            20,
        );
        assert_eq!(
            lines,
            [
                " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                &meta_row("firstlanding-wt8 · feat", 100),
                " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                "                                                         │ Review                          ● Unseen",
                "                                                         │ The home page after login",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │ FEEDBACK                               0",
                "                                                         │ No comments yet · leave concise, action…",
                "                                                         │",
                "                                                         │",
                "         Poster shown · press O to open the movie        │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │  c Send Feedback   s Seen",
                " ▶ ┼━━━━┼━━━━●──────────────────────────┼ 0:03 / 0:12    │ ← → page · space play · +/- zoom · [ ] …",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn player_stays_while_paused_past_the_start_and_prefers_the_probed_duration() {
        let shot = movie();
        let mut h = harness();
        h.playback(false, 6_000.0, Some(60_000.0));
        let lines = h.rows(props(&shot), 81, 25);
        assert_eq!(
            lines,
            [
                " Home   0001                                               ‹ 2 / 7 ›   esc close",
                &meta_row("firstlanding-wt8 · feat", 81),
                " ───────────────────────────────────────────────────────────────────────────────",
                "                                                  │ Review              ● Unseen",
                "                                                  │ The home page after login",
                "                                                  │ ────────────────────────────",
                "                                                  │ FEEDBACK                   0",
                "                                                  │ No comments yet · leave con…",
                "                                                  │",
                "                                                  │",
                "                                                  │",
                "                                                  │",
                "                     ▶ movie                      │",
                "     Poster shown · press O to open the movie     │",
                "                                                  │",
                "                                                  │",
                "                                                  │",
                "                                                  │",
                "                                                  │",
                "                                                  │",
                "                                                  │",
                "                                                  │ ────────────────────────────",
                "                                                  │  c Send Feedback   s Seen",
                " ⏸ ┼┼━●──────────────────────────┼ 0:06 / 1:00    │ ← → page · space play · +/-…",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn player_at_the_start_shows_while_the_parent_says_playing() {
        let shot = movie();
        let mut h = harness();
        h.playback(true, 0.0, None);
        let lines = h.rows(
            ReviewTakeoverProps {
                playing: true,
                ..props(&shot)
            },
            100,
            21,
        );
        assert_eq!(
            lines[10],
            "                         ▶ movie                         │"
        );
        assert_eq!(
            lines[11],
            "         Poster shown · press O to open the movie        │"
        );
        assert_eq!(
            lines[19],
            " ▶ ●────┼───────────────────────────────┼ 0:00 / 0:12    │ ← → page · space play · +/- zoom · [ ] …"
        );
    }

    #[tokio::test]
    async fn comments_fill_the_rail_between_the_feedback_row_and_the_actions() {
        {
            let shot = Shot {
                review: review(
                    &["one", "two words here", &"x".repeat(70), FOX, "last"],
                    ReviewState::Pending,
                    true,
                ),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(props(&shot), 100, 30);
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                ● Unseen · changed",
                    "                                                         │ A newer image was captured since this",
                    "                                                         │ was seen.",
                    "                                                         │ The home page after login",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ FEEDBACK                               5",
                    "                                                         │ … 1 earlier",
                    &reviewer_row("", 100, 30),
                    "                                                         │ two words here",
                    &reviewer_row("", 100, 30),
                    "                                                         │ xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                    "                                                         │ xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                    &reviewer_row("                 Screenshot unavailable", 100, 30),
                    "                                                         │ The quick brown fox jumps over the lazy",
                    "                                                         │ dog and keeps running through the forest",
                    "                                                         │  until it reaches the river bank where …",
                    &reviewer_row("", 100, 30),
                    "                                                         │ last",
                    "                                                         │",
                    "                                                         │",
                    "                                                         │",
                    "                                                         │",
                    "                                                         │",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │  c Send Feedback   s Seen",
                    "                                                         │ ← → page · +/- zoom · c send · s seen",
                    "",
                ]
            );
        }
        {
            let shot = Shot {
                review: review(
                    &["one", "two words here", &"x".repeat(70), FOX, "last"],
                    ReviewState::Pending,
                    true,
                ),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(props(&shot), 100, 20);
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                ● Unseen · changed",
                    "                                                         │ A newer image was captured since this",
                    "                                                         │ was seen.",
                    "                                                         │ The home page after login",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ FEEDBACK                               5",
                    "                                                         │ … 3 earlier",
                    &reviewer_row("                 Screenshot unavailable", 100, 20),
                    "                                                         │ The quick brown fox jumps over the lazy",
                    "                                                         │ dog and keeps running through the forest",
                    "                                                         │  until it reaches the river bank where …",
                    &reviewer_row("", 100, 20),
                    "                                                         │ last",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │  c Send Feedback   s Seen",
                    "                                                         │ ← → page · +/- zoom · c send · s seen",
                    "",
                ]
            );
        }
        {
            let shot = Shot {
                review: review(&["one", "two words here", FOX], ReviewState::Seen, false),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(props(&shot), 80, 24);
            assert_eq!(
                lines,
                [
                    " Home   0001                                              ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 80),
                    " ──────────────────────────────────────────────────────────────────────────────",
                    "                                                 │ Review                ● Seen",
                    "                                                 │ The home page after login",
                    "                                                 │ ────────────────────────────",
                    "                                                 │ FEEDBACK                   3",
                    "                                                 │ … 1 earlier",
                    &reviewer_row("", 80, 24),
                    "                                                 │ two words here",
                    &reviewer_row("", 80, 24),
                    "                                                 │ The quick brown fox jumps",
                    "             Screenshot unavailable              │ over the lazy dog and keeps",
                    "                                                 │ running through the forest",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │",
                    "                                                 │ ────────────────────────────",
                    "                                                 │  c Send Feedback   s Seen",
                    "                                                 │ ← → page · +/- zoom · c sen…",
                    "",
                ]
            );
        }
    }

    #[tokio::test]
    async fn comment_list_gives_up_its_rows_first_on_a_short_terminal() {
        let shot = Shot {
            review: review(&["one", "two words here", FOX], ReviewState::Seen, false),
            ..still()
        };
        let mut h = harness();
        let lines = h.rows(props(&shot), 100, 12);
        assert_eq!(
            lines,
            [
                " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                &meta_row("firstlanding-wt8 · feat", 100),
                " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                "                                                         │ Review                            ● Seen",
                "                                                         │ The home page after login",
                "                                                         │ ────────────────────────────────────────",
                "                 Screenshot unavailable                  │ FEEDBACK                               3",
                "                                                         │ … 1 earlier",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │  c Send Feedback   s Seen",
                "                                                         │ ← → page · +/- zoom · c send · s seen",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn composer_replaces_the_action_rows() {
        let shot = still();
        let mut h = harness();
        let lines = h.rows(
            ReviewTakeoverProps {
                composer: true,
                ..props(&shot)
            },
            100,
            20,
        );
        assert_eq!(
            lines,
            [
                " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                &meta_row("firstlanding-wt8 · feat", 100),
                " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                "                                                         │ Review                          ● Unseen",
                "                                                         │ The home page after login",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │ FEEDBACK                               0",
                "                                                         │ No comments yet · leave concise, action…",
                "                                                         │",
                "                                                         │",
                "                 Screenshot unavailable                  │",
                "                                                         │",
                "                                                         │",
                "                                                         │",
                "                                                         │ ────────────────────────────────────────",
                "                                                         │ ╭──────────────────────────────────────╮",
                "                                                         │ │  Share feedback…                     │",
                "                                                         │ ╰──────────────────────────────────────╯",
                "                                                         │   ⏎ Send Feedback  ·  esc cancel",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn composer_hint_wraps_in_a_rail_narrower_than_34_columns() {
        let shot = still();
        let mut h = harness();
        let lines = h.rows(
            ReviewTakeoverProps {
                composer: true,
                ..props(&shot)
            },
            80,
            24,
        );
        assert_eq!(
            lines,
            [
                " Home   0001                                              ‹ 2 / 7 ›   esc close",
                &meta_row("firstlanding-wt8 · feat", 80),
                " ──────────────────────────────────────────────────────────────────────────────",
                "                                                 │ Review              ● Unseen",
                "                                                 │ The home page after login",
                "                                                 │ ────────────────────────────",
                "                                                 │ FEEDBACK                   0",
                "                                                 │ No comments yet · leave con…",
                "                                                 │",
                "                                                 │",
                "                                                 │",
                "                                                 │",
                "             Screenshot unavailable              │",
                "                                                 │",
                "                                                 │",
                "                                                 │",
                "                                                 │",
                "                                                 │ ────────────────────────────",
                "                                                 │ ╭──────────────────────────╮",
                "                                                 │ │  Share feedback…         │",
                "                                                 │ ╰──────────────────────────╯",
                "                                                 │   ⏎ Send Feedback  ·  esc",
                "                                                 │ cancel",
                "",
            ]
        );
    }

    #[tokio::test]
    async fn composer_hint_lands_on_the_box_border_when_comments_are_clipped() {
        {
            let shot = Shot {
                review: review(
                    &["one", "two words here", &"x".repeat(70), FOX, "last"],
                    ReviewState::Pending,
                    true,
                ),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    composer: true,
                    ..props(&shot)
                },
                100,
                20,
            );
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                ● Unseen · changed",
                    "                                                         │ A newer image was captured since this",
                    "                                                         │ was seen.",
                    "                                                         │ The home page after login",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ FEEDBACK                               5",
                    "                                                         │ … 3 earlier",
                    &reviewer_row("                 Screenshot unavailable", 100, 20),
                    "                                                         │ The quick brown fox jumps over the lazy",
                    "                                                         │ dog and keeps running through the forest",
                    "                                                         │  until it reaches the river bank where …",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ ╭──────────────────────────────────────╮",
                    "                                                         │ │  Share feedback…                     │",
                    "                                                         │   ⏎ Send Feedback  ·  esc cancel───────╯",
                    "                                                         │",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    composer: true,
                    ..props(&shot)
                },
                100,
                13,
            );
            assert_eq!(
                lines[6..13],
                [
                    "                                                         │ FEEDBACK                               0",
                    "                 Screenshot unavailable                  │ ────────────────────────────────────────",
                    "                                                         │ ╭──────────────────────────────────────╮",
                    "                                                         │ │  Share feedback…                     │",
                    "                                                         │   ⏎ Send Feedback  ·  esc cancel───────╯",
                    "                                                         │",
                    "",
                ]
            );
        }
    }

    #[tokio::test]
    async fn composer_box_shrinks_like_yoga_once_the_comment_list_is_gone() {
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    composer: true,
                    ..props(&shot)
                },
                100,
                12,
            );
            assert_eq!(
                lines[6..12],
                [
                    "                 Screenshot unavailable                  │ FEEDBACK                               0",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ ╭──────────────────────────────────────╮",
                    "                                                         │ ╰─ Share feedback…─────────────────────╯",
                    "                                                         │   ⏎ Send Feedback  ·  esc cancel",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    composer: true,
                    ..props(&shot)
                },
                100,
                11,
            );
            assert_eq!(
                lines[6..11],
                [
                    "                 Screenshot unavailable                  │ FEEDBACK                               0",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ ╭──────────────────────────────────────╮",
                    "                                                         │ │  Share feedback…                     │",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    composer: true,
                    ..props(&shot)
                },
                100,
                10,
            );
            assert_eq!(
                lines[6..10],
                [
                    "                                                         │ FEEDBACK                               0",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ ╭──────────────────────────────────────╮",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    composer: true,
                    ..props(&shot)
                },
                100,
                9,
            );
            assert_eq!(
                lines[6..9],
                [
                    "                                                         │ FEEDBACK                               0",
                    "                                                         │ No comments yet · leave concise, action…",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    composer: true,
                    ..props(&shot)
                },
                100,
                8,
            );
            assert_eq!(
                lines[6..8],
                [
                    "                                                         │ FEEDBACK                               0",
                    "",
                ]
            );
        }
    }

    #[tokio::test]
    async fn busy_and_error_rows_sit_above_the_actions_or_the_composer() {
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    busy: true,
                    ..props(&shot)
                },
                100,
                20,
            );
            assert_eq!(
                lines[14..20],
                [
                    "                                                         │",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ Saving review…",
                    "                                                         │  c Send Feedback   s Seen",
                    "                                                         │ ← → page · +/- zoom · c send · s seen",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    error: Some(LONG_ERROR),
                    ..props(&shot)
                },
                100,
                20,
            );
            assert_eq!(
                lines[14..20],
                [
                    "                                                         │",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ Could not write review.json: EACCES per…",
                    "                                                         │  c Send Feedback   s Seen",
                    "                                                         │ ← → page · +/- zoom · c send · s seen",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    busy: true,
                    error: Some("short error"),
                    composer: true,
                    ..props(&shot)
                },
                100,
                20,
            );
            assert_eq!(
                lines[11..20],
                [
                    "                                                         │",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ Saving review…",
                    "                                                         │ short error",
                    "                                                         │ ╭──────────────────────────────────────╮",
                    "                                                         │ │  Share feedback…                     │",
                    "                                                         │ ╰──────────────────────────────────────╯",
                    "                                                         │   ⏎ Send Feedback  ·  esc cancel",
                    "",
                ]
            );
        }
        {
            let shot = Shot {
                review: review(&["one", FOX], ReviewState::Pending, false),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    busy: true,
                    error: Some("nope"),
                    composer: true,
                    ..props(&shot)
                },
                100,
                14,
            );
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                          ● Unseen",
                    "                                                         │ The home page after login",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ FEEDBACK                               2",
                    &reviewer_row("                 Screenshot unavailable", 100, 14),
                    "                                                         │ one",
                    &reviewer_row("", 100, 14),
                    "                                                         │ The quick brown fox jumps over the lazy",
                    "                                                         │ dog and keeps running through the forest",
                    "                                                         │  until it reaches the river bank where …",
                    "",
                ]
            );
        }
        {
            let shot = Shot {
                review: review(&["one"], ReviewState::Pending, false),
                ..still()
            };
            let mut h = harness();
            let lines = h.rows(
                ReviewTakeoverProps {
                    busy: true,
                    error: Some("short error"),
                    ..props(&shot)
                },
                100,
                10,
            );
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                          ● Unseen",
                    "                                                         │ The home page after login",
                    "                 Screenshot unavailable                  │ ────────────────────────────────────────",
                    "                                                         │ FEEDBACK                               1",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │ Saving review…",
                    "",
                ]
            );
        }
    }

    #[tokio::test]
    async fn short_terminals_clip_the_rail_to_the_stage() {
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(props(&shot), 100, 10);
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                          ● Unseen",
                    "                                                         │ The home page after login",
                    "                 Screenshot unavailable                  │ ────────────────────────────────────────",
                    "                                                         │ FEEDBACK                               0",
                    "                                                         │ ────────────────────────────────────────",
                    "                                                         │  c Send Feedback   s Seen",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(props(&shot), 100, 9);
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                          ● Unseen",
                    "                                                         │ The home page after login",
                    "                 Screenshot unavailable                  │ ────────────────────────────────────────",
                    "                                                         │ FEEDBACK                               0",
                    "                                                         │ ────────────────────────────────────────",
                    "",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(props(&shot), 100, 6);
            assert_eq!(
                lines,
                [
                    " Home   0001                                                                  ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 100),
                    " ──────────────────────────────────────────────────────────────────────────────────────────────────",
                    "                                                         │ Review                          ● Unseen",
                    "                 Screenshot unavailable                  │ The home page after login",
                    "                                                         │ ────────────────────────────────────────",
                ]
            );
        }
        {
            let shot = still();
            let mut h = harness();
            let lines = h.rows(props(&shot), 60, 8);
            assert_eq!(
                lines,
                [
                    " Home   0001                          ‹ 2 / 7 ›   esc close",
                    &meta_row("firstlanding-wt8 · feat", 60),
                    " ──────────────────────────────────────────────────────────",
                    "                               │ Review            ● Unseen",
                    "    Screenshot unavailable     │ The home page after login",
                    "                               │ ──────────────────────────",
                    "                               │ FEEDBACK                 0",
                    "",
                ]
            );
        }
    }

    // ---- styles ------------------------------------------------------------

    #[tokio::test]
    async fn header_stage_and_rail_use_the_ink_colors() {
        let shot = still();
        let buf = harness().draw(props(&shot), 100, 20);
        // Header: bold title, muted gap, the sequence on the selection chip.
        assert_eq!(modifier_at(&buf, 1, 0), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 1, 0), Color::Reset);
        assert_eq!(bg_at(&buf, 6, 0), Color::Reset);
        for x in 7..13 {
            assert_eq!(bg_at(&buf, x, 0), THEME.selection, "column {x}");
            assert_eq!(fg_at(&buf, x, 0), THEME.muted);
        }
        assert_eq!(bg_at(&buf, 13, 0), Color::Reset);
        // Paging: blue chevrons around muted text.
        assert_eq!(fg_at(&buf, 78, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 80, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 86, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 98, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 1, 1), THEME.muted);
        assert_eq!(fg_at(&buf, 1, 2), THEME.faint);
        // Stage caption and divider.
        assert_eq!(fg_at(&buf, 17, 10), THEME.muted);
        for y in 3..19 {
            assert_eq!(fg_at(&buf, 57, y), THEME.faint, "row {y}");
        }
        // Rail.
        assert_eq!(modifier_at(&buf, 59, 3), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 59, 3), Color::Reset);
        assert_eq!(fg_at(&buf, 91, 3), THEME.amber);
        assert_eq!(fg_at(&buf, 59, 4), THEME.muted);
        assert_eq!(fg_at(&buf, 59, 5), THEME.faint);
        assert_eq!(fg_at(&buf, 59, 6), THEME.muted);
        assert_eq!(modifier_at(&buf, 59, 6), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 98, 6), THEME.muted);
        assert_eq!(modifier_at(&buf, 98, 6), Modifier::empty());
        assert_eq!(fg_at(&buf, 59, 7), THEME.muted);
        assert_eq!(fg_at(&buf, 59, 16), THEME.faint);
        // Actions: bold colored keys, plain labels, muted key hints.
        assert_eq!(fg_at(&buf, 60, 17), THEME.blue);
        assert_eq!(modifier_at(&buf, 60, 17), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 62, 17), Color::Reset);
        assert_eq!(modifier_at(&buf, 62, 17), Modifier::empty());
        assert_eq!(fg_at(&buf, 78, 17), THEME.green);
        assert_eq!(modifier_at(&buf, 78, 17), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 80, 17), Color::Reset);
        assert_eq!(fg_at(&buf, 59, 18), THEME.muted);
    }

    #[tokio::test]
    async fn status_rows_stale_note_and_seen_badge_use_their_colors() {
        let shot = Shot {
            review: review(&[], ReviewState::Pending, true),
            ..still()
        };
        let buf = harness().draw(
            ReviewTakeoverProps {
                busy: true,
                error: Some("short error"),
                zoom: 2.0,
                ..props(&shot)
            },
            100,
            20,
        );
        assert_eq!(fg_at(&buf, 59, 4), THEME.amber);
        assert_eq!(fg_at(&buf, 59, 5), THEME.amber);
        assert_eq!(fg_at(&buf, 59, 15), THEME.muted);
        assert_eq!(fg_at(&buf, 59, 16), THEME.red);
        // The cut key hint keeps its color through the ellipsis.
        assert_eq!(fg_at(&buf, 98, 18), THEME.muted);

        let seen = Shot {
            review: review(&[], ReviewState::Seen, false),
            ..still()
        };
        let buf = harness().draw(props(&seen), 100, 20);
        assert_eq!(fg_at(&buf, 93, 3), THEME.blue);
    }

    #[tokio::test]
    async fn progress_bar_follows_the_shared_playback_state() {
        let shot = movie();
        let mut h = harness();
        h.playback(true, 3_000.0, None);
        let playing = h.draw(
            ReviewTakeoverProps {
                playing: true,
                ..props(&shot)
            },
            100,
            20,
        );
        assert_eq!(fg_at(&playing, 1, 18), THEME.green);
        assert_eq!(fg_at(&playing, 3, 18), THEME.purple);
        assert_eq!(fg_at(&playing, 42, 18), THEME.muted);
        // Paused by the parent: the glyph changes on the next frame.
        h.playback(false, 3_000.0, None);
        let paused = h.draw(props(&shot), 100, 20);
        assert_eq!(rows(&paused)[18].chars().nth(1), Some('⏸'));
        assert_eq!(fg_at(&paused, 1, 18), THEME.muted);
        assert!(h.wakes.load(Ordering::SeqCst) >= 2);
    }

    // ---- mounted children --------------------------------------------------

    #[tokio::test]
    async fn picture_and_player_replace_each_other() {
        let shot = movie();
        let mut h = harness();
        h.draw(props(&shot), 100, 20);
        assert!(h.state.picture().is_some() && h.state.player().is_none());
        h.draw(
            ReviewTakeoverProps {
                playing: true,
                ..props(&shot)
            },
            100,
            20,
        );
        assert!(h.state.picture().is_none() && h.state.player().is_some());
        h.draw(props(&shot), 100, 20);
        assert!(h.state.picture().is_some() && h.state.player().is_none());
        // A position past the start keeps the player without the flag.
        h.playback(false, 1.0, None);
        h.draw(props(&shot), 100, 20);
        assert!(h.state.player().is_some());
        // A still never mounts the player.
        let plain = still();
        let lines = h.rows(
            ReviewTakeoverProps {
                playing: true,
                ..props(&plain)
            },
            100,
            20,
        );
        assert!(h.state.player().is_none() && h.state.picture().is_some());
        assert_eq!(
            lines[10],
            "                 Screenshot unavailable                  │"
        );
    }

    // ---- composer input ----------------------------------------------------

    #[tokio::test]
    async fn typed_text_shows_in_the_composer_and_enter_submits_it() {
        let shot = still();
        let mut h = harness();
        let open = || ReviewTakeoverProps {
            composer: true,
            ..props(&shot)
        };
        h.draw(open(), 100, 20);
        type_text(&mut h.state, "looks off");
        assert_eq!(h.state.composer_value(), "looks off");
        let buf = h.draw(open(), 100, 20);
        let lines = rows(&buf);
        assert_eq!(
            lines[15..19],
            [
                "                                                         │ ╭──────────────────────────────────────╮",
                "                                                         │ │ looks off                            │",
                "                                                         │ ╰──────────────────────────────────────╯",
                "                                                         │   ⏎ Send Feedback  ·  esc cancel",
            ]
        );
        // Blue box, the caret after the text, muted hint.
        assert_eq!(fg_at(&buf, 59, 15), THEME.blue);
        assert_eq!(fg_at(&buf, 59, 16), THEME.blue);
        assert_eq!(modifier_at(&buf, 70, 16), Modifier::REVERSED);
        assert_eq!(modifier_at(&buf, 69, 16), Modifier::empty());
        assert_eq!(fg_at(&buf, 61, 18), THEME.muted);
        // Editing keys go to the input.
        assert_eq!(press(&mut h.state, KeyCode::Backspace), Outcome::Pending);
        assert_eq!(h.state.composer_value(), "looks of");
        assert_eq!(
            press(&mut h.state, KeyCode::Enter),
            Outcome::Submit("looks of".into())
        );
    }

    #[tokio::test]
    async fn escape_cancels_and_closing_the_composer_drops_its_text() {
        let shot = still();
        let mut h = harness();
        let open = || ReviewTakeoverProps {
            composer: true,
            ..props(&shot)
        };
        h.draw(open(), 100, 20);
        type_text(&mut h.state, "draft");
        assert_eq!(press(&mut h.state, KeyCode::Esc), Outcome::Cancel);
        // Still there until the parent renders without the composer.
        assert_eq!(h.state.composer_value(), "draft");
        h.draw(props(&shot), 100, 20);
        assert_eq!(h.state.composer_value(), "");
        let lines = h.rows(open(), 100, 20);
        assert_eq!(
            lines[16],
            "                                                         │ │  Share feedback…                     │"
        );
        type_text(&mut h.state, "again");
        h.state.reset_composer();
        assert_eq!(h.state.composer_value(), "");
    }

    #[tokio::test]
    async fn paste_inserts_text_and_a_trailing_newline_submits() {
        let mut h = harness();
        assert_eq!(h.state.handle_paste("pasted text"), Outcome::Pending);
        assert_eq!(h.state.composer_value(), "pasted text");
        assert_eq!(
            h.state.handle_paste(" and more\n"),
            Outcome::Submit("pasted text and more".into())
        );
    }

    #[tokio::test]
    async fn text_painted_over_the_border_covers_exactly_its_own_columns() {
        // 12 rows: the box is down to its two border rows (see the Yoga test).
        let shot = still();
        let mut h = harness();
        let open = || ReviewTakeoverProps {
            composer: true,
            ..props(&shot)
        };
        h.draw(open(), 100, 12);
        type_text(&mut h.state, "ok");
        let buf = h.draw(open(), 100, 12);
        assert_eq!(
            rows(&buf)[8..11],
            [
                "                                                         │ ╭──────────────────────────────────────╮",
                "                                                         │ ╰─ok ──────────────────────────────────╯",
                "                                                         │   ⏎ Send Feedback  ·  esc cancel",
            ]
        );
        assert_eq!(modifier_at(&buf, 63, 9), Modifier::REVERSED);
        assert_eq!(fg_at(&buf, 64, 9), THEME.blue);
    }

    #[test]
    fn composer_text_width_counts_the_caret_and_scrolls_with_the_box() {
        let mut state = TextInputState::new();
        // Empty: the caret and the placeholder, cut to the box.
        assert_eq!(composer_text_width(&state, 40), 16);
        assert_eq!(composer_text_width(&state, 12), 8);
        state.handle_paste("hello");
        // Caret after the text.
        assert_eq!(composer_text_width(&state, 40), 6);
        state.handle(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(composer_text_width(&state, 40), 5);
        // Longer than the box: the visible window is full.
        state.handle_paste(&"x".repeat(60));
        assert_eq!(composer_text_width(&state, 40), 36);
    }

    // ---- divergences and edges ---------------------------------------------

    #[tokio::test]
    async fn header_title_gives_way_to_the_paging_text_when_both_do_not_fit() {
        // Ink shrinks both and wraps `esc close` away; here the paging text
        // stays whole and the title takes what is left.
        let shot = Shot {
            title: "A very long title that goes on".into(),
            sequence: Some("0001-long-sequence".into()),
            ..still()
        };
        let lines = harness().rows(props(&shot), 50, 12);
        assert_eq!(
            lines[0],
            " A very lo…   0001-long-seq…‹ 2 / 7 ›   esc close"
        );
        assert_eq!(
            lines[3..11],
            [
                "                     │ Review            ● Unseen",
                "                     │ The home page after login",
                "                     │ ──────────────────────────",
                "Screenshot unavailab…│ FEEDBACK                 0",
                "                     │ No comments yet · leave c…",
                "                     │ ──────────────────────────",
                "                     │  c Send Feedback   s Seen",
                "                     │ ← → page · +/- zoom · c s…",
            ]
        );
    }

    #[tokio::test]
    async fn screen_narrower_than_the_minimum_columns_is_clipped() {
        // Ink paints the 28-column rail past the right edge of a 30-column screen.
        let shot = still();
        let lines = harness().rows(props(&shot), 30, 12);
        assert_eq!(
            lines[..11],
            [
                " Home  …‹ 2 / 7 ›   esc close",
                meta_row("firstlanding-wt8 · feat", 30).as_str(),
                " ────────────────────────────",
                "          │ Review",
                "          │ The home page afte",
                "          │ ──────────────────",
                "Screensho…│ FEEDBACK",
                "          │ No comments yet · ",
                "          │ ──────────────────",
                "          │  c Send Feedback  ",
                "          │ ← → page · +/- zoo",
            ]
            .map(|line| line.trim_end().to_string())
        );
    }

    #[tokio::test]
    async fn narrow_player_keeps_the_bar_on_the_last_stage_row() {
        let shot = movie();
        let mut h = harness();
        h.playback(true, 3_000.0, None);
        let lines = h.rows(
            ReviewTakeoverProps {
                playing: true,
                ..props(&shot)
            },
            60,
            12,
        );
        // Row 6 is the player's own poster hint, wider than this stage.
        assert_eq!(
            lines[7..11],
            [
                "                               │ No comments yet · leave c…",
                "                               │ ──────────────────────────",
                "                               │  c Send Feedback   s Seen",
                " ▶ ┼━┼●───────┼ 0:03 / 0:12    │ ← → page · space play · +…",
            ]
        );
    }

    #[tokio::test]
    async fn renders_at_an_offset_exactly_as_at_the_origin() {
        let shot = Shot {
            review: review(&["one", FOX], ReviewState::Pending, true),
            ..movie()
        };
        for (composer, playing, height) in [(false, false, 20), (true, true, 20), (true, true, 12)]
        {
            let make = || ReviewTakeoverProps {
                composer,
                playing,
                busy: true,
                ..props(&shot)
            };
            let origin = harness().draw(make(), 90, height);
            let mut h = harness();
            let area = Rect::new(4, 3, 90, height);
            let mut buf = Buffer::empty(Rect::new(0, 0, 100, height + 6));
            ReviewTakeover::new(make()).render(area, &mut buf, &mut h.state);
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    let inside = area.contains((x, y).into());
                    if inside {
                        assert_eq!(buf[(x, y)], origin[(x - 4, y - 3)], "cell {x},{y}");
                    } else {
                        assert_eq!(buf[(x, y)].symbol(), " ", "cell {x},{y} is outside");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn any_small_size_renders_without_leaving_its_area() {
        let shot = Shot {
            review: review(&["one", FOX, "three"], ReviewState::Pending, true),
            description: LONG_DESCRIPTION.into(),
            ..movie()
        };
        for width in 0..=50 {
            for height in 0..=16 {
                for (composer, playing) in [(false, false), (true, true)] {
                    let mut h = harness();
                    let area = Rect::new(2, 1, width, height);
                    let mut buf = Buffer::empty(Rect::new(0, 0, width + 4, height + 2));
                    ReviewTakeover::new(ReviewTakeoverProps {
                        composer,
                        playing,
                        busy: true,
                        error: Some("nope"),
                        ..props(&shot)
                    })
                    .render(area, &mut buf, &mut h.state);
                    for y in 0..buf.area.height {
                        for x in 0..buf.area.width {
                            let inside = area.contains((x, y).into());
                            assert!(
                                inside || buf[(x, y)].symbol() == " ",
                                "{width}x{height}: cell {x},{y} is outside"
                            );
                        }
                    }
                }
            }
        }
    }
}
