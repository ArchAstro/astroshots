//! Port of `packages/astroshot-review/src/ui/stream.tsx`.
//!
//! The Shots stream: filter bar, contiguous worktree groups, and one row per
//! image with a live thumbnail.
//!
//! - `width`/`height` props are the `Rect` each widget renders into.
//! - `StreamItem.height` is the [`StreamItem::height`] method.
//! - `StreamList` keeps one [`PictureState`] per visible shot keyed by the
//!   shot path (the React `key`); a shot that scrolls out of view drops its
//!   state, which unregisters its image.
//! - A group header too wide for its row keeps the arrow and the worktree
//!   chip and cuts the count text with `…`. Ink shrinks all three in
//!   proportion and lets them overlap.

use std::collections::{HashMap, HashSet};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{StatefulWidget, Widget};

use super::chrome::{EmptyState, execution_pill, movie_badge, review_badge, worktree_chip};
use super::context::AppServices;
use super::hooks::Wake;
use super::inline::{fill_bg, put_truncated, space_between};
use super::picture::{Picture, PictureProps, PictureState};
use super::put_spans;
use super::selectors::{StreamCounts, StreamFilter, StreamGroup, review_state_of};
use super::theme::{THEME, clock_time, truncate};
use astroshot_engine::review_data::manifest::duration_label;
use astroshot_engine::review_data::model::{FeatureStatus, ReviewState, Shot};

#[derive(Debug, Clone)]
pub enum StreamItem<'a> {
    Header {
        group: &'a StreamGroup<'a>,
    },
    Shot {
        shot: &'a Shot,
        group: &'a StreamGroup<'a>,
    },
}

pub const SHOT_ROW_HEIGHT: usize = 4;

impl<'a> StreamItem<'a> {
    /// Lines the item occupies (TS `height`).
    pub fn height(&self) -> usize {
        match self {
            StreamItem::Header { .. } => 1,
            StreamItem::Shot { .. } => SHOT_ROW_HEIGHT,
        }
    }

    pub fn group(&self) -> &'a StreamGroup<'a> {
        match self {
            StreamItem::Header { group } | StreamItem::Shot { group, .. } => group,
        }
    }
}

pub fn flatten_stream<'a>(
    groups: &'a [StreamGroup<'a>],
    collapsed: &HashSet<String>,
) -> Vec<StreamItem<'a>> {
    let mut items = Vec::new();
    for group in groups {
        items.push(StreamItem::Header { group });
        if collapsed.contains(group.id) {
            continue;
        }
        for shot in &group.shots {
            items.push(StreamItem::Shot { shot, group });
        }
    }
    items
}

/// Headers of expanded groups are labels, not stops; collapsed ones stay reachable.
pub fn is_navigable(item: &StreamItem<'_>, collapsed: &HashSet<String>) -> bool {
    matches!(item, StreamItem::Shot { .. }) || collapsed.contains(item.group().id)
}

/// Nearest navigable index at or after `from` (direction +1) or before it (−1).
pub fn next_navigable(
    items: &[StreamItem<'_>],
    collapsed: &HashSet<String>,
    from: usize,
    direction: isize,
) -> usize {
    let len = items.len() as isize;
    let navigable = |index: isize| is_navigable(&items[index as usize], collapsed);
    let mut index = from as isize;
    while index >= 0 && index < len {
        if navigable(index) {
            return index as usize;
        }
        index += direction;
    }
    // Fall back to the nearest navigable item in the other direction.
    index = from as isize - direction;
    while index >= 0 && index < len {
        if navigable(index) {
            return index as usize;
        }
        index -= direction;
    }
    from.min(items.len().saturating_sub(1))
}

/// First item index so the cursor item is fully visible within `height` lines.
pub fn scroll_window(
    items: &[StreamItem<'_>],
    cursor: usize,
    scroll_top: usize,
    height: usize,
) -> usize {
    if items.is_empty() {
        return 0;
    }
    let cursor = cursor.min(items.len() - 1);
    let mut top = scroll_top.min(items.len() - 1);
    if cursor < top {
        top = cursor;
    }
    let fits = |start: usize| {
        let mut used = 0;
        for item in &items[start..=cursor] {
            used += item.height();
            if used > height {
                return false;
            }
        }
        true
    };
    while !fits(top) && top < cursor {
        top += 1;
    }
    top
}

/// Props of [`StreamList`]; TS `width`/`height` are the render `Rect`.
#[derive(Debug, Clone)]
pub struct StreamProps<'a> {
    pub items: &'a [StreamItem<'a>],
    pub collapsed: &'a HashSet<String>,
    pub cursor: usize,
    pub scroll_top: usize,
    pub filter: StreamFilter,
    pub movies_only: bool,
    pub counts: StreamCounts,
    pub focused: bool,
    pub scanning: bool,
    pub has_roots: bool,
    pub total_shots: usize,
    pub bulk_busy: bool,
}

/// `Math.round` for non-negative values.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

fn thumbnail_cols(cell_width: u32, cell_height: u32) -> u16 {
    let rows = (SHOT_ROW_HEIGHT - 1) as f64;
    let height_px = rows * f64::from(cell_height);
    let cols = js_round(height_px * (88.0 / 56.0) / f64::from(cell_width));
    cols.clamp(6.0, 16.0) as u16
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

/// The `Unseen (n)` / `History (n)` title row with its key hints.
pub struct FilterBar {
    pub filter: StreamFilter,
    pub movies_only: bool,
    pub counts: StreamCounts,
    pub bulk_busy: bool,
}

impl Widget for FilterBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height == 0 {
            return;
        }
        let inner = Rect::new(area.x + 1, area.y, area.width - 2, 1);
        let unseen = self.filter == StreamFilter::Unseen;
        let title = if unseen {
            format!("Unseen ({})", self.counts.pending)
        } else {
            format!("History ({})", self.counts.seen)
        };
        let mut right: Vec<Span<'static>> = Vec::new();
        if self.counts.movies > 0 {
            let mut style = Style::new().fg(if self.movies_only {
                THEME.purple
            } else {
                THEME.muted
            });
            if self.movies_only {
                style = style.add_modifier(Modifier::REVERSED);
            }
            right.push(Span::styled(" m Movies ", style));
        }
        if unseen && self.counts.pending > 0 {
            let (text, color) = if self.bulk_busy {
                (" S Marking… ", THEME.muted)
            } else {
                (" S Seen all ", THEME.green)
            };
            right.push(Span::styled(text, Style::new().fg(color)));
        }
        right.push(Span::styled(
            if unseen { " u History " } else { " u Unseen " },
            Style::new().fg(THEME.blue),
        ));
        space_between(buf, inner, inner.y, &[Span::styled(title, bold())], &right);
    }
}

fn render_group_header(
    buf: &mut Buffer,
    area: Rect,
    group: &StreamGroup<'_>,
    collapsed: bool,
    selected: bool,
) {
    let width = usize::from(area.width);
    let unseen = group
        .shots
        .iter()
        .filter(|shot| review_state_of(shot) != ReviewState::Seen)
        .count();
    if selected {
        fill_bg(buf, area, THEME.selection);
    }
    let arrow = Span::styled(
        if collapsed { "▸ " } else { "▾ " },
        Style::new().fg(if selected { THEME.brand } else { THEME.muted }),
    );
    let chip = worktree_chip(group.worktree_short);
    let x = put_spans(buf, area, area.x + 1, area.y, &[arrow, chip]);

    let location = if group.worktree != group.worktree_short {
        format!(
            "{} · ",
            truncate(group.worktree, width.saturating_sub(26).max(8))
        )
    } else {
        String::new()
    };
    let count = group.shots.len();
    let frames = if count == 1 { "frame" } else { "frames" };
    let mut spans = vec![Span::styled(
        format!(" {location}{count} {frames}"),
        Style::new().fg(THEME.muted),
    )];
    if unseen > 0 {
        spans.push(Span::styled(
            format!(" · {unseen} unseen"),
            Style::new().fg(THEME.amber),
        ));
    }
    let max = usize::from(area.right().saturating_sub(x));
    put_truncated(buf, area, x, area.y, &spans, max);
}

fn status_text(status: FeatureStatus) -> &'static str {
    match status {
        FeatureStatus::Running => "running",
        FeatureStatus::Pass => "pass",
        FeatureStatus::Fail => "fail",
        FeatureStatus::Idle => "idle",
    }
}

fn render_shot_row(
    buf: &mut Buffer,
    area: Rect,
    shot: &Shot,
    selected: bool,
    thumb_cols: u16,
    in_group: bool,
    picture: &mut PictureState,
) {
    let width = usize::from(area.width);
    let state = review_state_of(shot);
    let text_width_cols = width.saturating_sub(usize::from(thumb_cols) + 4).max(12);
    let time = clock_time(shot.captured_at);
    let title_width = text_width_cols
        .saturating_sub(time.chars().count() + 2)
        .max(8);
    let row_height = (SHOT_ROW_HEIGHT - 1) as u16;

    // Full-height selection bar. The highlight stays off the thumbnail cells
    // so selecting a row never repaints under the image (which made the herdr
    // layer flicker).
    if selected {
        fill_bg(
            buf,
            Rect::new(area.x, area.y, 1, row_height).intersection(area),
            THEME.green,
        );
    }
    let picture_area = Rect::new(area.x + 1, area.y, thumb_cols, row_height).intersection(area);
    if !picture_area.is_empty() {
        let props = PictureProps {
            src: Some(&shot.path),
            version: shot.mtime_ms,
            label: Some(if shot.is_movie { "▶ movie" } else { "still" }),
            ..PictureProps::default()
        };
        Picture::new(props).render(picture_area, buf, picture);
    }

    let text_area = Rect::new(
        area.x + 1 + thumb_cols + 1,
        area.y,
        text_width_cols as u16,
        row_height,
    )
    .intersection(area);
    if text_area.is_empty() {
        return;
    }
    if selected {
        fill_bg(buf, text_area, THEME.selection);
    }

    let mut title: Vec<Span<'static>> = Vec::new();
    if !in_group {
        title.push(worktree_chip(&shot.worktree_short));
    }
    if shot.status == Some(FeatureStatus::Fail) {
        title.push(Span::styled("● ", Style::new().fg(THEME.red)));
    }
    title.push(Span::styled(
        truncate(&format!("{} · {}", shot.feature, shot.title), title_width),
        bold(),
    ));
    space_between(
        buf,
        text_area,
        text_area.y,
        &title,
        &[Span::styled(time, Style::new().fg(THEME.muted))],
    );

    let description = if shot.description.is_empty() {
        &shot.file_name
    } else {
        &shot.description
    };
    put_truncated(
        buf,
        text_area,
        text_area.x,
        text_area.y + 1,
        &[Span::styled(
            truncate(description, text_width_cols),
            Style::new().fg(THEME.muted),
        )],
        usize::from(text_area.width),
    );

    let mut badges: Vec<Span<'static>> = Vec::new();
    if shot.is_movie {
        badges.push(movie_badge(duration_label(shot.duration_ms).as_deref()));
        badges.push(Span::raw(" "));
    }
    badges.push(review_badge(
        state,
        shot.review.as_ref().is_some_and(|review| review.is_stale),
    ));
    if let Some(status) = shot.status {
        badges.push(Span::raw(" "));
        if let Some(pill) = execution_pill(Some(status_text(status))) {
            badges.push(pill);
        }
    }
    put_truncated(
        buf,
        text_area,
        text_area.x,
        text_area.y + 2,
        &badges,
        usize::from(text_area.width),
    );
}

/// Per-list component state: one picture per visible shot, keyed by path.
pub struct StreamListState {
    services: AppServices,
    wake: Wake,
    pictures: HashMap<String, PictureState>,
}

impl StreamListState {
    pub fn new(services: &AppServices, wake: Wake) -> Self {
        Self {
            services: services.clone(),
            wake,
            pictures: HashMap::new(),
        }
    }
}

pub struct StreamList<'a> {
    pub props: StreamProps<'a>,
}

impl<'a> StreamList<'a> {
    pub fn new(props: StreamProps<'a>) -> Self {
        Self { props }
    }
}

impl StatefulWidget for StreamList<'_> {
    type State = StreamListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut StreamListState) {
        let props = &self.props;
        let capabilities = &state.services.capabilities;
        let thumb_cols = thumbnail_cols(capabilities.cell_width, capabilities.cell_height);
        let mut empty = |title: &str, body: &str, action: Option<&str>| {
            EmptyState {
                title,
                body,
                action,
            }
            .render(area, buf);
        };

        if !props.has_roots {
            state.pictures.clear();
            let hint = format!("{} --root <dir>", state.services.command);
            return empty(
                "Choose folders to watch",
                "Pick the directories that contain your projects. Astroshots watches them for .astroshot/ screenshots.",
                Some(&hint),
            );
        }
        if props.total_shots == 0 {
            state.pictures.clear();
            return if props.scanning {
                empty(
                    "Scanning watched folders…",
                    "First scan of a large folder can take a bit. Results stream in as they are found.",
                    None,
                )
            } else {
                empty(
                    "Waiting for frames",
                    "When any project under a watched folder writes to .astroshot/, shots land here.",
                    Some("r Rescan"),
                )
            };
        }
        if props.items.is_empty() {
            state.pictures.clear();
            return if props.filter == StreamFilter::Unseen {
                empty(
                    "You’re all caught up",
                    "Every current frame has been seen.",
                    Some("u View history"),
                )
            } else {
                empty(
                    "No history yet",
                    "Frames you mark Seen will appear here.",
                    Some("u Back to unseen"),
                )
            };
        }

        let height = usize::from(area.height);
        let mut used = 0;
        let mut rendered = 0;
        let mut visible: HashSet<String> = HashSet::new();
        for index in props.scroll_top..props.items.len() {
            let item = &props.items[index];
            if used + item.height() > height {
                break;
            }
            let row = Rect::new(
                area.x,
                area.y + used as u16,
                area.width,
                item.height() as u16,
            );
            used += item.height();
            rendered += 1;
            let selected = props.focused && index == props.cursor;
            match item {
                StreamItem::Header { group } => render_group_header(
                    buf,
                    row,
                    group,
                    props.collapsed.contains(group.id),
                    selected,
                ),
                StreamItem::Shot { shot, .. } => {
                    visible.insert(shot.path.clone());
                    let services = &state.services;
                    let wake = &state.wake;
                    let picture = state
                        .pictures
                        .entry(shot.path.clone())
                        .or_insert_with(|| PictureState::new(services, wake.clone()));
                    render_shot_row(buf, row, shot, selected, thumb_cols, true, picture);
                }
            }
        }
        state.pictures.retain(|path, _| visible.contains(path));

        let remaining = props
            .items
            .len()
            .saturating_sub(props.scroll_top + rendered);
        if remaining > 0 && used < height {
            put_spans(
                buf,
                area,
                area.x,
                area.y + used as u16,
                &[Span::styled(
                    format!("  ↓ {remaining} more"),
                    Style::new().fg(THEME.faint),
                )],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::probe::GraphicsProtocol;
    use crate::ui::context::test_support::{FakeService, services};
    use crate::ui::selectors::contiguous_groups;
    use crate::ui::testing::{bg_at, fg_at, modifier_at, render, render_stateful, rows};
    use astroshot_engine::review_data::model::ReviewSnapshot;
    use ratatui::style::Color;
    use std::sync::Arc;

    fn shot(path: &str, worktree_path: &str, f: impl FnOnce(&mut Shot)) -> Shot {
        let mut shot = Shot {
            id: path.into(),
            path: path.into(),
            file_name: "0001-a.png".into(),
            worktree_path: worktree_path.into(),
            worktree: "w".into(),
            worktree_short: "w".into(),
            feature: "f".into(),
            feature_dir: "/w/.astroshot/f".into(),
            sequence: Some("0001".into()),
            slug: "a".into(),
            title: "A".into(),
            description: String::new(),
            url: None,
            run_id: Some("r".into()),
            status: None,
            captured_at: 0.0,
            mtime_ms: 0.0,
            is_movie: false,
            video_file_name: None,
            video_path: None,
            duration_ms: None,
            source: None,
            chapters: vec![],
            review: None,
        };
        f(&mut shot);
        shot
    }

    fn seen() -> ReviewSnapshot {
        ReviewSnapshot {
            state: ReviewState::Seen,
            decision: Some("seen".into()),
            hash_matches: true,
            is_stale: false,
            comments: vec![],
            reviewed_at: None,
        }
    }

    fn shots() -> Vec<Shot> {
        vec![
            shot("/w1/1", "/w1", |s| {
                s.sequence = Some("0002".into());
                s.captured_at = 4.0;
                s.is_movie = true;
            }),
            shot("/w2/1", "/w2", |s| s.captured_at = 3.0),
            shot("/w1/2", "/w1", |s| {
                s.captured_at = 2.0;
                s.review = Some(seen());
            }),
            shot("/w1/3", "/w1", |s| {
                s.feature = "g".into();
                s.captured_at = 1.0;
            }),
        ]
    }

    #[test]
    fn flattens_groups_skips_expanded_headers_when_navigating_and_windows_around_the_cursor() {
        let shots = shots();
        let groups = contiguous_groups(&shots);
        let collapsed = HashSet::new();
        let items = flatten_stream(&groups, &collapsed);
        let kinds: Vec<_> = items
            .iter()
            .map(|item| matches!(item, StreamItem::Header { .. }))
            .collect();
        // header, shot, header, shot, header, shot, shot
        assert_eq!(kinds, [true, false, true, false, true, false, false]);
        assert!(!is_navigable(&items[0], &collapsed));
        assert_eq!(next_navigable(&items, &collapsed, 0, 1), 1);
        assert_eq!(next_navigable(&items, &collapsed, 2, 1), 3);
        assert_eq!(next_navigable(&items, &collapsed, 2, -1), 1);
        let collapsed_first = HashSet::from([groups[0].id.to_string()]);
        assert!(is_navigable(
            &flatten_stream(&groups, &collapsed_first)[0],
            &collapsed_first
        ));
        // 7 items: heights 1,4,1,4,1,4,4. A 10-line window ending at item 6 starts at item 4 (1+4+4).
        assert_eq!(scroll_window(&items, 6, 0, 10), 4);
        assert_eq!(scroll_window(&items, 1, 3, 10), 1);
        assert_eq!(scroll_window(&[], 0, 0, 10), 0);
    }

    #[test]
    fn next_navigable_falls_back_to_the_other_direction_then_clamps() {
        let shots = shots();
        let groups = contiguous_groups(&shots);
        let expanded = HashSet::new();
        let items = flatten_stream(&groups, &expanded);
        // Nothing past the last item: the nearest navigable item before it.
        assert_eq!(next_navigable(&items, &expanded, 7, 1), 6);
        // Only headers (every group collapsed -> all navigable); with none navigable it clamps.
        let only_headers = &items[..1];
        assert_eq!(next_navigable(only_headers, &expanded, 5, 1), 0);
        assert_eq!(next_navigable(&[], &expanded, 3, 1), 0);
    }

    fn state_and_wake() -> (AppServices, Arc<FakeService>) {
        let service = FakeService::new(false);
        (services(GraphicsProtocol::None, service.clone()), service)
    }

    fn props<'a>(items: &'a [StreamItem<'a>], collapsed: &'a HashSet<String>) -> StreamProps<'a> {
        StreamProps {
            items,
            collapsed,
            cursor: 0,
            scroll_top: 0,
            filter: StreamFilter::Unseen,
            movies_only: false,
            counts: StreamCounts {
                pending: 3,
                seen: 1,
                movies: 1,
            },
            focused: true,
            scanning: false,
            has_roots: true,
            total_shots: 4,
            bulk_busy: false,
        }
    }

    #[test]
    fn thumbnail_columns_follow_cell_aspect_and_clamp() {
        assert_eq!(thumbnail_cols(10, 20), 9);
        assert_eq!(thumbnail_cols(1, 20), 16);
        assert_eq!(thumbnail_cols(100, 20), 6);
    }

    #[test]
    fn filter_bar_shows_unseen_title_with_hints_and_styles() {
        let counts = StreamCounts {
            pending: 3,
            seen: 1,
            movies: 1,
        };
        let bar = |filter, movies_only, bulk_busy| {
            render(
                FilterBar {
                    filter,
                    movies_only,
                    counts,
                    bulk_busy,
                },
                60,
                1,
            )
        };
        let buf = bar(StreamFilter::Unseen, false, false);
        assert_eq!(
            rows(&buf),
            [" Unseen (3)                m Movies  S Seen all  u History"]
        );
        assert_eq!(modifier_at(&buf, 1, 0), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 27, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 40, 0), THEME.green);
        assert_eq!(fg_at(&buf, 50, 0), THEME.blue);

        let buf = bar(StreamFilter::Unseen, true, true);
        assert_eq!(
            rows(&buf),
            [" Unseen (3)                m Movies  S Marking…  u History"]
        );
        assert_eq!(fg_at(&buf, 27, 0), THEME.purple);
        assert_eq!(modifier_at(&buf, 27, 0), Modifier::REVERSED);
        assert_eq!(fg_at(&buf, 40, 0), THEME.muted);

        let buf = bar(StreamFilter::History, false, false);
        assert_eq!(
            rows(&buf),
            [" History (1)                            m Movies  u Unseen"]
        );
    }

    /// 60 columns: bar(1) + thumbnail(9) + gap(1) + 47 text columns, then 2 spare.
    fn title_row(title: &str, time: &str) -> String {
        format!("           {title:<39}{time}")
    }

    #[tokio::test]
    async fn stream_list_renders_headers_rows_and_the_more_indicator() {
        let (services, _) = state_and_wake();
        let mut state = StreamListState::new(&services, Arc::new(|| {}));
        let shots = shots();
        let groups = contiguous_groups(&shots);
        let collapsed = HashSet::new();
        let items = flatten_stream(&groups, &collapsed);
        let mut p = props(&items, &collapsed);
        p.focused = false;
        // header(1) + shot(4) + header(1) + shot(4) + header(1) = 11 lines, one spare for "more".
        let buf = render_stateful(StreamList::new(p), &mut state, 60, 12);
        let time = clock_time(0.0);
        let expected = [
            " ▾  w  1 frame · 1 unseen".to_string(),
            title_row("f · A", &time),
            "  ▶ movie  0001-a.png".to_string(),
            "            Movie  ● Unseen".to_string(),
            String::new(),
            " ▾  w  1 frame · 1 unseen".to_string(),
            title_row("f · A", &time),
            "   still   0001-a.png".to_string(),
            "           ● Unseen".to_string(),
            String::new(),
            " ▾  w  2 frames · 1 unseen".to_string(),
            "  ↓ 2 more".to_string(),
        ];
        assert_eq!(rows(&buf), expected);
        // One picture state per visible shot, keyed by path.
        assert_eq!(state.pictures.len(), 2);
        assert_eq!(fg_at(&buf, 2, 11), THEME.faint);
    }

    #[tokio::test]
    async fn stream_list_highlights_the_selected_row_off_the_thumbnail() {
        let (services, _) = state_and_wake();
        let mut state = StreamListState::new(&services, Arc::new(|| {}));
        let shots = shots();
        let groups = contiguous_groups(&shots);
        let collapsed = HashSet::new();
        let items = flatten_stream(&groups, &collapsed);
        let mut p = props(&items, &collapsed);
        p.cursor = 1;
        let buf = render_stateful(StreamList::new(p.clone()), &mut state, 60, 12);
        for y in 1..=3 {
            assert_eq!(bg_at(&buf, 0, y), THEME.green, "bar row {y}");
            assert_eq!(bg_at(&buf, 5, y), Color::Reset, "thumbnail row {y}");
            assert_eq!(bg_at(&buf, 10, y), Color::Reset, "gap row {y}");
            if y != 3 {
                assert_eq!(bg_at(&buf, 11, y), THEME.selection, "text row {y}");
            }
            assert_eq!(bg_at(&buf, 45, y), THEME.selection, "text row {y}");
            assert_eq!(bg_at(&buf, 57, y), THEME.selection, "text end row {y}");
            assert_eq!(bg_at(&buf, 58, y), Color::Reset, "past text row {y}");
        }
        // The spacer line below the shot is not highlighted, the header is not either.
        assert_eq!(bg_at(&buf, 11, 4), Color::Reset);
        assert_eq!(bg_at(&buf, 0, 0), Color::Reset);
        assert_eq!(fg_at(&buf, 11, 1), Color::Reset);
        assert_eq!(modifier_at(&buf, 11, 1), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 11, 3), THEME.purple);

        // Cursor on a header highlights the whole row and brightens the arrow.
        p.cursor = 0;
        let buf = render_stateful(StreamList::new(p.clone()), &mut state, 60, 12);
        assert_eq!(bg_at(&buf, 0, 0), THEME.selection);
        assert_eq!(bg_at(&buf, 59, 0), THEME.selection);
        assert_eq!(fg_at(&buf, 1, 0), THEME.brand);
        assert_eq!(bg_at(&buf, 0, 1), Color::Reset);

        // An unfocused list highlights nothing.
        p.focused = false;
        let buf = render_stateful(StreamList::new(p), &mut state, 60, 12);
        assert_eq!(bg_at(&buf, 0, 0), Color::Reset);
        assert_eq!(fg_at(&buf, 1, 0), THEME.muted);
    }

    #[tokio::test]
    async fn stream_list_scrolls_to_the_window_start_and_drops_offscreen_pictures() {
        let (services, _) = state_and_wake();
        let mut state = StreamListState::new(&services, Arc::new(|| {}));
        let shots = shots();
        let groups = contiguous_groups(&shots);
        let collapsed = HashSet::new();
        let items = flatten_stream(&groups, &collapsed);
        let mut p = props(&items, &collapsed);
        p.scroll_top = 4;
        p.cursor = 6;
        let buf = render_stateful(StreamList::new(p.clone()), &mut state, 60, 10);
        let lines = rows(&buf);
        // Header of the third group, then its two shots; nothing remains below.
        assert_eq!(lines[0], " ▾  w  2 frames · 1 unseen");
        assert_eq!(lines[1], title_row("f · A", &clock_time(0.0)));
        assert_eq!(lines[5], title_row("g · A", &clock_time(0.0)));
        assert!(lines.iter().all(|line| !line.contains('↓')));
        assert_eq!(bg_at(&buf, 0, 5), THEME.green);
        assert_eq!(
            state.pictures.keys().collect::<HashSet<_>>(),
            HashSet::from([&"/w1/2".to_string(), &"/w1/3".to_string()])
        );
    }

    #[tokio::test]
    async fn stream_list_marks_collapsed_groups_and_shows_the_worktree_path() {
        let (services, _) = state_and_wake();
        let mut state = StreamListState::new(&services, Arc::new(|| {}));
        let mut shots = shots();
        for shot in &mut shots {
            shot.worktree = format!("/Users/me{}", shot.worktree_path);
        }
        let groups = contiguous_groups(&shots);
        let collapsed = HashSet::from([groups[0].id.to_string()]);
        let items = flatten_stream(&groups, &collapsed);
        let mut p = props(&items, &collapsed);
        p.cursor = 0;
        let buf = render_stateful(StreamList::new(p), &mut state, 50, 6);
        let lines = rows(&buf);
        assert_eq!(lines[0], " ▸  w  /Users/me/w1 · 1 frame · 1 unseen");
        assert_eq!(lines[1], " ▾  w  /Users/me/w2 · 1 frame · 1 unseen");
        // The collapsed group's shot is gone, so the next group's shot follows its header.
        assert!(lines[2].contains("f · A"));
    }

    #[tokio::test]
    async fn stream_list_row_shows_failures_descriptions_pills_and_stale_badges() {
        let (services, _) = state_and_wake();
        let mut state = StreamListState::new(&services, Arc::new(|| {}));
        let shots = vec![shot("/w1/1", "/w1", |s| {
            s.status = Some(FeatureStatus::Fail);
            s.description = "  Checkout   page\nloaded ".into();
            s.is_movie = true;
            s.duration_ms = Some(2800.0);
            s.review = Some(ReviewSnapshot {
                is_stale: true,
                state: ReviewState::Pending,
                ..seen()
            });
        })];
        let groups = contiguous_groups(&shots);
        let collapsed = HashSet::new();
        let items = flatten_stream(&groups, &collapsed);
        let buf = render_stateful(
            StreamList::new(props(&items, &collapsed)),
            &mut state,
            70,
            5,
        );
        let lines = rows(&buf);
        assert!(lines[1].starts_with("           ● f · A"), "{}", lines[1]);
        assert_eq!(fg_at(&buf, 11, 1), THEME.red);
        assert_eq!(lines[2], "  ▶ movie  Checkout page loaded");
        assert_eq!(
            lines[3],
            "            Movie · 2.8s  ● Unseen · changed · run fail"
        );
        assert_eq!(fg_at(&buf, 30, 3), THEME.amber);
        assert_eq!(fg_at(&buf, 46, 3), THEME.red);
    }

    #[tokio::test]
    async fn stream_list_truncates_long_titles_with_an_ellipsis() {
        let (services, _) = state_and_wake();
        let mut state = StreamListState::new(&services, Arc::new(|| {}));
        let shots = vec![shot("/w1/1", "/w1", |s| {
            s.title =
                "An extremely long title that cannot possibly fit in the available room".into();
            s.description = "d".repeat(100);
        })];
        let groups = contiguous_groups(&shots);
        let collapsed = HashSet::new();
        let items = flatten_stream(&groups, &collapsed);
        let buf = render_stateful(
            StreamList::new(props(&items, &collapsed)),
            &mut state,
            40,
            5,
        );
        let lines = rows(&buf);
        // text column: 40 - 9 - 4 = 27 -> max(12, 27); title budget 27 - 8 - 2 = 17.
        assert!(
            lines[1].starts_with("           f · An extremely…  "),
            "{}",
            lines[1]
        );
        assert_eq!(lines[2].chars().skip(11).count(), 27, "{}", lines[2]);
        assert!(lines[2].ends_with('…'));
    }

    fn empty_rows(configure: impl FnOnce(&mut StreamProps<'_>)) -> Vec<String> {
        let service = FakeService::new(false);
        let mut state =
            StreamListState::new(&services(GraphicsProtocol::None, service), Arc::new(|| {}));
        let collapsed = HashSet::new();
        let mut p = props(&[], &collapsed);
        configure(&mut p);
        let buf = render_stateful(StreamList::new(p), &mut state, 40, 9);
        rows(&buf)
            .into_iter()
            .filter(|line| !line.is_empty())
            .map(|line| line.trim().to_string())
            .collect()
    }

    #[tokio::test]
    async fn stream_list_empty_states() {
        assert_eq!(
            empty_rows(|p| p.has_roots = false),
            [
                "Choose folders to watch",
                "Pick the directories that contain",
                "your projects. Astroshots watches",
                "them for .astroshot/ screenshots.",
                "astroshot review --root <dir>",
            ]
        );
        assert_eq!(
            empty_rows(|p| {
                p.total_shots = 0;
                p.scanning = true;
            }),
            [
                "Scanning watched folders…",
                "First scan of a large folder can",
                "take a bit. Results stream in as",
                "they are found.",
            ]
        );
        assert_eq!(
            empty_rows(|p| p.total_shots = 0),
            [
                "Waiting for frames",
                "When any project under a watched",
                "folder writes to .astroshot/, shots",
                "land here.",
                "r Rescan",
            ]
        );
        assert_eq!(
            empty_rows(|_| {}),
            [
                "You’re all caught up",
                "Every current frame has been seen.",
                "u View history",
            ]
        );
        assert_eq!(
            empty_rows(|p| p.filter = StreamFilter::History),
            [
                "No history yet",
                "Frames you mark Seen will appear",
                "here.",
                "u Back to unseen",
            ]
        );
    }
}
