//! Port of `packages/astroshot-review/src/ui/detail.tsx`.
//!
//! Shot detail: preview, heading, movie actions, chapters, feedback, Seen,
//! and the metadata card — the tray's Detail pane, in the app's order.
//!
//! # Mapping
//!
//! - `<DetailPane>` is the [`DetailPane`] `StatefulWidget`; its Ink
//!   `width`/`height` props are the `Rect` it renders into. The remaining
//!   props are [`DetailPaneProps`].
//! - [`DetailPaneState`] owns what the Ink children kept in hooks: the
//!   preview's [`PictureState`], the inline [`MoviePlayerState`] and the
//!   composer's [`TextInputState`]. Like React, the picture and the player
//!   replace each other (the one not rendered is dropped, which unregisters
//!   it from the image layer) and the composer text is dropped when the pane
//!   renders with `composer: false`.
//! - `playback` / `onPlayback` are the [`Playback`] handle the state was
//!   built with; the parent keeps a clone and patches it.
//! - The component binds no keys itself. While the composer is open the
//!   parent routes every key and paste to [`DetailPaneState::handle`] /
//!   [`DetailPaneState::handle_paste`]; `onComposerSubmit` / `onComposerCancel`
//!   are the returned [`Outcome`]. The keys named in the action rows
//!   (`c s f +/- p O`) belong to `app.tsx`.
//! - `<CommentList>` is the [`CommentList`] widget (also used by the takeover
//!   screen); [`CommentList::height`] is its intrinsic Ink height.
//!
//! # Layout
//!
//! Checked row for row against frames captured from the Ink component. Every
//! row is fixed except the first rule and the metadata card:
//!
//! - The metadata card takes the rows that are left and clips.
//! - When the fixed rows alone fill the pane, Yoga shrinks the rule above
//!   `FEEDBACK` to zero rows but Ink still paints it, so the `FEEDBACK` row is
//!   drawn over it (`FEEDBACK 0──────`). That is reproduced.
//! - Default-wrap `<Text>` inside a one-row `<Line>` shows the first line of
//!   Ink's word wrap ([`ink_wrap`], a port of `wrap-ansi` with `trim: false,
//!   hard: true`); `wrap="truncate"` cuts with `…`.
//!
//! Divergences, all in layouts Ink itself breaks:
//!
//! - Heading row: when the heading and the badges do not fit, Ink shrinks both
//!   and wraps the badge text onto the description row. Here the badges keep
//!   their width and the heading is cut to the room that is left.
//! - Header row in `page` mode narrower than 34 columns: Ink wraps both texts
//!   into the preview. Here the right text starts after the left and clips.
//! - Width below 22: `inner` stays 20 as in TS, and Ink paints past the pane's
//!   right edge. Here everything is clipped to the pane.

use std::sync::Arc;

use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{StatefulWidget, Widget};

use crate::astroshot_review::data::manifest::{chapter_time_label, duration_label};
use crate::astroshot_review::data::model::{FeatureStatus, ReviewState, Shot};
use crate::astroshot_review::data::paths::dirname;
use crate::astroshot_review::ui::chrome::{
    MetaRow, movie_badge, review_badge, rule, section_label,
};
use crate::astroshot_review::ui::context::AppServices;
use crate::astroshot_review::ui::hooks::Wake;
use crate::astroshot_review::ui::movie_player::{
    MovieBackend, MoviePlayer, MoviePlayerState, Playback, ProgressBar,
};
use crate::astroshot_review::ui::picture::{Picture, PictureProps, PictureState};
use crate::astroshot_review::ui::selectors::review_state_of;
use crate::astroshot_review::ui::text_input::{Outcome, TextInput, TextInputState};
use crate::astroshot_review::ui::theme::{
    DateInput, THEME, abbreviated_date_time, iso_date_time, truncate,
};
use crate::astroshot_review::ui::{put_spans, text_width};

/// `mode: "pane" | "page"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailMode {
    /// The split-view column.
    Pane,
    /// Shows the back/paging header.
    Page,
}

/// `position: { index, count }` (1-based index, 0 when the shot is not listed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailPosition {
    pub index: usize,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct DetailPaneProps<'a> {
    pub shot: &'a Shot,
    pub position: DetailPosition,
    pub mode: DetailMode,
    pub composer: bool,
    pub inline_player: bool,
    pub busy: bool,
    pub error: Option<&'a str>,
    /// 1 shows the whole image; above magnifies.
    pub zoom: f64,
    /// Pan center as a fraction of the image, [0,1].
    pub pan_x: f64,
    pub pan_y: f64,
}

impl<'a> DetailPaneProps<'a> {
    /// Pane mode, no composer, no player, whole image.
    pub fn new(shot: &'a Shot, position: DetailPosition) -> Self {
        Self {
            shot,
            position,
            mode: DetailMode::Pane,
            composer: false,
            inline_player: false,
            busy: false,
            error: None,
            zoom: 1.0,
            pan_x: 0.5,
            pan_y: 0.5,
        }
    }
}

/// `Math.max(6, Math.min(22, Math.round(height * 0.4)))`.
pub fn preview_height(height: u16) -> u16 {
    let rounded = (u32::from(height) * 4 + 5) / 10;
    rounded.clamp(6, 22) as u16
}

// ---- text helpers --------------------------------------------------------

fn char_width(c: char) -> usize {
    text_width(c.encode_utf8(&mut [0; 4]))
}

/// `wrap-ansi`'s `wrapWord`: hard-break `word` across rows of `columns`.
fn wrap_word(rows: &mut Vec<String>, word: &str, columns: usize) {
    let mut visible = rows.last().map_or(0, |row| text_width(row));
    let count = word.chars().count();
    for (index, c) in word.chars().enumerate() {
        let width = char_width(c);
        if visible + width <= columns {
            rows.last_mut().expect("rows is never empty").push(c);
        } else {
            rows.push(c.to_string());
            visible = 0;
        }
        visible += width;
        if visible == columns && index + 1 < count {
            rows.push(String::new());
            visible = 0;
        }
    }
}

/// Ink's default `<Text>` wrap: `wrapAnsi(text, columns, { trim: false, hard: true })`.
///
/// Unlike `chrome::wrap_words` it keeps spaces: a row that is exactly full
/// pushes the separating space onto the next row, and runs of spaces survive.
pub(crate) fn ink_wrap(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    let mut out = Vec::new();
    for line in text.replace("\r\n", "\n").split('\n') {
        let mut rows = vec![String::new()];
        for (index, word) in line.split(' ').enumerate() {
            let mut row_length = text_width(rows.last().expect("rows is never empty"));
            let word_length = text_width(word);
            if index != 0 {
                if row_length >= columns {
                    rows.push(String::new());
                    row_length = 0;
                }
                rows.last_mut().expect("rows is never empty").push(' ');
                row_length += 1;
            }
            if word_length > columns {
                let remaining = columns.saturating_sub(row_length);
                let breaks_this_line = 1 + (word_length - remaining - 1) / columns;
                let breaks_next_line = (word_length - 1) / columns;
                if breaks_next_line < breaks_this_line {
                    rows.push(String::new());
                }
                wrap_word(&mut rows, word, columns);
                continue;
            }
            if row_length + word_length > columns && row_length > 0 && word_length > 0 {
                rows.push(String::new());
            }
            rows.last_mut().expect("rows is never empty").push_str(word);
        }
        out.extend(rows);
    }
    out
}

type Spans = Vec<Span<'static>>;

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// The first `columns` terminal columns of `spans`, styles kept.
fn take_columns(spans: &[Span<'_>], columns: usize) -> Spans {
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let mut text = String::new();
        let mut full = false;
        for c in span.content.chars() {
            let width = char_width(c);
            if used + width > columns {
                full = true;
                break;
            }
            used += width;
            text.push(c);
        }
        if !text.is_empty() {
            out.push(Span::styled(text, span.style));
        }
        if full {
            break;
        }
    }
    out
}

/// Ink `wrap="truncate"`: cut to `width` columns, the last one being `…`.
/// The ellipsis takes the style of the outer `<Text>`.
fn truncate_end(spans: Spans, width: usize, outer: Style) -> Spans {
    if spans_width(&spans) <= width {
        return spans;
    }
    if width == 0 {
        return Vec::new();
    }
    let mut out = take_columns(&spans, width - 1);
    out.push(Span::styled("…", outer));
    out
}

/// A default-wrap `<Text>` in a one-row `<Line>`: only its first wrapped row shows.
fn first_wrapped_row(spans: Spans, width: usize) -> Spans {
    let plain: String = spans.iter().map(|span| span.content.as_ref()).collect();
    let first = ink_wrap(&plain, width).swap_remove(0);
    take_columns(&spans, text_width(&first))
}

fn styled(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

fn fg(color: ratatui::style::Color) -> Style {
    Style::new().fg(color)
}

fn bold(color: ratatui::style::Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

// ---- comment list --------------------------------------------------------

const NO_COMMENTS: &str = "No comments yet · leave concise, actionable feedback for this frame.";

/// The rows `<CommentList>` renders at `width`, top to bottom.
fn comment_rows(shot: &Shot, width: usize, max: usize) -> Vec<Spans> {
    let comments = shot
        .review
        .as_ref()
        .map_or(&[][..], |review| review.comments.as_slice());
    if comments.is_empty() {
        let muted = fg(THEME.muted);
        return vec![truncate_end(vec![styled(NO_COMMENTS, muted)], width, muted)];
    }
    // `comments.slice(-max)`: `-0` is `0`, which keeps everything.
    let hidden = if max == 0 {
        0
    } else {
        comments.len().saturating_sub(max)
    };
    let mut rows = Vec::new();
    if hidden > 0 {
        rows.push(first_wrapped_row(
            vec![styled(format!("… {hidden} earlier"), fg(THEME.faint))],
            width,
        ));
    }
    for comment in &comments[hidden..] {
        let muted = fg(THEME.muted);
        let when = abbreviated_date_time(DateInput::Text(&comment.created_at));
        rows.push(truncate_end(
            vec![
                styled("Reviewer", fg(THEME.purple)),
                styled(format!(" · {when}"), muted),
            ],
            width,
            muted,
        ));
        // Reserved from the raw length; the wrapped text is clipped to it.
        let height = comment
            .body
            .chars()
            .count()
            .div_ceil(width.max(10))
            .clamp(1, 3);
        let mut body = ink_wrap(&truncate(&comment.body, width * 3), width);
        body.resize(height, String::new());
        rows.extend(body.into_iter().map(|line| vec![Span::raw(line)]));
    }
    rows
}

/// `<CommentList shot width max>`: the last `max` comments of `shot`, or the
/// empty-state line. The width is the `Rect` it renders into.
pub struct CommentList<'a> {
    pub shot: &'a Shot,
    pub max: usize,
}

impl CommentList<'_> {
    /// Rows the list occupies at `width` (its intrinsic Ink height).
    pub fn height(&self, width: u16) -> u16 {
        comment_rows(self.shot, usize::from(width), self.max)
            .len()
            .min(usize::from(u16::MAX)) as u16
    }
}

impl Widget for CommentList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rows = comment_rows(self.shot, usize::from(area.width), self.max);
        for (index, spans) in rows.iter().enumerate().take(usize::from(area.height)) {
            put_spans(buf, area, area.x, area.y + index as u16, spans);
        }
    }
}

// ---- detail pane ---------------------------------------------------------

/// Hook state of the pane's children.
pub struct DetailPaneState {
    services: AppServices,
    wake: Wake,
    playback: Playback,
    backend: Option<Arc<dyn MovieBackend>>,
    picture: Option<PictureState>,
    player: Option<MoviePlayerState>,
    composer: TextInputState,
}

impl DetailPaneState {
    /// `playback` is the parent's handle; the inline player patches it.
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

    /// The still preview, while it is mounted.
    pub fn picture(&self) -> Option<&PictureState> {
        self.picture.as_ref()
    }

    /// The inline movie player, while it is mounted.
    pub fn player(&self) -> Option<&MoviePlayerState> {
        self.player.as_ref()
    }
}

/// Top-to-bottom writer over the pane's content column, clipped to the pane
/// (Ink `overflow="hidden"`). `inner` is the TS layout width; `clip` is the
/// part of it that is on screen.
struct Pen<'b> {
    buf: &'b mut Buffer,
    clip: Rect,
    inner: usize,
    y: u16,
}

impl Pen<'_> {
    fn put(&mut self, offset: usize, spans: &[Span<'_>]) -> usize {
        let x = usize::from(self.clip.x) + offset;
        if x >= usize::from(self.clip.right()) {
            return offset;
        }
        let end = put_spans(self.buf, self.clip, x as u16, self.y, spans);
        usize::from(end - self.clip.x)
    }

    fn advance(&mut self, rows: u16) {
        self.y = self.y.saturating_add(rows);
    }

    /// One row, left aligned.
    fn line(&mut self, spans: &[Span<'_>]) {
        self.put(0, spans);
        self.advance(1);
    }

    /// One row with `justifyContent="space-between"`.
    fn between(&mut self, left: &[Span<'_>], right: &[Span<'_>]) {
        let left_end = self.put(0, left);
        let start = self.inner.saturating_sub(spans_width(right)).max(left_end);
        self.put(start, right);
        self.advance(1);
    }

    /// Reserve `rows` rows; returns the on-screen part, if any.
    fn block(&mut self, rows: u16) -> Option<Rect> {
        let rect = Rect::new(self.clip.x, self.y, self.clip.width, rows).intersection(self.clip);
        let visible = self.y < self.clip.bottom() && !rect.is_empty();
        self.advance(rows);
        visible.then_some(rect)
    }

    fn meta(&mut self, label: &str, value: &str) {
        if let Some(rect) = self.block(1) {
            MetaRow { label, value }.render(rect, self.buf);
        }
    }
}

fn execution_label(status: FeatureStatus) -> &'static str {
    match status {
        FeatureStatus::Running => "Running",
        FeatureStatus::Pass => "Pass",
        FeatureStatus::Fail => "Fail",
        FeatureStatus::Idle => "Idle",
    }
}

/// JS truthiness of a `string | null`.
fn truthy(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// The movie block under the description: actions, video notes, chapters.
fn movie_rows(shot: &Shot, inline_player: bool, has_ffmpeg: bool, inner: usize) -> Vec<Spans> {
    let can_play = shot.video_path.is_some();
    let pick = |color| if can_play { color } else { THEME.faint };
    let mut rows = Vec::new();

    let mut actions = vec![
        styled(" p ", bold(pick(THEME.green))),
        styled(
            if inline_player {
                "Hide player"
            } else {
                "Play in tray"
            },
            fg(pick(THEME.text)),
        ),
        styled("   O ", bold(pick(THEME.blue))),
        styled("Open movie", fg(pick(THEME.text))),
    ];
    if !has_ffmpeg && can_play {
        actions.push(styled(
            "   ffmpeg missing · install to play here",
            fg(THEME.amber),
        ));
    }
    rows.push(truncate_end(actions, inner, Style::new()));

    let video_file = truthy(shot.video_file_name.as_deref());
    if video_file.is_some() && truthy(shot.video_path.as_deref()).is_none() {
        rows.push(first_wrapped_row(
            vec![styled("Video missing on disk", fg(THEME.amber))],
            inner,
        ));
    } else if video_file.is_none() {
        rows.push(first_wrapped_row(
            vec![styled(
                "Poster only — no video path in manifest",
                fg(THEME.muted),
            )],
            inner,
        ));
    }

    let chapters = &shot.chapters[..shot.chapters.len().min(3)];
    if !chapters.is_empty() {
        rows.push(first_wrapped_row(vec![section_label("CHAPTERS")], inner));
        for chapter in chapters {
            let title = chapter
                .title
                .as_deref()
                .or(chapter.slug.as_deref())
                .unwrap_or("chapter");
            rows.push(truncate_end(
                vec![
                    styled(
                        format!("{:>6}", chapter_time_label(chapter.t_ms)),
                        fg(THEME.purple),
                    ),
                    Span::raw(format!("  {}", truncate(title, inner.saturating_sub(8)))),
                ],
                inner,
                Style::new(),
            ));
        }
        if shot.chapters.len() > chapters.len() {
            rows.push(first_wrapped_row(
                vec![styled(
                    format!("       … {} more", shot.chapters.len() - chapters.len()),
                    fg(THEME.faint),
                )],
                inner,
            ));
        }
    }
    rows
}

fn meta_rows(shot: &Shot, state: ReviewState) -> Vec<(&'static str, String)> {
    let dash = || "—".to_string();
    let mut rows = vec![
        ("Tree", shot.worktree.clone()),
        ("Feature", shot.feature.clone()),
        ("File", shot.file_name.clone()),
    ];
    if shot.is_movie {
        rows.push(("Kind", "Movie".to_string()));
        rows.push(("Video", shot.video_file_name.clone().unwrap_or_else(dash)));
        rows.push((
            "Duration",
            duration_label(shot.duration_ms).unwrap_or_else(dash),
        ));
        if let Some(source) = truthy(shot.source.as_deref()) {
            rows.push(("Source", source.to_string()));
        }
        if !shot.chapters.is_empty() {
            rows.push(("Chapters", shot.chapters.len().to_string()));
        }
    }
    rows.push(("Time", iso_date_time(shot.captured_at)));
    if let Some(url) = truthy(shot.url.as_deref()) {
        rows.push(("URL", url.to_string()));
    }
    if let Some(run_id) = truthy(shot.run_id.as_deref()) {
        rows.push(("Run", run_id.to_string()));
    }
    if let Some(status) = shot.status {
        rows.push(("Execution", execution_label(status).to_string()));
    }
    rows.push((
        "Review",
        if state == ReviewState::Seen {
            "Seen"
        } else {
            "Unseen"
        }
        .to_string(),
    ));
    rows.push(("Path", dirname(&shot.path)));
    rows
}

/// `<DetailPane>`: renders into the `Rect` it is given (the Ink `width`/`height`).
pub struct DetailPane<'a> {
    pub props: DetailPaneProps<'a>,
}

impl<'a> DetailPane<'a> {
    pub fn new(props: DetailPaneProps<'a>) -> Self {
        Self { props }
    }
}

impl StatefulWidget for DetailPane<'_> {
    type State = DetailPaneState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut DetailPaneState) {
        let props = &self.props;
        let shot = props.shot;
        if !props.composer {
            // The `<TextInput>` is unmounted; its text goes with it.
            state.reset_composer();
        }
        if area.width == 0 || area.height == 0 {
            return;
        }

        let review_state = review_state_of(shot);
        let inner = usize::from(area.width.saturating_sub(2)).max(20);
        let preview = preview_height(area.height);
        let can_play = shot.is_movie && shot.video_path.is_some();
        let player_shown = props.inline_player && can_play;
        let has_ffmpeg = truthy(state.services.ffmpeg.ffmpeg.as_deref()).is_some();
        let playback = state.playback.snapshot();

        let movie = if shot.is_movie {
            movie_rows(shot, props.inline_player, has_ffmpeg, inner)
        } else {
            Vec::new()
        };
        let comments = comment_rows(shot, inner, if props.composer { 2 } else { 3 });
        let composer_rows: u16 = if props.composer { 4 } else { 1 };
        // Every `flexShrink={0}` row; the first rule and the metadata card are the rest.
        let fixed = 1
            + usize::from(preview)
            + usize::from(player_shown)
            + 2
            + movie.len()
            + 1
            + comments.len()
            + usize::from(composer_rows)
            + 1;

        // paddingX={1}; everything is clipped to the pane.
        let clip = Rect::new(
            area.x + 1,
            area.y,
            (inner.min(usize::from(area.width - 1))) as u16,
            area.height,
        );
        let mut pen = Pen {
            buf,
            clip,
            inner,
            y: area.y,
        };

        // Header.
        let muted = fg(THEME.muted);
        let blue = fg(THEME.blue);
        let count = format!("{} / {}", props.position.index, props.position.count);
        match props.mode {
            DetailMode::Page => pen.between(
                &[styled("‹ Stream", blue)],
                &[
                    styled(format!("{count}  "), muted),
                    styled("‹", blue),
                    styled(" older · newer ", muted),
                    styled("›", blue),
                ],
            ),
            DetailMode::Pane => pen.between(&[styled("Detail", muted)], &[styled(count, muted)]),
        }

        // Preview: the inline player or the picture, never both.
        let stage = pen.block(preview);
        if player_shown {
            state.picture = None;
            let (services, wake, backend) = (&state.services, &state.wake, &state.backend);
            let playback = &state.playback;
            let player = state.player.get_or_insert_with(|| match backend {
                Some(backend) => MoviePlayerState::with_backend(
                    services,
                    playback.clone(),
                    wake.clone(),
                    backend.clone(),
                ),
                None => MoviePlayerState::new(services, playback.clone(), wake.clone()),
            });
            if let Some(stage) = stage {
                MoviePlayer {
                    video_path: shot.video_path.as_deref().expect("can_play checked it"),
                    poster_path: &shot.path,
                    poster_version: shot.mtime_ms,
                }
                .render(stage, pen.buf, player);
            }
        } else {
            state.player = None;
            let (services, wake) = (&state.services, &state.wake);
            let picture = state
                .picture
                .get_or_insert_with(|| PictureState::new(services, wake.clone()));
            if let Some(stage) = stage {
                Picture::new(PictureProps {
                    src: Some(&shot.path),
                    version: shot.mtime_ms,
                    label: Some(if shot.is_movie {
                        "▶ movie poster"
                    } else {
                        "still"
                    }),
                    max_upscale: 8.0,
                    zoom: props.zoom,
                    pan_x: props.pan_x,
                    pan_y: props.pan_y,
                    ..PictureProps::default()
                })
                .render(stage, pen.buf, picture);
            }
        }
        if player_shown && let Some(rect) = pen.block(1) {
            ProgressBar {
                position_ms: playback.position_ms,
                duration_ms: playback.duration_ms.or(shot.duration_ms),
                chapters: &shot.chapters,
                playing: playback.playing,
            }
            .render(rect, pen.buf);
        }

        // Heading and badges.
        let heading = match truthy(shot.sequence.as_deref()) {
            Some(sequence) => format!("{sequence} · {}", shot.slug),
            None => shot.slug.clone(),
        };
        let mut badges = Vec::new();
        if shot.is_movie {
            badges.push(movie_badge(duration_label(shot.duration_ms).as_deref()));
            badges.push(Span::raw(" "));
        }
        let stale = shot.review.as_ref().is_some_and(|review| review.is_stale);
        badges.push(review_badge(review_state, stale));
        let heading_style = Style::new().add_modifier(Modifier::BOLD);
        let heading = truncate_end(
            vec![styled(
                truncate(&heading, inner.saturating_sub(24).max(8)),
                heading_style,
            )],
            inner.saturating_sub(spans_width(&badges)),
            heading_style,
        );
        pen.between(&heading, &badges);

        let description = if shot.description.is_empty() {
            &shot.file_name
        } else {
            &shot.description
        };
        pen.line(&truncate_end(
            vec![styled(truncate(description, inner), muted)],
            inner,
            muted,
        ));

        for row in &movie {
            pen.line(row);
        }

        // The shrinkable rule: zero rows once the fixed rows fill the pane,
        // but still painted, so the FEEDBACK row lands on top of it.
        pen.put(0, &[rule(inner, None)]);
        if fixed < usize::from(area.height) {
            pen.advance(1);
        }

        let comment_count = shot
            .review
            .as_ref()
            .map_or(0, |review| review.comments.len());
        let status: Spans = if props.busy {
            vec![styled("Saving review…", muted)]
        } else if let Some(error) = truthy(props.error) {
            vec![styled(
                truncate(error, inner.saturating_sub(12)),
                fg(THEME.red),
            )]
        } else {
            Vec::new()
        };
        pen.between(
            &[section_label(&format!("FEEDBACK {comment_count}"))],
            &status,
        );

        for row in &comments {
            pen.line(row);
        }

        if props.composer {
            if let Some(rect) = pen.block(4) {
                TextInput {
                    placeholder: "Share feedback…",
                    submit_label: Some("Send Feedback"),
                }
                .render(rect, pen.buf, &mut state.composer);
            }
        } else {
            pen.line(&first_wrapped_row(
                vec![
                    styled(" c ", bold(THEME.blue)),
                    styled("Send Feedback", muted),
                    styled("   s ", bold(THEME.green)),
                    styled("Seen", muted),
                    styled("   f ", bold(THEME.purple)),
                    styled("Full screen", muted),
                    styled("   +/- ", bold(THEME.blue)),
                    styled("zoom", muted),
                ],
                inner,
            ));
        }
        pen.line(&[rule(inner, None)]);

        // Metadata card: whatever rows are left.
        for (label, value) in meta_rows(shot, review_state) {
            pen.meta(label, &value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::data::model::{Chapter, ReviewComment, ReviewSnapshot};
    use crate::astroshot_review::terminal::image_layer::CellBox;
    use crate::astroshot_review::terminal::probe::GraphicsProtocol;
    use crate::astroshot_review::ui::context::test_support::{FakeService, services};
    use crate::astroshot_review::ui::movie_player::{
        FrameSource, PlaybackPatch, PlaybackState, ProbeFuture,
    };
    use crate::astroshot_review::ui::testing::{
        bg_at, fg_at, modifier_at, render, render_stateful, row, rows,
    };
    use crate::astroshot_review::video::ffmpeg::FramePlayerOptions;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::style::Color;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The expected rows below are frames captured from the Ink `<DetailPane>`
    // (graphics "none", TZ-independent rows only) at the same size and props.

    // ---- fixtures --------------------------------------------------------

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
            captured_at: 1_786_460_040_123.0,
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

    /// The comment header row in this machine's timezone.
    fn reviewer_row() -> String {
        format!(
            " Reviewer · {}",
            abbreviated_date_time(DateInput::Text(CREATED_AT))
        )
    }

    const FOX: &str = "The quick brown fox jumps over the lazy dog and keeps running through the forest until it reaches the river bank where it stops to drink some water and rest for a while before going on again and again and again";

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
        state: DetailPaneState,
        wakes: Arc<AtomicUsize>,
    }

    fn harness_with(graphics: GraphicsProtocol, ffmpeg: bool) -> Harness {
        let mut services = services(graphics, FakeService::new(false));
        if ffmpeg {
            services.ffmpeg.ffmpeg = Some("/usr/bin/ffmpeg".into());
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let wake: Wake = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let playback = Playback::new(PlaybackState::default(), wake.clone());
        Harness {
            state: DetailPaneState::with_backend(&services, playback, wake, Arc::new(NoMovie)),
            wakes,
        }
    }

    fn harness() -> Harness {
        harness_with(GraphicsProtocol::None, false)
    }

    fn props(shot: &Shot) -> DetailPaneProps<'_> {
        DetailPaneProps::new(shot, DetailPosition { index: 2, count: 7 })
    }

    impl Harness {
        fn draw(&mut self, props: DetailPaneProps<'_>, width: u16, height: u16) -> Buffer {
            render_stateful(DetailPane::new(props), &mut self.state, width, height)
        }

        fn rows(&mut self, props: DetailPaneProps<'_>, width: u16, height: u16) -> Vec<String> {
            rows(&self.draw(props, width, height))
        }
    }

    fn rule_row(width: usize) -> String {
        format!(" {}", "─".repeat(width))
    }

    fn press(state: &mut DetailPaneState, code: KeyCode) -> Outcome {
        state.handle(KeyEvent::new(code, KeyModifiers::NONE))
    }

    // ---- preview_height / ink_wrap ----------------------------------------

    #[test]
    fn preview_height_is_forty_percent_clamped_to_6_and_22() {
        assert_eq!(preview_height(0), 6);
        assert_eq!(preview_height(12), 6);
        assert_eq!(preview_height(16), 6);
        assert_eq!(preview_height(17), 7);
        assert_eq!(preview_height(20), 8);
        assert_eq!(preview_height(30), 12);
        assert_eq!(preview_height(31), 12);
        assert_eq!(preview_height(40), 16);
        assert_eq!(preview_height(54), 22);
        assert_eq!(preview_height(200), 22);
    }

    #[test]
    fn ink_wrap_keeps_the_space_after_an_exactly_full_row() {
        assert_eq!(
            ink_wrap(&truncate(FOX, 174), 58),
            [
                "The quick brown fox jumps over the lazy dog and keeps ",
                "running through the forest until it reaches the river bank",
                " where it stops to drink some water and rest for a while ",
                "befo…",
            ]
        );
    }

    #[test]
    fn ink_wrap_hard_breaks_long_words_and_keeps_runs_of_spaces() {
        assert_eq!(
            ink_wrap(
                "short words then averyveryverylongwordthatdoesnotfitinonelineatall end",
                28
            ),
            [
                "short words then ",
                "averyveryverylongwordthatdoe",
                "snotfitinonelineatall end",
            ]
        );
        assert_eq!(
            ink_wrap(" c Send Feedback   s Seen   f Full", 22),
            [" c Send Feedback   s ", "Seen   f Full"]
        );
        assert_eq!(ink_wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(ink_wrap("a\r\nb\nc", 5), ["a", "b", "c"]);
        assert_eq!(ink_wrap("", 5), [""]);
        // A zero width is treated as one column instead of looping.
        assert_eq!(ink_wrap("ab", 0), ["a", "b"]);
    }

    // ---- still shot --------------------------------------------------------

    #[tokio::test]
    async fn still_shot_in_pane_mode_renders_the_ink_frame() {
        let shot = still();
        let mut h = harness();
        let buf = h.draw(props(&shot), 60, 30);
        assert_eq!(
            rows(&buf),
            [
                " Detail                                               2 / 7",
                "",
                "",
                "",
                "",
                "",
                "                           still",
                "",
                "",
                "",
                "",
                "",
                "",
                " 0001 · home                                       ● Unseen",
                " The home page after login",
                &rule_row(58),
                " FEEDBACK 0",
                " No comments yet · leave concise, actionable feedback for …",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
                &rule_row(58),
                " Tree      firstlanding-wt8",
                " Feature   feat",
                " File      0001-home.png",
                " Time      2026-08-11T14:54:00Z",
                " Review    Unseen",
                " Path      /w/.astroshot/feat",
                "",
                "",
                "",
                "",
            ]
        );
        // Header and picture caption are muted; the heading is bold.
        assert_eq!(fg_at(&buf, 1, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 54, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 27, 6), THEME.muted);
        assert_eq!(modifier_at(&buf, 1, 13), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 51, 13), THEME.amber);
        assert_eq!(fg_at(&buf, 1, 14), THEME.muted);
        assert_eq!(fg_at(&buf, 1, 15), THEME.faint);
        assert_eq!(fg_at(&buf, 1, 16), THEME.muted);
        assert_eq!(modifier_at(&buf, 1, 16), Modifier::BOLD);
        // The truncated empty-state line keeps its color through the ellipsis.
        assert_eq!(fg_at(&buf, 58, 17), THEME.muted);
        // Action row: bold colored keys, muted labels.
        let keys = [
            (2, THEME.blue),
            (20, THEME.green),
            (29, THEME.purple),
            (45, THEME.blue),
        ];
        for (x, color) in keys {
            assert_eq!(fg_at(&buf, x, 18), color, "key at {x}");
            assert_eq!(modifier_at(&buf, x, 18), Modifier::BOLD);
        }
        assert_eq!(fg_at(&buf, 4, 18), THEME.muted);
        assert_eq!(modifier_at(&buf, 4, 18), Modifier::empty());
        assert_eq!(fg_at(&buf, 1, 20), THEME.muted);
        assert_eq!(fg_at(&buf, 11, 20), Color::Reset);
    }

    #[tokio::test]
    async fn page_mode_shows_the_paging_header_and_every_optional_meta_row() {
        let shot = Shot {
            sequence: None,
            description: String::new(),
            url: Some("https://x.test/a".into()),
            run_id: Some("run-1".into()),
            status: Some(FeatureStatus::Pass),
            review: review(&[], ReviewState::Seen, false),
            ..still()
        };
        let mut h = harness();
        let buf = h.draw(
            DetailPaneProps {
                mode: DetailMode::Page,
                ..props(&shot)
            },
            60,
            30,
        );
        let lines = rows(&buf);
        assert_eq!(
            lines[0],
            " ‹ Stream                          2 / 7  ‹ older · newer ›"
        );
        // No sequence: the slug alone. No description: the file name.
        assert_eq!(
            lines[13],
            " home                                                ● Seen"
        );
        assert_eq!(lines[14], " 0001-home.png");
        assert_eq!(
            lines[20..],
            [
                " Tree      firstlanding-wt8",
                " Feature   feat",
                " File      0001-home.png",
                " Time      2026-08-11T14:54:00Z",
                " URL       https://x.test/a",
                " Run       run-1",
                " Execution Pass",
                " Review    Seen",
                " Path      /w/.astroshot/feat",
                "",
            ]
        );
        assert_eq!(fg_at(&buf, 1, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 35, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 42, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 44, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 58, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 53, 13), THEME.blue);
    }

    #[tokio::test]
    async fn empty_strings_count_as_missing_like_js_truthiness() {
        let shot = Shot {
            sequence: Some(String::new()),
            url: Some(String::new()),
            run_id: Some(String::new()),
            ..still()
        };
        let lines = harness().rows(props(&shot), 60, 30);
        assert_eq!(
            lines[13],
            " home                                              ● Unseen"
        );
        assert_eq!(lines[23], " Time      2026-08-11T14:54:00Z");
        assert_eq!(lines[24], " Review    Unseen");
    }

    #[tokio::test]
    async fn execution_row_capitalizes_each_status() {
        for (status, label) in [
            (FeatureStatus::Running, "Running"),
            (FeatureStatus::Pass, "Pass"),
            (FeatureStatus::Fail, "Fail"),
            (FeatureStatus::Idle, "Idle"),
        ] {
            let shot = Shot {
                status: Some(status),
                ..still()
            };
            let lines = harness().rows(props(&shot), 60, 30);
            assert_eq!(lines[24], format!(" Execution {label}"));
        }
    }

    #[tokio::test]
    async fn long_headings_are_cut_and_the_stale_badge_is_right_aligned() {
        let shot = Shot {
            slug: "a-very-long-slug-name-that-overflows-the-heading-row-for-sure".into(),
            review: review(&[], ReviewState::Pending, true),
            ..still()
        };
        let lines = harness().rows(props(&shot), 60, 30);
        assert_eq!(
            lines[13],
            " 0001 · a-very-long-slug-name-that…      ● Unseen · changed"
        );
    }

    #[tokio::test]
    async fn heading_gives_way_to_the_badges_when_both_do_not_fit() {
        // Ink shrinks both and wraps the badge onto the next row; here the
        // badges stay whole and the heading takes what is left.
        let shot = Shot {
            slug: "a-very-long-slug-name-that-overflows-the-heading-row".into(),
            chapters: Vec::new(),
            review: review(&[], ReviewState::Pending, true),
            ..movie()
        };
        let lines = harness().rows(props(&shot), 50, 30);
        assert_eq!(
            lines[13],
            " 0001 · a-very-l… Movie · 12s  ● Unseen · changed"
        );
        assert_eq!(lines[14], " The home page after login");
    }

    // ---- movie shot --------------------------------------------------------

    #[tokio::test]
    async fn movie_shot_in_page_mode_renders_the_ink_frame() {
        let shot = movie();
        let mut h = harness();
        let buf = h.draw(
            DetailPaneProps {
                mode: DetailMode::Page,
                ..props(&shot)
            },
            70,
            40,
        );
        assert_eq!(
            rows(&buf),
            [
                " ‹ Stream                                    2 / 7  ‹ older · newer ›",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "                            ▶ movie poster",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                " 0001 · home                                    Movie · 12s  ● Unseen",
                " The home page after login",
                "  p Play in tray   O Open movie   ffmpeg missing · install to play h…",
                " CHAPTERS",
                "   0.0s  Open",
                "   1.5s  b",
                "   1:05  chapter",
                "        … 2 more",
                &rule_row(68),
                " FEEDBACK 0",
                " No comments yet · leave concise, actionable feedback for this frame.",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
                &rule_row(68),
                " Tree      firstlanding-wt8",
                " Feature   feat",
                " File      0001-home.png",
                " Kind      Movie",
                " Video     0001-home.webm",
                " Duration  12s",
                " Source    browser",
                " Chapters  5",
                " Time      2026-08-11T14:54:00Z",
                " Review    Unseen",
            ]
        );
        // Movie badge on its own background, then the review badge.
        assert_eq!(bg_at(&buf, 47, 17), Color::Rgb(0x3b, 0x2f, 0x6e));
        assert_eq!(fg_at(&buf, 48, 17), THEME.purple);
        assert_eq!(bg_at(&buf, 60, 17), Color::Reset);
        assert_eq!(fg_at(&buf, 61, 17), THEME.amber);
        // Playable: green p, text label, blue O, amber ffmpeg note, plain ellipsis.
        assert_eq!(fg_at(&buf, 2, 19), THEME.green);
        assert_eq!(modifier_at(&buf, 2, 19), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 4, 19), THEME.text);
        assert_eq!(fg_at(&buf, 19, 19), THEME.blue);
        assert_eq!(fg_at(&buf, 21, 19), THEME.text);
        assert_eq!(fg_at(&buf, 34, 19), THEME.amber);
        assert_eq!(fg_at(&buf, 68, 19), Color::Reset);
        // Chapters: bold muted label, purple right-aligned time, faint overflow.
        assert_eq!(modifier_at(&buf, 1, 20), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 3, 21), THEME.purple);
        assert_eq!(fg_at(&buf, 9, 21), Color::Reset);
        assert_eq!(fg_at(&buf, 8, 24), THEME.faint);
    }

    #[tokio::test]
    async fn inline_player_replaces_the_picture_and_adds_the_progress_bar() {
        let shot = movie();
        let mut h = harness_with(GraphicsProtocol::None, true);
        h.state.playback().patch(&PlaybackPatch {
            playing: Some(true),
            position_ms: Some(3_000.0),
            ..PlaybackPatch::default()
        });
        let lines = h.rows(
            DetailPaneProps {
                inline_player: true,
                ..props(&shot)
            },
            70,
            40,
        );
        assert_eq!(
            lines,
            [
                " Detail                                                         2 / 7",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "                               ▶ movie",
                "               Poster shown · press O to open the movie",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                " ▶ ┼━━━━━┼━━━━━━●────────────────────────────────────┼ 0:03 / 0:12",
                " 0001 · home                                    Movie · 12s  ● Unseen",
                " The home page after login",
                "  p Hide player   O Open movie",
                " CHAPTERS",
                "   0.0s  Open",
                "   1.5s  b",
                "   1:05  chapter",
                "        … 2 more",
                &rule_row(68),
                " FEEDBACK 0",
                " No comments yet · leave concise, actionable feedback for this frame.",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
                &rule_row(68),
                " Tree      firstlanding-wt8",
                " Feature   feat",
                " File      0001-home.png",
                " Kind      Movie",
                " Video     0001-home.webm",
                " Duration  12s",
                " Source    browser",
                " Chapters  5",
                " Time      2026-08-11T14:54:00Z",
            ]
        );
        assert!(h.state.player().is_some());
        assert!(h.state.picture().is_none());
    }

    #[tokio::test]
    async fn paused_player_without_ffmpeg_on_an_odd_width_matches_ink() {
        let shot = movie();
        let mut h = harness();
        let lines = h.rows(
            DetailPaneProps {
                inline_player: true,
                ..props(&shot)
            },
            71,
            31,
        );
        assert_eq!(
            lines[..17],
            [
                " Detail                                                          2 / 7",
                "",
                "",
                "",
                "",
                "",
                "                                ▶ movie",
                "                Poster shown · press O to open the movie",
                "",
                "",
                "",
                "",
                "",
                " ⏸ ●──────┼───────────────────────────────────────────┼ 0:00 / 0:12",
                " 0001 · home                                     Movie · 12s  ● Unseen",
                " The home page after login",
                "  p Hide player   O Open movie   ffmpeg missing · install to play here",
            ]
        );
        assert_eq!(lines[30], " Kind      Movie");
    }

    #[tokio::test]
    async fn progress_bar_prefers_the_probed_duration_over_the_manifest() {
        let shot = movie();
        let mut h = harness_with(GraphicsProtocol::None, true);
        h.state.playback().patch(&PlaybackPatch {
            duration_ms: Some(Some(60_000.0)),
            ..PlaybackPatch::default()
        });
        let lines = h.rows(
            DetailPaneProps {
                inline_player: true,
                ..props(&shot)
            },
            70,
            40,
        );
        assert!(lines[17].ends_with(" 0:00 / 1:00"), "{}", lines[17]);
    }

    #[tokio::test]
    async fn switching_the_player_on_and_off_swaps_the_mounted_child() {
        let shot = movie();
        let mut h = harness_with(GraphicsProtocol::None, true);
        h.draw(props(&shot), 70, 40);
        assert!(h.state.picture().is_some() && h.state.player().is_none());
        h.draw(
            DetailPaneProps {
                inline_player: true,
                ..props(&shot)
            },
            70,
            40,
        );
        assert!(h.state.picture().is_none() && h.state.player().is_some());
        h.draw(props(&shot), 70, 40);
        assert!(h.state.picture().is_some() && h.state.player().is_none());
        // A still never mounts the player, even with the flag on.
        let plain = still();
        let lines = h.rows(
            DetailPaneProps {
                inline_player: true,
                ..props(&plain)
            },
            60,
            30,
        );
        assert!(h.state.player().is_none());
        assert_eq!(lines[6], "                           still");
        assert_eq!(
            lines[13],
            " 0001 · home                                       ● Unseen"
        );
    }

    #[tokio::test]
    async fn movie_whose_video_is_missing_on_disk_greys_the_actions() {
        let shot = Shot {
            video_path: None,
            chapters: Vec::new(),
            duration_ms: None,
            source: None,
            ..movie()
        };
        let mut h = harness();
        // The player flag is ignored when there is nothing to play.
        let buf = h.draw(
            DetailPaneProps {
                inline_player: true,
                ..props(&shot)
            },
            60,
            30,
        );
        assert!(h.state.player().is_none());
        assert_eq!(
            rows(&buf)[13..],
            [
                " 0001 · home                                Movie  ● Unseen",
                " The home page after login",
                "  p Hide player   O Open movie",
                " Video missing on disk",
                &rule_row(58),
                " FEEDBACK 0",
                " No comments yet · leave concise, actionable feedback for …",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
                &rule_row(58),
                " Tree      firstlanding-wt8",
                " Feature   feat",
                " File      0001-home.png",
                " Kind      Movie",
                " Video     0001-home.webm",
                " Duration  —",
                " Time      2026-08-11T14:54:00Z",
                " Review    Unseen",
            ]
        );
        assert_eq!(row(&buf, 6), "                       ▶ movie poster");
        for x in [2, 4, 18, 20] {
            assert_eq!(fg_at(&buf, x, 15), THEME.faint, "column {x}");
        }
        assert_eq!(fg_at(&buf, 1, 16), THEME.amber);
    }

    #[tokio::test]
    async fn poster_only_movie_says_so_and_lists_its_single_chapter() {
        let shot = Shot {
            video_path: None,
            video_file_name: None,
            chapters: vec![chapter(Some("a"), Some("Open"), Some(0.0))],
            ..movie()
        };
        let buf = harness().draw(props(&shot), 60, 30);
        assert_eq!(
            rows(&buf)[13..],
            [
                " 0001 · home                          Movie · 12s  ● Unseen",
                " The home page after login",
                "  p Play in tray   O Open movie",
                " Poster only — no video path in manifest",
                " CHAPTERS",
                "   0.0s  Open",
                &rule_row(58),
                " FEEDBACK 0",
                " No comments yet · leave concise, actionable feedback for …",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
                &rule_row(58),
                " Tree      firstlanding-wt8",
                " Feature   feat",
                " File      0001-home.png",
                " Kind      Movie",
                " Video     —",
                " Duration  12s",
            ]
        );
        assert_eq!(fg_at(&buf, 1, 16), THEME.muted);
    }

    #[tokio::test]
    async fn chapter_titles_fall_back_to_slug_then_a_placeholder_and_are_cut() {
        let shot = Shot {
            chapters: vec![
                chapter(Some("slug"), Some(""), None),
                chapter(
                    None,
                    Some("A chapter title that is much too long for the row"),
                    Some(125_000.0),
                ),
            ],
            ..movie()
        };
        let lines = harness().rows(props(&shot), 30, 40);
        // `??` keeps an empty title; an unknown time is a right-aligned dash.
        assert_eq!(lines[21], "      —");
        // inner 28: the title is cut to 20 characters.
        assert_eq!(lines[22], "   2:05  A chapter title tha…");
        assert_eq!(lines[23], rule_row(28));
    }

    // ---- comments ----------------------------------------------------------

    #[tokio::test]
    async fn comments_show_the_last_three_with_reserved_body_rows() {
        let long = "x".repeat(70);
        let shot = Shot {
            review: review(
                &["one", "two words here", &long, FOX, "last"],
                ReviewState::Pending,
                true,
            ),
            ..still()
        };
        let buf = harness().draw(props(&shot), 60, 30);
        let lines = rows(&buf);
        assert_eq!(
            lines[13],
            " 0001 · home                             ● Unseen · changed"
        );
        assert_eq!(
            lines[15..],
            [
                rule_row(58).as_str(),
                " FEEDBACK 5",
                " … 2 earlier",
                &reviewer_row(),
                &format!(" {}", "x".repeat(58)),
                " xxxxxxxxxxxx",
                &reviewer_row(),
                " The quick brown fox jumps over the lazy dog and keeps",
                " running through the forest until it reaches the river bank",
                "  where it stops to drink some water and rest for a while",
                &reviewer_row(),
                " last",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
                &rule_row(58),
                // The metadata card is clipped to the one row that is left.
                " Tree      firstlanding-wt8",
            ]
        );
        assert_eq!(fg_at(&buf, 1, 17), THEME.faint);
        assert_eq!(fg_at(&buf, 1, 18), THEME.purple);
        assert_eq!(fg_at(&buf, 10, 18), THEME.muted);
        assert_eq!(fg_at(&buf, 1, 19), Color::Reset);
    }

    #[tokio::test]
    async fn comment_bodies_wrap_like_ink_and_collapse_whitespace() {
        let shot = Shot {
            review: review(
                &[
                    "short words then averyveryverylongwordthatdoesnotfitinonelineatall end",
                    "a  b\n\nc",
                ],
                ReviewState::Pending,
                false,
            ),
            ..still()
        };
        let lines = harness().rows(props(&shot), 30, 40);
        let header: String = reviewer_row().chars().take(28).collect();
        assert_eq!(
            lines[19..29],
            [
                rule_row(28).as_str(),
                " FEEDBACK 2",
                &format!("{header}…"),
                " short words then",
                " averyveryverylongwordthatdoe",
                " snotfitinonelineatall end",
                &format!("{header}…"),
                " a b c",
                "  c Send Feedback   s Seen",
                &rule_row(28),
            ]
        );
    }

    #[tokio::test]
    async fn comment_list_widget_renders_empty_state_and_counts_its_rows() {
        let empty = still();
        let list = CommentList {
            shot: &empty,
            max: 3,
        };
        assert_eq!(list.height(40), 1);
        let buf = render(list, 40, 2);
        assert_eq!(rows(&buf), ["No comments yet · leave concise, action…", ""]);
        assert_eq!(fg_at(&buf, 0, 0), THEME.muted);
        // Wide enough: the whole sentence, no ellipsis.
        let wide = render(
            CommentList {
                shot: &empty,
                max: 3,
            },
            80,
            1,
        );
        assert_eq!(row(&wide, 0), NO_COMMENTS);
        // A review with no comments is the same empty state.
        let reviewed = Shot {
            review: review(&[], ReviewState::Seen, false),
            ..still()
        };
        assert_eq!(
            CommentList {
                shot: &reviewed,
                max: 3
            }
            .height(80),
            1
        );
    }

    #[tokio::test]
    async fn comment_list_widget_limits_to_max_and_reserves_one_to_three_rows() {
        let long = "y".repeat(200);
        let shot = Shot {
            review: review(&["a", "b", &long], ReviewState::Pending, false),
            ..still()
        };
        let header: String = reviewer_row().chars().skip(1).collect();
        // max 2: one hidden, then 1 + 1 and 1 + 3 rows.
        let list = CommentList {
            shot: &shot,
            max: 2,
        };
        assert_eq!(list.height(40), 7);
        assert_eq!(
            rows(&render(list, 40, 8)),
            [
                "… 1 earlier",
                &header,
                "b",
                &header,
                &"y".repeat(40),
                &"y".repeat(40),
                // 3 rows of 40 hold the 120-character cut: 119 + ellipsis.
                &format!("{}…", "y".repeat(39)),
                "",
            ]
        );
        // max covers everything: no "earlier" row.
        assert_eq!(
            CommentList {
                shot: &shot,
                max: 3
            }
            .height(40),
            8
        );
        // `slice(-0)` keeps every comment.
        assert_eq!(
            CommentList {
                shot: &shot,
                max: 0
            }
            .height(40),
            8
        );
        // Narrow boxes divide by at least 10 columns: 15 chars reserve 2 rows.
        let narrow = Shot {
            review: review(&["123456789012345"], ReviewState::Pending, false),
            ..still()
        };
        let list = CommentList {
            shot: &narrow,
            max: 3,
        };
        assert_eq!(list.height(5), 3);
        // Rows past the area are clipped.
        let clipped = render(
            CommentList {
                shot: &shot,
                max: 3,
            },
            40,
            2,
        );
        assert_eq!(rows(&clipped), [header.as_str(), "a"]);
    }

    // ---- feedback status ---------------------------------------------------

    #[tokio::test]
    async fn busy_wins_over_the_error_on_the_feedback_row() {
        let shot = still();
        let buf = harness().draw(
            DetailPaneProps {
                busy: true,
                error: Some("boom"),
                ..props(&shot)
            },
            60,
            30,
        );
        assert_eq!(
            row(&buf, 16),
            " FEEDBACK 0                                  Saving review…"
        );
        assert_eq!(fg_at(&buf, 45, 16), THEME.muted);
    }

    #[tokio::test]
    async fn error_is_red_cut_to_the_row_and_narrow_actions_wrap_at_a_word() {
        let shot = still();
        let buf = harness().draw(
            DetailPaneProps {
                error: Some("Could not write the review file because the disk is full"),
                ..props(&shot)
            },
            40,
            30,
        );
        let lines = rows(&buf);
        assert_eq!(lines[16], " FEEDBACK 0  Could not write the revie…");
        assert_eq!(fg_at(&buf, 13, 16), THEME.red);
        assert_eq!(lines[17], " No comments yet · leave concise, acti…");
        // Default wrap: the row ends at the last word that fits.
        assert_eq!(lines[18], "  c Send Feedback   s Seen   f Full");
        // An empty error is no error.
        let none = harness().rows(
            DetailPaneProps {
                error: Some(""),
                ..props(&shot)
            },
            40,
            30,
        );
        assert_eq!(none[16], " FEEDBACK 0");
    }

    // ---- composer ----------------------------------------------------------

    #[tokio::test]
    async fn composer_replaces_the_action_row_and_shows_two_comments() {
        let shot = Shot {
            review: review(&["one", "two", "three"], ReviewState::Pending, false),
            ..still()
        };
        let mut h = harness();
        let lines = h.rows(
            DetailPaneProps {
                composer: true,
                ..props(&shot)
            },
            60,
            30,
        );
        assert_eq!(
            lines[15..],
            [
                rule_row(58).as_str(),
                " FEEDBACK 3",
                " … 1 earlier",
                &reviewer_row(),
                " two",
                &reviewer_row(),
                " three",
                " ╭────────────────────────────────────────────────────────╮",
                " │  Share feedback…                                       │",
                " ╰────────────────────────────────────────────────────────╯",
                "   ⏎ Send Feedback  ·  esc cancel",
                &rule_row(58),
                " Tree      firstlanding-wt8",
                " Feature   feat",
                " File      0001-home.png",
            ]
        );
    }

    #[tokio::test]
    async fn composer_keys_edit_submit_and_cancel() {
        let shot = still();
        let mut h = harness();
        let open = |shot| DetailPaneProps {
            composer: true,
            mode: DetailMode::Page,
            ..props(shot)
        };
        let lines = h.rows(open(&shot), 60, 30);
        assert_eq!(
            lines[16..23],
            [
                " FEEDBACK 0",
                " No comments yet · leave concise, actionable feedback for …",
                " ╭────────────────────────────────────────────────────────╮",
                " │  Share feedback…                                       │",
                " ╰────────────────────────────────────────────────────────╯",
                "   ⏎ Send Feedback  ·  esc cancel",
                rule_row(58).as_str(),
            ]
        );

        // Typing, cursor movement and deletion go to the text input.
        for c in "Tighten".chars() {
            assert_eq!(press(&mut h.state, KeyCode::Char(c)), Outcome::Pending);
        }
        assert_eq!(press(&mut h.state, KeyCode::Char('!')), Outcome::Pending);
        assert_eq!(press(&mut h.state, KeyCode::Backspace), Outcome::Pending);
        assert_eq!(press(&mut h.state, KeyCode::Left), Outcome::Pending);
        assert_eq!(press(&mut h.state, KeyCode::Right), Outcome::Pending);
        assert_eq!(h.state.handle_paste(" the spacing"), Outcome::Pending);
        assert_eq!(h.state.composer_value(), "Tighten the spacing");
        let buf = h.draw(open(&shot), 60, 30);
        assert_eq!(
            row(&buf, 19),
            " │ Tighten the spacing                                    │"
        );
        // The caret is the inverse cell after the text.
        assert_eq!(modifier_at(&buf, 22, 19), Modifier::REVERSED);

        // The app's own keys are plain text while the composer is open.
        for c in ['c', 's', 'f', 'p', 'O', '+', '-'] {
            assert_eq!(press(&mut h.state, KeyCode::Char(c)), Outcome::Pending);
        }
        assert_eq!(h.state.composer_value(), "Tighten the spacingcsfpO+-");
        let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(h.state.handle(ctrl_u), Outcome::Pending);
        assert_eq!(h.state.composer_value(), "");

        // Enter is onComposerSubmit(text); Esc is onComposerCancel().
        h.state.handle_paste("Ship it");
        assert_eq!(
            press(&mut h.state, KeyCode::Enter),
            Outcome::Submit("Ship it".into())
        );
        assert_eq!(press(&mut h.state, KeyCode::Esc), Outcome::Cancel);
        // A paste ending in a newline submits.
        assert_eq!(
            h.state.handle_paste(" now\n"),
            Outcome::Submit("Ship it now".into())
        );
        // Neither outcome clears the text; closing the composer does.
        assert_eq!(h.state.composer_value(), "Ship it now");
    }

    #[tokio::test]
    async fn closing_the_composer_drops_its_text() {
        let shot = still();
        let mut h = harness();
        h.state.handle_paste("draft");
        let open = h.rows(
            DetailPaneProps {
                composer: true,
                ..props(&shot)
            },
            60,
            30,
        );
        assert_eq!(
            open[19],
            " │ draft                                                  │"
        );
        // Rendering closed unmounts the input.
        let closed = h.rows(props(&shot), 60, 30);
        assert_eq!(
            closed[18],
            "  c Send Feedback   s Seen   f Full screen   +/- zoom"
        );
        assert_eq!(h.state.composer_value(), "");
        // The parent can also drop it without waiting for a frame.
        h.state.handle_paste("again");
        h.state.reset_composer();
        assert_eq!(h.state.composer_value(), "");
    }

    // ---- short and narrow panes --------------------------------------------

    #[tokio::test]
    async fn metadata_card_takes_only_the_rows_that_are_left() {
        let shot = still();
        // 14 rows: preview 6, fixed rows 13, the rule, no metadata.
        assert_eq!(
            harness().rows(props(&shot), 60, 14),
            [
                " Detail                                               2 / 7",
                "",
                "",
                "                           still",
                "",
                "",
                "",
                " 0001 · home                                       ● Unseen",
                " The home page after login",
                &rule_row(58),
                " FEEDBACK 0",
                " No comments yet · leave concise, actionable feedback for …",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
                &rule_row(58),
            ]
        );
        // 16 rows: two metadata rows fit.
        let lines = harness().rows(props(&shot), 60, 16);
        assert_eq!(lines[14], " Tree      firstlanding-wt8");
        assert_eq!(lines[15], " Feature   feat");
    }

    #[tokio::test]
    async fn feedback_row_is_drawn_over_the_rule_once_the_fixed_rows_fill_the_pane() {
        let shot = still();
        let buf = harness().draw(props(&shot), 60, 12);
        assert_eq!(
            rows(&buf),
            [
                " Detail                                               2 / 7",
                "",
                "",
                "                           still",
                "",
                "",
                "",
                " 0001 · home                                       ● Unseen",
                " The home page after login",
                &format!(" FEEDBACK 0{}", "─".repeat(48)),
                " No comments yet · leave concise, actionable feedback for …",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
            ]
        );
        assert_eq!(fg_at(&buf, 1, 9), THEME.muted);
        assert_eq!(fg_at(&buf, 11, 9), THEME.faint);

        // Same with a movie at 20 rows and with the composer at 22.
        let film = movie();
        let lines = harness().rows(props(&film), 60, 20);
        assert_eq!(
            lines[11..],
            [
                "  p Play in tray   O Open movie   ffmpeg missing · install…",
                " CHAPTERS",
                "   0.0s  Open",
                "   1.5s  b",
                "   1:05  chapter",
                "        … 2 more",
                &format!(" FEEDBACK 0{}", "─".repeat(48)),
                " No comments yet · leave concise, actionable feedback for …",
                "  c Send Feedback   s Seen   f Full screen   +/- zoom",
            ]
        );
        let commented = Shot {
            review: review(&["one", "two", "three"], ReviewState::Pending, false),
            ..still()
        };
        let lines = harness().rows(
            DetailPaneProps {
                composer: true,
                ..props(&commented)
            },
            60,
            22,
        );
        assert_eq!(
            lines[12..],
            [
                format!(" FEEDBACK 3{}", "─".repeat(48)).as_str(),
                " … 1 earlier",
                &reviewer_row(),
                " two",
                &reviewer_row(),
                " three",
                " ╭────────────────────────────────────────────────────────╮",
                " │  Share feedback…                                       │",
                " ╰────────────────────────────────────────────────────────╯",
                "   ⏎ Send Feedback  ·  esc cancel",
            ]
        );
    }

    #[tokio::test]
    async fn narrow_panes_cut_values_and_never_draw_outside_the_area() {
        let shot = Shot {
            slug: "a-very-long-slug-name-that-overflows".into(),
            ..still()
        };
        let lines = harness().rows(props(&shot), 24, 30);
        assert_eq!(
            lines[13..26],
            [
                " 0001 · …      ● Unseen",
                " The home page after l…",
                &rule_row(22),
                " FEEDBACK 0",
                " No comments yet · lea…",
                "  c Send Feedback   s",
                &rule_row(22),
                " Tree      …landing-wt8",
                " Feature   feat",
                " File      …01-home.png",
                " Time      …1T14:54:00Z",
                " Review    Unseen",
                " Path      …roshot/feat",
            ]
        );
        // Page header too wide for the row: the right text follows the left.
        let page = harness().rows(
            DetailPaneProps {
                mode: DetailMode::Page,
                ..props(&shot)
            },
            24,
            30,
        );
        assert_eq!(page[0], " ‹ Stream2 / 7  ‹ older");
        // Below 22 columns the layout width stays 20 and is clipped.
        let tiny = harness().rows(props(&shot), 16, 30);
        assert_eq!(tiny[0], " Detail");
        assert_eq!(tiny[14], " The home page a");
        for (width, height) in [(0, 0), (1, 1), (2, 5), (5, 3), (16, 1), (60, 7)] {
            harness().draw(
                DetailPaneProps {
                    composer: true,
                    inline_player: true,
                    mode: DetailMode::Page,
                    ..props(&movie())
                },
                width,
                height,
            );
        }
    }

    // ---- image layer -------------------------------------------------------

    #[tokio::test]
    async fn picture_reserves_the_preview_box_and_survives_a_shot_change() {
        let shot = still();
        let mut h = harness_with(GraphicsProtocol::Kitty, false);
        let buf = h.draw(
            DetailPaneProps {
                zoom: 2.0,
                pan_x: 0.25,
                pan_y: 0.75,
                ..props(&shot)
            },
            60,
            30,
        );
        // Kitty paints the picture out of band: no caption in the cells.
        assert_eq!(row(&buf, 6), "");
        let picture = h.state.picture().expect("picture mounted");
        assert_eq!(
            picture.cell_box(),
            Some(CellBox {
                x: 1,
                y: 1,
                width: 58,
                height: 12
            })
        );
        let id = picture.image_id();
        assert!(id.is_some());
        // Another shot reuses the same layer entry (no React key on <Picture>).
        let other = Shot {
            path: "/w/.astroshot/feat/0002-next.png".into(),
            ..still()
        };
        h.draw(props(&other), 60, 30);
        assert_eq!(h.state.picture().and_then(PictureState::image_id), id);
        assert_eq!(h.wakes.load(Ordering::SeqCst), 0);
    }
}
