//! Port of `packages/astroshot-review/src/ui/friction.tsx`.
//!
//! Friction Logs: scenario list, scenario detail with run picker and steps,
//! step detail, and the step takeover.
//!
//! - `width`/`height` props are the `Rect` each widget renders into.
//! - `FrictionList` renders a one-row-per-4-lines list; the detail views are
//!   plain widgets. The two step views own a [`PictureState`] through
//!   [`FrictionStepState`].
//! - A `<Text wrap="wrap">` run is wrapped with `chrome::wrap_words`.
//!
//! Where the port differs from Ink: content that does not fit. Yoga shrinks
//! every row of an overfull column in proportion to its height, so Ink draws
//! rows on top of each other (a long scenario prompt overwrites its
//! `SCENARIO PROMPT` label; a step with many notes loses its title). The port
//! keeps each row whole and drops the rows past the bottom edge. Likewise a
//! one-line row that is too wide cuts its left text with `…` and keeps the
//! right side, where Ink shrinks both sides. Rows that fit match Ink cell
//! for cell; the expected rows in the tests were captured from the TS
//! components with Ink's `renderToString`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, StatefulWidget, Widget};

use super::chrome::{
    EmptyState, MetaRow, rule, section_label, status_pill, worktree_chip, wrap_words,
};
use super::context::AppServices;
use super::hooks::Wake;
use super::inline::{fill_bg, put_truncated, space_between};
use super::picture::{Picture, PictureProps, PictureState};
use super::put_spans;
use super::selectors::{StreamFilter, friction_state, friction_summary, latest_run};
use super::theme::{THEME, relative_time, truncate};
use crate::astroshot_review::data::friction::{
    friction_status_label, run_display_title, step_count_label,
};
use crate::astroshot_review::data::model::{FrictionLog, FrictionRun, FrictionStep, ReviewState};
use crate::astroshot_review::data::paths::basename;
use crate::tui_shot::kitty_graphics::js_number_string;

pub const FRICTION_ROW_HEIGHT: usize = 4;

/// Props of [`FrictionList`]; TS `width`/`height` are the render `Rect`.
#[derive(Debug, Clone, Copy)]
pub struct FrictionListProps<'a> {
    pub logs: &'a [&'a FrictionLog],
    pub cursor: usize,
    pub scroll_top: usize,
    pub filter: StreamFilter,
    pub pending: usize,
    pub seen: usize,
    pub focused: bool,
    pub total: usize,
    pub now: f64,
}

fn fg(color: Color) -> Style {
    Style::new().fg(color)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

fn bold_fg(color: Color) -> Style {
    fg(color).add_modifier(Modifier::BOLD)
}

fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

/// `String(step.step).padStart(2, "0")`.
fn step_number(step: &FrictionStep) -> String {
    format!("{:0>2}", js_number_string(step.step))
}

/// A negative width in TS reaches `truncate` as an empty result.
fn sub(a: u16, b: usize) -> usize {
    usize::from(a).saturating_sub(b)
}

/// Draw `text` wrapped to `width` columns from row `y`, clipped to `area`.
/// Returns the rows drawn.
fn put_wrapped(
    buf: &mut Buffer,
    area: Rect,
    x: u16,
    y: u16,
    width: usize,
    text: &str,
    style: Style,
) -> u16 {
    let lines = wrap_words(text, width);
    for (offset, line) in lines.iter().enumerate() {
        put_spans(
            buf,
            area,
            x,
            y + offset as u16,
            &[span(line.clone(), style)],
        );
    }
    lines.len() as u16
}

/// `<FrictionFilterBar>`: title plus key hints.
pub struct FrictionFilterBar {
    pub filter: StreamFilter,
    pub pending: usize,
    pub seen: usize,
}

impl Widget for FrictionFilterBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height == 0 {
            return;
        }
        let inner = Rect::new(area.x + 1, area.y, area.width - 2, 1);
        let unseen = self.filter == StreamFilter::Unseen;
        let title = if unseen {
            format!("Unseen ({})", self.pending)
        } else {
            format!("History ({})", self.seen)
        };
        let mut right = Vec::new();
        if unseen && self.pending > 0 {
            right.push(span(" S Seen all ", fg(THEME.green)));
        }
        right.push(span(
            if unseen { " u History " } else { " u Unseen " },
            fg(THEME.blue),
        ));
        space_between(buf, inner, inner.y, &[span(title, bold())], &right);
    }
}

fn render_friction_row(buf: &mut Buffer, area: Rect, log: &FrictionLog, selected: bool, now: f64) {
    let run = latest_run(log);
    let inner = usize::from(area.width).saturating_sub(3);
    let status = friction_status_label(log.status.as_deref());
    let footer = match run {
        Some(run) if log.runs.len() > 1 => {
            format!(
                "{} runs · {}",
                log.runs.len(),
                run_display_title(&run.run_id)
            )
        }
        Some(run) => run_display_title(&run.run_id),
        None => "Prompt only · no runs yet".to_string(),
    };
    if selected {
        fill_bg(buf, area, THEME.selection);
    }
    put_spans(
        buf,
        area,
        area.x,
        area.y,
        &[span(if selected { "▎" } else { " " }, fg(THEME.brand))],
    );
    let column = Rect::new(area.x + 2, area.y, inner as u16, area.height).intersection(area);
    if column.is_empty() {
        return;
    }
    let row = |n: u16| column.y + n;

    let pill: Vec<Span<'static>> = status_pill(status.as_deref()).into_iter().collect();
    space_between(
        buf,
        Rect {
            height: 1,
            y: row(0),
            ..column
        },
        row(0),
        &[span(
            truncate(&log.title, inner.saturating_sub(12).max(8)),
            bold(),
        )],
        &pill,
    );
    put_truncated(
        buf,
        column,
        column.x,
        row(1),
        &[span(truncate(&log.description, inner), fg(THEME.muted))],
        inner,
    );
    space_between(
        buf,
        column,
        row(2),
        &[
            worktree_chip(&log.worktree_short),
            span(format!(" {}", log.slug), fg(THEME.muted)),
        ],
        &[span(friction_summary(log), fg(THEME.muted))],
    );
    let seen = friction_state(log) == ReviewState::Seen;
    let mut state = vec![span(
        if seen { "Seen" } else { "Unseen" },
        fg(if seen { THEME.blue } else { THEME.amber }),
    )];
    if let Some(run) = run {
        state.push(span(
            format!(" · {}", relative_time(run.captured_at, now)),
            fg(THEME.muted),
        ));
    }
    space_between(
        buf,
        column,
        row(3),
        &[span(footer, fg(THEME.muted))],
        &state,
    );
}

pub fn friction_scroll(count: usize, cursor: usize, scroll_top: usize, height: usize) -> usize {
    let per_page = (height / FRICTION_ROW_HEIGHT).max(1);
    let mut top = scroll_top.min(count.saturating_sub(1));
    if cursor < top {
        top = cursor;
    }
    if cursor >= top + per_page {
        top = cursor - per_page + 1;
    }
    top
}

pub struct FrictionList<'a> {
    pub props: FrictionListProps<'a>,
}

impl<'a> FrictionList<'a> {
    pub fn new(props: FrictionListProps<'a>) -> Self {
        Self { props }
    }
}

impl Widget for FrictionList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let props = &self.props;
        if props.total == 0 {
            let hints = 2.min(area.height);
            EmptyState {
                title: "No friction logs yet",
                body: "Author a scenario with the friction-log skill, then run it. Results land under .astroshot/friction-logs/.",
                action: None,
            }
            .render(Rect { height: area.height - hints, ..area }, buf);
            let hint_area = Rect::new(
                area.x + 2,
                area.bottom() - hints,
                area.width.saturating_sub(4),
                hints,
            );
            let muted = fg(THEME.muted);
            put_truncated(
                buf,
                hint_area,
                hint_area.x,
                hint_area.y,
                &[span(".astroshot/friction-logs/<slug>/prompt.md", muted)],
                usize::from(hint_area.width),
            );
            put_truncated(
                buf,
                hint_area,
                hint_area.x,
                hint_area.y + 1,
                &[span("runs/<run-id>/log.jsonl + screenshots", muted)],
                usize::from(hint_area.width),
            );
            return;
        }
        if props.logs.is_empty() {
            return if props.filter == StreamFilter::Unseen {
                EmptyState {
                    title: "You’re all caught up",
                    body: "Every friction log has been seen.",
                    action: Some("u View history"),
                }
                .render(area, buf)
            } else {
                EmptyState {
                    title: "No history yet",
                    body: "Logs you mark Seen will appear here.",
                    action: Some("u Back to unseen"),
                }
                .render(area, buf)
            };
        }
        let per_page = (usize::from(area.height) / FRICTION_ROW_HEIGHT).max(1);
        let start = props.scroll_top.min(props.logs.len());
        let end = (props.scroll_top + per_page).min(props.logs.len());
        for (offset, log) in props.logs[start..end].iter().enumerate() {
            let row = Rect::new(
                area.x,
                area.y + (offset * FRICTION_ROW_HEIGHT) as u16,
                area.width,
                FRICTION_ROW_HEIGHT as u16,
            )
            .intersection(area);
            let selected = props.focused && props.scroll_top + offset == props.cursor;
            render_friction_row(buf, row, log, selected, props.now);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrictionLogDetailProps<'a> {
    pub log: &'a FrictionLog,
    pub run: Option<&'a FrictionRun>,
    pub step_cursor: usize,
    pub prompt_open: bool,
    pub prompt: Option<&'a str>,
}

/// Scenario detail: header, prompt, run picker, improve rollup, and steps.
pub struct FrictionLogDetail<'a> {
    pub props: FrictionLogDetailProps<'a>,
}

impl<'a> FrictionLogDetail<'a> {
    pub fn new(props: FrictionLogDetailProps<'a>) -> Self {
        Self { props }
    }
}

impl Widget for FrictionLogDetail<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let FrictionLogDetailProps {
            log,
            run,
            step_cursor,
            prompt_open,
            prompt,
        } = self.props;
        if area.width < 2 {
            return;
        }
        let inner_area = Rect::new(area.x + 1, area.y, area.width - 2, area.height);
        let inner = usize::from(inner_area.width);
        let run_index = run.and_then(|run| log.runs.iter().position(|r| std::ptr::eq(r, run)));
        let improve: Vec<(&FrictionStep, &String)> = run
            .map(|run| {
                run.steps
                    .iter()
                    .flat_map(|step| step.improve.iter().map(move |note| (step, note)))
                    .collect()
            })
            .unwrap_or_default();
        let header_lines = 6
            + if prompt_open { 6 } else { 0 }
            + if improve.is_empty() {
                0
            } else {
                improve.len().min(3) + 1
            };
        let steps_height = usize::from(area.height)
            .saturating_sub(header_lines + 2)
            .max(3);
        let per_page = (steps_height / 2).max(1);
        let step_count = run.map_or(0, |run| run.steps.len()) as isize;
        let cursor = step_cursor as isize;
        let steps_top = (cursor - per_page as isize + 1)
            .min(if run.is_some() {
                step_count - per_page as isize
            } else {
                0
            })
            .max(0);
        let first_index = steps_top.min(cursor).max(0) as usize;

        let mut y = inner_area.y;
        let row_area = |y: u16| Rect {
            y,
            height: 1,
            ..inner_area
        };

        // ‹ Logs  ...  n steps  s Seen
        let mut right = vec![span(
            run.map_or("no runs".to_string(), |run| {
                step_count_label(run.steps.len())
            }),
            fg(THEME.muted),
        )];
        if friction_state(log) != ReviewState::Seen && run.is_some() {
            right.push(span("   s Seen", fg(THEME.green)));
        }
        space_between(
            buf,
            row_area(y),
            y,
            &[span("‹ Logs", fg(THEME.blue))],
            &right,
        );
        y += 1;

        let pill: Vec<Span<'static>> =
            status_pill(friction_status_label(log.status.as_deref()).as_deref())
                .into_iter()
                .collect();
        space_between(
            buf,
            row_area(y),
            y,
            &[span(
                truncate(&log.title, sub(inner_area.width, 12)),
                bold(),
            )],
            &pill,
        );
        y += 1;
        put_truncated(
            buf,
            inner_area,
            inner_area.x,
            y,
            &[span(truncate(&log.description, inner), fg(THEME.muted))],
            inner,
        );
        y += 1;
        let mut slug = vec![
            worktree_chip(&log.worktree_short),
            span(format!(" {}", log.slug), fg(THEME.muted)),
        ];
        if log.prompt_path.is_some() {
            slug.push(span(
                format!(
                    "   p {}",
                    if prompt_open { "Hide prompt" } else { "Prompt" }
                ),
                fg(THEME.blue),
            ));
        }
        put_truncated(buf, inner_area, inner_area.x, y, &slug, inner);
        y += 1;

        if let (true, Some(prompt)) = (prompt_open, prompt) {
            let box_area = Rect::new(inner_area.x, y, inner_area.width, 6).intersection(inner_area);
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(fg(THEME.faint))
                .render(box_area, buf);
            if box_area.width > 4 && box_area.height > 2 {
                let content = Rect::new(
                    box_area.x + 2,
                    box_area.y + 1,
                    box_area.width - 4,
                    box_area.height - 2,
                );
                put_spans(
                    buf,
                    content,
                    content.x,
                    content.y,
                    &[section_label("SCENARIO PROMPT")],
                );
                let text = truncate(prompt, sub(inner_area.width, 4) * 4);
                put_wrapped(
                    buf,
                    content,
                    content.x,
                    content.y + 1,
                    usize::from(content.width),
                    &text,
                    fg(THEME.muted),
                );
            }
            y += 6;
        }

        // Run row.
        let mut left = vec![span(
            if log.runs.len() > 1 {
                format!("Runs {}", log.runs.len())
            } else {
                "Run".to_string()
            },
            bold(),
        )];
        if let Some(run) = run {
            left.push(span(
                format!("  {}", run_display_title(&run.run_id)),
                fg(THEME.muted),
            ));
            if run_index == Some(0) {
                left.push(span(" Latest", fg(THEME.purple)));
            }
            left.push(span(format!("  {}", run.run_id), fg(THEME.muted)));
        }
        let switch: Vec<Span<'static>> = if log.runs.len() > 1 {
            vec![span("[ ] switch run", fg(THEME.blue))]
        } else {
            Vec::new()
        };
        space_between(buf, row_area(y), y, &left, &switch);
        y += 1;

        if !improve.is_empty() {
            put_spans(
                buf,
                inner_area,
                inner_area.x,
                y,
                &[span(
                    format!("Improve rollup · {}", improve.len()),
                    bold_fg(THEME.amber),
                )],
            );
            y += 1;
            for (step, note) in improve.iter().take(3) {
                let text = format!(
                    "{} {}",
                    step_number(step),
                    truncate(note, sub(inner_area.width, 3))
                );
                put_truncated(
                    buf,
                    inner_area,
                    inner_area.x,
                    y,
                    &[span(text, fg(THEME.amber))],
                    inner,
                );
                y += 1;
            }
        }

        space_between(
            buf,
            row_area(y),
            y,
            &[section_label("STEPS")],
            &[span("⏎ open step · ↑↓ move", fg(THEME.muted))],
        );
        y += 1;

        let Some(run) = run else {
            put_spans(
                buf,
                inner_area,
                inner_area.x,
                y,
                &[span("No runs yet", bold())],
            );
            put_wrapped(
                buf,
                inner_area,
                inner_area.x,
                y + 1,
                inner,
                "This scenario has a prompt but no log.jsonl run. Use the friction-log skill to execute it; steps will appear here as the agent writes them.",
                fg(THEME.muted),
            );
            return;
        };
        if run.steps.is_empty() {
            put_spans(
                buf,
                inner_area,
                inner_area.x,
                y,
                &[span("No JSONL steps in this run", fg(THEME.muted))],
            );
            return;
        }
        let end = (first_index + per_page).min(run.steps.len());
        for (offset, step) in run.steps[first_index.min(end)..end].iter().enumerate() {
            let row = Rect::new(inner_area.x, y + (offset * 2) as u16, inner_area.width, 2)
                .intersection(inner_area);
            render_step_row(buf, row, step, first_index + offset == step_cursor);
        }
    }
}

fn render_step_row(buf: &mut Buffer, area: Rect, step: &FrictionStep, selected: bool) {
    if area.is_empty() {
        return;
    }
    let width = usize::from(area.width);
    if selected {
        fill_bg(buf, area, THEME.selection);
    }
    let left = [
        span(
            step_number(step),
            fg(if selected { THEME.brand } else { THEME.purple }),
        ),
        span("  ", Style::new()),
        span(truncate(&step.title, width.saturating_sub(16)), bold()),
    ];
    let mut right = Vec::new();
    if !step.transcript.is_empty() {
        right.push(span("✎ ", fg(THEME.purple)));
    }
    if !step.screenshots.is_empty() {
        right.push(span("◫ ", fg(THEME.muted)));
    }
    if !step.good.is_empty() {
        right.push(span(format!("{}+ ", step.good.len()), fg(THEME.green)));
    }
    if !step.improve.is_empty() {
        right.push(span(format!("{}! ", step.improve.len()), fg(THEME.amber)));
    }
    space_between(buf, area, area.y, &left, &right);
    put_truncated(
        buf,
        area,
        area.x,
        area.y + 1,
        &[span(
            format!(
                "    {}",
                truncate(&step.description, width.saturating_sub(4))
            ),
            fg(THEME.muted),
        )],
        width,
    );
}

/// Whether the step views render as a page or the full-screen takeover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    Page,
    Takeover,
}

#[derive(Debug, Clone, Copy)]
pub struct FrictionStepDetailProps<'a> {
    pub log: &'a FrictionLog,
    pub run: &'a FrictionRun,
    pub step_index: usize,
    pub image_index: usize,
    pub mode: StepMode,
}

/// Picture state shared by the step views (one screenshot is on screen).
pub struct FrictionStepState {
    picture: PictureState,
}

impl FrictionStepState {
    pub fn new(services: &AppServices, wake: Wake) -> Self {
        Self {
            picture: PictureState::new(services, wake),
        }
    }
}

/// Draw a titled list of up to four bullets; returns the rows used.
fn render_note_card(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    title: &str,
    color: Color,
    items: &[String],
    empty: &str,
) -> u16 {
    let width = usize::from(area.width);
    let mut head = vec![span(title, bold_fg(color))];
    if !items.is_empty() {
        // Nested in the bold title `<Text>`, so the count is bold too.
        head.push(span(format!(" {}", items.len()), bold_fg(THEME.muted)));
    }
    put_spans(buf, area, area.x, y, &head);
    if items.is_empty() {
        put_spans(buf, area, area.x, y + 1, &[span(empty, fg(THEME.muted))]);
        return 2;
    }
    let shown = items.len().min(4);
    for (index, item) in items.iter().take(4).enumerate() {
        let text = format!("• {}", truncate(item, width.saturating_sub(2)));
        put_truncated(
            buf,
            area,
            area.x,
            y + 1 + index as u16,
            &[span(text, Style::new())],
            width,
        );
    }
    1 + shown as u16
}

fn screenshot_of(step: &FrictionStep, image_index: usize) -> Option<&str> {
    if step.screenshots.is_empty() {
        return None;
    }
    let index = image_index.min(step.screenshots.len() - 1);
    Some(&step.screenshots[index])
}

fn step_picture(src: Option<&str>) -> PictureProps<'_> {
    PictureProps {
        src,
        max_upscale: 8.0,
        label: Some("No screenshot for this step"),
        ..PictureProps::default()
    }
}

/// Step detail: preview, notes, and metadata.
pub struct FrictionStepDetail<'a> {
    pub props: FrictionStepDetailProps<'a>,
}

impl<'a> FrictionStepDetail<'a> {
    pub fn new(props: FrictionStepDetailProps<'a>) -> Self {
        Self { props }
    }
}

impl StatefulWidget for FrictionStepDetail<'_> {
    type State = FrictionStepState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut FrictionStepState) {
        let FrictionStepDetailProps {
            log,
            run,
            step_index,
            image_index,
            ..
        } = self.props;
        let step = &run.steps[step_index];
        if area.width < 2 {
            return;
        }
        let content = Rect::new(area.x + 1, area.y, area.width - 2, area.height);
        let inner = usize::from(content.width);
        let preview = ((f64::from(area.height) * 0.38 + 0.5).floor() as u16).clamp(6, 20);
        let screenshot = screenshot_of(step, image_index);
        let row_area = |y: u16| Rect {
            y,
            height: 1,
            ..content
        };
        let mut y = content.y;

        space_between(
            buf,
            row_area(y),
            y,
            &[span("‹ Steps", fg(THEME.blue))],
            &[span(
                format!("{} / {}   ← → step", step_index + 1, run.steps.len()),
                fg(THEME.muted),
            )],
        );
        y += 1;

        let picture_area = Rect::new(content.x, y, content.width, preview).intersection(content);
        if !picture_area.is_empty() {
            Picture::new(step_picture(screenshot)).render(picture_area, buf, &mut state.picture);
        }
        y += preview;

        if step.screenshots.len() > 1 {
            let dots: String = (0..step.screenshots.len())
                .map(|index| if index == image_index { "● " } else { "○ " })
                .collect();
            let text = format!(
                "{dots}{} of {} · [ ] image",
                image_index + 1,
                step.screenshots.len()
            );
            put_spans(buf, content, content.x, y, &[span(text, fg(THEME.muted))]);
            y += 1;
        }
        put_truncated(
            buf,
            content,
            content.x,
            y,
            &[
                span(step_number(step), bold_fg(THEME.purple)),
                span("  ", Style::new()),
                span(truncate(&step.title, sub(content.width, 4)), bold()),
            ],
            inner,
        );
        y += 1;
        if !step.description.is_empty() {
            put_truncated(
                buf,
                content,
                content.x,
                y,
                &[span(truncate(&step.description, inner), fg(THEME.muted))],
                inner,
            );
            y += 1;
        }
        if let Some(url) = step.url.as_deref().filter(|url| !url.is_empty()) {
            put_spans(buf, content, content.x, y, &[span(url, fg(THEME.blue))]);
            y += 1;
        }

        // Remaining column: clipped at the bottom of the area.
        let rest = Rect::new(
            content.x,
            y.min(area.bottom()),
            content.width,
            area.bottom().saturating_sub(y),
        );
        if rest.is_empty() {
            return;
        }
        let mut y = rest.y;
        if !step.transcript.is_empty() {
            put_spans(
                buf,
                rest,
                rest.x,
                y,
                &[span("Transcript", bold_fg(THEME.purple))],
            );
            y += 1;
            y += put_wrapped(
                buf,
                rest,
                rest.x,
                y,
                inner,
                &truncate(&step.transcript, inner * 3),
                Style::new(),
            );
        }
        y += render_note_card(
            buf,
            rest,
            y,
            "Looks good",
            THEME.green,
            &step.good,
            "No positives noted for this step",
        );
        y += render_note_card(
            buf,
            rest,
            y,
            "Can improve",
            THEME.amber,
            &step.improve,
            "No friction found for this step",
        );
        put_spans(buf, rest, rest.x, y, &[rule(inner, None)]);
        y += 1;
        let file = screenshot.map_or("—".to_string(), |path| basename(path).to_string());
        for (label, value) in [
            ("Log", log.slug.as_str()),
            ("Run", run.run_id.as_str()),
            ("Tree", log.worktree.as_str()),
            ("File", file.as_str()),
        ] {
            let meta = Rect::new(rest.x, y, rest.width, 1).intersection(rest);
            if !meta.is_empty() {
                MetaRow { label, value }.render(meta, buf);
            }
            y += 1;
        }
    }
}

/// Full-screen step view: screenshot stage on the left, notes rail on the right.
pub struct FrictionStepTakeover<'a> {
    pub props: FrictionStepDetailProps<'a>,
}

impl<'a> FrictionStepTakeover<'a> {
    pub fn new(props: FrictionStepDetailProps<'a>) -> Self {
        Self { props }
    }
}

impl StatefulWidget for FrictionStepTakeover<'_> {
    type State = FrictionStepState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut FrictionStepState) {
        let FrictionStepDetailProps {
            log,
            run,
            step_index,
            image_index,
            ..
        } = self.props;
        let step = &run.steps[step_index];
        let width = usize::from(area.width);
        let rail_width = if width >= 100 {
            42
        } else {
            28.max(width * 38 / 100)
        };
        let stage_width = 10.max(width.saturating_sub(rail_width + 1));
        let stage_height = 4.max(usize::from(area.height).saturating_sub(3));
        let screenshot = screenshot_of(step, image_index);
        let meta = [
            log.title.as_str(),
            log.worktree_short.as_str(),
            run.run_id.as_str(),
            step.url.as_deref().unwrap_or(""),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let rail_inner = rail_width.saturating_sub(2);
        if area.width < 2 {
            return;
        }
        let padded = Rect::new(area.x + 1, area.y, area.width - 2, area.height.min(3));

        let number = format!(" {} ", step_number(step));
        space_between(
            buf,
            padded,
            area.y,
            &[
                span(number, fg(THEME.purple).bg(THEME.selection)),
                span("  ", Style::new()),
                span(truncate(&step.title, width.saturating_sub(40)), bold()),
            ],
            &[
                span("‹", fg(THEME.blue)),
                span(
                    format!(" {} / {} ", step_index + 1, run.steps.len()),
                    fg(THEME.muted),
                ),
                span("›", fg(THEME.blue)),
                span("   esc close", fg(THEME.muted)),
            ],
        );
        put_truncated(
            buf,
            padded,
            padded.x,
            area.y + 1,
            &[span(
                truncate(&meta, width.saturating_sub(2)),
                fg(THEME.muted),
            )],
            usize::from(padded.width),
        );
        put_spans(
            buf,
            padded,
            padded.x,
            area.y + 2,
            &[rule(width.saturating_sub(2), None)],
        );

        let stage =
            Rect::new(area.x, area.y + 3, area.width, stage_height as u16).intersection(area);
        if stage.is_empty() {
            return;
        }
        let stage_area =
            Rect::new(stage.x, stage.y, stage_width as u16, stage.height).intersection(stage);
        let multiple = step.screenshots.len() > 1;
        let picture_height = if multiple {
            stage_height - 1
        } else {
            stage_height
        };
        let picture_area = Rect::new(
            stage_area.x,
            stage_area.y,
            stage_area.width,
            picture_height as u16,
        )
        .intersection(stage_area);
        if !picture_area.is_empty() {
            Picture::new(step_picture(screenshot)).render(picture_area, buf, &mut state.picture);
        }
        if multiple {
            let caption = format!(
                "Image {} / {} · [ ] switch",
                image_index + 1,
                step.screenshots.len()
            );
            let caption_width = super::text_width(&caption);
            let x = stage_area.x + (stage_width.saturating_sub(caption_width) / 2) as u16;
            put_spans(
                buf,
                stage_area,
                x,
                stage_area.y + picture_height as u16,
                &[span(caption, fg(THEME.muted))],
            );
        }

        let divider =
            Rect::new(stage.x + stage_width as u16, stage.y, 1, stage.height).intersection(stage);
        for row in 0..divider.height {
            put_spans(
                buf,
                divider,
                divider.x,
                divider.y + row,
                &[span("│", fg(THEME.faint))],
            );
        }

        let rail = Rect::new(
            stage.x + stage_width as u16 + 1,
            stage.y,
            rail_width as u16,
            stage.height,
        )
        .intersection(stage);
        if rail.width < 2 {
            return;
        }
        let rail = Rect::new(rail.x + 1, rail.y, rail.width - 2, rail.height);
        let rail_text = usize::from(rail.width).min(rail_inner);
        let mut y = rail.y;
        put_spans(buf, rail, rail.x, y, &[span("Step notes", bold())]);
        y += 1;
        let description = if step.description.is_empty() {
            "—"
        } else {
            &step.description
        };
        y += put_wrapped(
            buf,
            rail,
            rail.x,
            y,
            rail_text,
            &truncate(description, rail_inner * 3),
            fg(THEME.muted),
        );
        if let Some(url) = step.url.as_deref().filter(|url| !url.is_empty()) {
            put_spans(buf, rail, rail.x, y, &[span(url, fg(THEME.blue))]);
            y += 1;
        }
        put_spans(buf, rail, rail.x, y, &[rule(rail_inner, None)]);
        y += 1;
        put_spans(
            buf,
            rail,
            rail.x,
            y,
            &[span("Transcript", bold_fg(THEME.purple))],
        );
        y += 1;
        let transcript = if step.transcript.is_empty() {
            "—"
        } else {
            &step.transcript
        };
        y += put_wrapped(
            buf,
            rail,
            rail.x,
            y,
            rail_text,
            &truncate(transcript, rail_inner * 5),
            Style::new(),
        );
        put_spans(buf, rail, rail.x, y, &[rule(rail_inner, None)]);
        y += 1;
        y += render_note_card(
            buf,
            rail,
            y,
            "Looks good",
            THEME.green,
            &step.good,
            "No positives noted",
        );
        render_note_card(
            buf,
            rail,
            y,
            "Can improve",
            THEME.amber,
            &step.improve,
            "No friction found",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::data::model::ReviewSnapshot;
    use crate::astroshot_review::terminal::probe::GraphicsProtocol;
    use crate::astroshot_review::ui::context::test_support::{FakeService, services};
    use crate::astroshot_review::ui::testing::{
        bg_at, fg_at, modifier_at, render, render_stateful, rows,
    };
    use ratatui::style::Color;
    use std::sync::Arc;

    fn step(n: f64, title: &str, f: impl FnOnce(&mut FrictionStep)) -> FrictionStep {
        let mut step = FrictionStep {
            id: format!("s{n}"),
            step: n,
            step_id: format!("step-{n}"),
            title: title.into(),
            description: format!("Description of {title}"),
            transcript: String::new(),
            screenshots: vec![],
            good: vec![],
            improve: vec![],
            url: None,
            captured_at: None,
        };
        f(&mut step);
        step
    }

    fn run(run_id: &str, steps: Vec<FrictionStep>) -> FrictionRun {
        FrictionRun {
            run_id: run_id.into(),
            directory: format!("/w/.astroshot/friction-logs/signup/runs/{run_id}"),
            log_path: None,
            captured_at: 1_000_000.0,
            status: None,
            steps,
            review: None,
        }
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

    fn make_log(slug: &str, runs: Vec<FrictionRun>) -> FrictionLog {
        FrictionLog {
            id: format!("/w::{slug}"),
            slug: slug.into(),
            directory: format!("/w/.astroshot/friction-logs/{slug}"),
            worktree_path: "/w".into(),
            worktree: "/w".into(),
            worktree_short: "w".into(),
            title: format!("Title {slug}"),
            description: format!("About {slug}"),
            status: Some("complete".into()),
            updated_at: 0.0,
            prompt_path: Some(format!("/w/.astroshot/friction-logs/{slug}/prompt.md")),
            runs,
        }
    }

    fn fixture() -> FrictionLog {
        let steps = vec![
            step(1.0, "Open the page", |s| {
                s.good = vec!["Fast load".into()];
                s.screenshots = vec!["/shots/a.png".into(), "/shots/b.png".into()];
                s.transcript = "Went to the page and looked around for a sign up button".into();
                s.url = Some("https://example.com/signup".into());
            }),
            step(2.0, "Fill the form", |s| {
                s.improve = vec!["Label unclear".into(), "Error text is red on red".into()];
            }),
            step(3.0, "Submit", |_| {}),
        ];
        make_log("signup", vec![run("run-b", steps), run("run-a", vec![])])
    }

    fn list_props<'a>(logs: &'a [&'a FrictionLog]) -> FrictionListProps<'a> {
        FrictionListProps {
            logs,
            cursor: 0,
            scroll_top: 0,
            filter: StreamFilter::Unseen,
            pending: 2,
            seen: 0,
            focused: true,
            total: 2,
            now: 1_090_000.0,
        }
    }

    fn step_state() -> FrictionStepState {
        let service = FakeService::new(false);
        FrictionStepState::new(&services(GraphicsProtocol::None, service), Arc::new(|| {}))
    }

    #[test]
    fn friction_scroll_keeps_the_cursor_page_visible() {
        // 8 lines hold two 4-line rows.
        assert_eq!(friction_scroll(5, 3, 0, 8), 2);
        assert_eq!(friction_scroll(5, 0, 3, 8), 0);
        assert_eq!(friction_scroll(5, 2, 2, 8), 2);
        // The scroll position is clamped to the last row, and a short pane still shows one row.
        assert_eq!(friction_scroll(5, 4, 99, 1), 4);
        assert_eq!(friction_scroll(0, 0, 0, 8), 0);
    }

    #[test]
    fn friction_filter_bar_shows_titles_and_hints() {
        let bar = |filter, pending, seen| {
            render(
                FrictionFilterBar {
                    filter,
                    pending,
                    seen,
                },
                40,
                1,
            )
        };
        let buf = bar(StreamFilter::Unseen, 2, 5);
        assert_eq!(rows(&buf), [" Unseen (2)      S Seen all  u History"]);
        assert_eq!(modifier_at(&buf, 1, 0), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 26, 0), THEME.green);
        assert_eq!(fg_at(&buf, 38, 0), THEME.blue);
        assert_eq!(
            rows(&bar(StreamFilter::Unseen, 0, 5)),
            [" Unseen (0)                  u History"]
        );
        assert_eq!(
            rows(&bar(StreamFilter::History, 2, 5)),
            [" History (5)                  u Unseen"]
        );
    }

    #[test]
    fn friction_list_marks_seen_logs_and_hides_status_without_label() {
        let mut seen_log = make_log("done", vec![run("run-x", vec![step(1.0, "S", |_| {})])]);
        seen_log.runs[0].review = Some(seen());
        seen_log.status = None;
        let logs = [&seen_log];
        let mut p = list_props(&logs);
        p.focused = false;
        let buf = render(FrictionList::new(p), 50, 4);
        assert_eq!(
            rows(&buf),
            [
                "  Title done",
                "  About done",
                "   w  done                                 1 step",
                "  run-x                          Seen · 2 min ago",
            ]
        );
        assert_eq!(bg_at(&buf, 0, 0), Color::Reset);
        // The unselected marker is a brand-colored space.
        assert_eq!(fg_at(&buf, 0, 0), THEME.brand);
        assert_eq!(fg_at(&buf, 33, 3), THEME.blue); // Seen
        assert_eq!(fg_at(&buf, 38, 3), THEME.muted); // relative time
    }

    #[test]
    fn friction_list_empty_filters_show_caught_up_and_history_copy() {
        let mut p = list_props(&[]);
        let caught_up: Vec<String> = rows(&render(FrictionList::new(p), 40, 7))
            .into_iter()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(
            caught_up,
            [
                "You’re all caught up",
                "Every friction log has been seen.",
                "u View history"
            ]
        );
        p.filter = StreamFilter::History;
        let history: Vec<String> = rows(&render(FrictionList::new(p), 40, 7))
            .into_iter()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(
            history,
            [
                "No history yet",
                "Logs you mark Seen will appear here.",
                "u Back to unseen"
            ]
        );
    }

    #[test]
    fn friction_list_truncates_long_titles_and_descriptions() {
        let mut long = make_log("long", vec![]);
        long.title = "T".repeat(80);
        long.description = "d".repeat(80);
        // inner = 37 columns; the title is cut to max(8, 37 - 12) = 25 and the
        // status pill ends at column 39.
        let logs = [&long];
        let buf = render(FrictionList::new(list_props(&logs)), 40, 4);
        assert_eq!(rows(&buf)[0], format!("▎ {}…    Complete", "T".repeat(24)));
        assert_eq!(rows(&buf)[1], format!("  {}…", "d".repeat(36)));
        long.status = Some("draft".into());
        let logs = [&long];
        let buf = render(FrictionList::new(list_props(&logs)), 40, 4);
        assert_eq!(rows(&buf)[0], format!("▎ {}…       Draft", "T".repeat(24)));
    }

    #[tokio::test]
    async fn friction_list_renders_rows_selection_and_summary_lines() {
        let log = fixture();
        let other = log_named("second");
        let logs = [&log, &other];
        let buf = render(FrictionList::new(list_props(&logs)), 50, 9);
        let expected = [
            "▎ Title signup                           Complete",
            "  About signup",
            "   w  signup                  3 steps · 2 improve",
            "  2 runs · run-b               Unseen · 2 min ago",
            "  Title second                              Draft",
            "  About second",
            "   w  second",
            "  Prompt only · no runs yet                Unseen",
            "",
        ];
        assert_eq!(rows(&buf), expected);
        // The selected row: marker bar in the brand color, selection background across the row.
        assert_eq!(fg_at(&buf, 0, 0), THEME.brand);
        for y in 0..4 {
            assert_eq!(bg_at(&buf, 0, y), THEME.selection);
            assert_eq!(bg_at(&buf, 49, y), THEME.selection);
        }
        assert_eq!(bg_at(&buf, 0, 4), Color::Reset);
        assert_eq!(modifier_at(&buf, 2, 0), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 42, 0), THEME.green); // Complete
        assert_eq!(fg_at(&buf, 2, 1), THEME.muted);
        assert_eq!(fg_at(&buf, 40, 2), THEME.muted); // summary
        assert_eq!(fg_at(&buf, 31, 3), THEME.amber); // Unseen
        assert_eq!(fg_at(&buf, 38, 3), THEME.muted); // relative time
        assert_eq!(fg_at(&buf, 44, 4), THEME.muted); // Draft
        assert_eq!(fg_at(&buf, 43, 7), THEME.amber); // Unseen, no run so no time
        assert_eq!(fg_at(&buf, 2, 7), THEME.muted); // footer
    }

    #[tokio::test]
    async fn friction_list_pages_by_four_line_rows() {
        let log = fixture();
        let other = log_named("second");
        let logs = [&log, &other];
        let mut p = list_props(&logs);
        p.cursor = 1;
        p.scroll_top = 1;
        let buf = render(FrictionList::new(p), 50, 5);
        let expected = [
            "▎ Title second                              Draft",
            "  About second",
            "   w  second",
            "  Prompt only · no runs yet                Unseen",
            "",
        ];
        assert_eq!(rows(&buf), expected);
        assert_eq!(bg_at(&buf, 0, 0), THEME.selection);
    }

    #[tokio::test]
    async fn friction_list_without_logs_shows_empty_state_and_layout_hints() {
        let mut p = list_props(&[]);
        p.total = 0;
        let buf = render(FrictionList::new(p), 60, 12);
        let expected = [
            "",
            "",
            "",
            "                    No friction logs yet",
            "",
            "  Author a scenario with the friction-log skill, then run",
            "  it. Results land under .astroshot/friction-logs/.",
            "",
            "",
            "",
            "  .astroshot/friction-logs/<slug>/prompt.md",
            "  runs/<run-id>/log.jsonl + screenshots",
        ];
        assert_eq!(rows(&buf), expected);
        assert_eq!(fg_at(&buf, 2, 10), THEME.muted);
    }

    #[tokio::test]
    async fn log_detail_renders_header_prompt_run_picker_rollup_and_steps() {
        let log = fixture();
        let d = FrictionLogDetailProps {
            log: &log,
            run: Some(&log.runs[0]),
            step_cursor: 1,
            prompt_open: true,
            prompt: Some("Sign up as a new user and report friction"),
        };
        let buf = render(FrictionLogDetail::new(d), 60, 22);
        let expected = [
            " ‹ Logs                                    3 steps   s Seen",
            " Title signup                                      Complete",
            " About signup",
            "  w  signup   p Hide prompt",
            " ╭────────────────────────────────────────────────────────╮",
            " │ SCENARIO PROMPT                                        │",
            " │ Sign up as a new user and report friction              │",
            " │                                                        │",
            " │                                                        │",
            " ╰────────────────────────────────────────────────────────╯",
            " Runs 2  run-b Latest  run-b                 [ ] switch run",
            " Improve rollup · 2",
            " 02 Label unclear",
            " 02 Error text is red on red",
            " STEPS                                ⏎ open step · ↑↓ move",
            " 01  Open the page                                  ✎ ◫ 1+",
            "     Description of Open the page",
            " 02  Fill the form                                      2!",
            "     Description of Fill the form",
            "",
            "",
            "",
        ];
        assert_eq!(rows(&buf), expected);
        assert_eq!(fg_at(&buf, 1, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 43, 0), THEME.muted); // 3 steps: columns 43..=49
        assert_eq!(fg_at(&buf, 49, 0), THEME.muted);
        assert_eq!(fg_at(&buf, 53, 0), THEME.green); // s Seen: columns 50..=58
        assert_eq!(fg_at(&buf, 4, 4), THEME.faint); // prompt border
        assert_eq!(fg_at(&buf, 14, 10), THEME.purple); // Latest
        assert_eq!(fg_at(&buf, 46, 10), THEME.blue);
        assert_eq!(modifier_at(&buf, 1, 11), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 1, 12), THEME.amber);
        // Selected step (cursor 1): two rows with the selection background and brand number.
        assert_eq!(bg_at(&buf, 1, 17), THEME.selection);
        assert_eq!(bg_at(&buf, 58, 18), THEME.selection);
        assert_eq!(fg_at(&buf, 1, 17), THEME.brand);
        assert_eq!(bg_at(&buf, 1, 15), Color::Reset);
        assert_eq!(fg_at(&buf, 1, 15), THEME.purple);
        assert_eq!(fg_at(&buf, 56, 17), THEME.amber); // 2!
        assert_eq!(fg_at(&buf, 52, 15), THEME.purple); // transcript mark
        assert_eq!(fg_at(&buf, 54, 15), THEME.muted); // screenshot mark
        assert_eq!(fg_at(&buf, 56, 15), THEME.green); // 1+
    }

    #[tokio::test]
    async fn log_detail_without_a_run_explains_how_to_get_one() {
        let log = fixture();
        let d = FrictionLogDetailProps {
            log: &log,
            run: None,
            step_cursor: 0,
            prompt_open: false,
            prompt: None,
        };
        let buf = render(FrictionLogDetail::new(d), 60, 12);
        let expected = [
            " ‹ Logs                                             no runs",
            " Title signup                                      Complete",
            " About signup",
            "  w  signup   p Prompt",
            " Runs 2                                      [ ] switch run",
            " STEPS                                ⏎ open step · ↑↓ move",
            " No runs yet",
            " This scenario has a prompt but no log.jsonl run. Use the",
            " friction-log skill to execute it; steps will appear here",
            " as the agent writes them.",
            "",
            "",
        ];
        assert_eq!(rows(&buf), expected);
    }

    #[tokio::test]
    async fn log_detail_with_an_empty_run_says_there_are_no_steps() {
        let log = fixture();
        let d = FrictionLogDetailProps {
            log: &log,
            run: Some(&log.runs[1]),
            step_cursor: 0,
            prompt_open: false,
            prompt: None,
        };
        let buf = render(FrictionLogDetail::new(d), 60, 12);
        let expected = [
            " ‹ Logs                                    0 steps   s Seen",
            " Title signup                                      Complete",
            " About signup",
            "  w  signup   p Prompt",
            " Runs 2  run-a  run-a                        [ ] switch run",
            " STEPS                                ⏎ open step · ↑↓ move",
            " No JSONL steps in this run",
            "",
            "",
            "",
            "",
            "",
        ];
        assert_eq!(rows(&buf), expected);
    }

    #[tokio::test]
    async fn log_detail_windows_steps_around_the_cursor() {
        let steps: Vec<_> = (1..=5)
            .map(|n| step(f64::from(n), &format!("S{n}"), |_| {}))
            .collect();
        let many = make_log("many", vec![run("only", steps)]);
        let d = FrictionLogDetailProps {
            log: &many,
            run: Some(&many.runs[0]),
            step_cursor: 3,
            prompt_open: false,
            prompt: None,
        };
        let buf = render(FrictionLogDetail::new(d), 60, 11);
        let expected = [
            " ‹ Logs                                    5 steps   s Seen",
            " Title many                                        Complete",
            " About many",
            "  w  many   p Prompt",
            " Run  only Latest  only",
            " STEPS                                ⏎ open step · ↑↓ move",
            " 04  S4",
            "     Description of S4",
            "",
            "",
            "",
        ];
        assert_eq!(rows(&buf), expected);
        // The one visible step is the selected one.
        assert_eq!(bg_at(&buf, 1, 6), THEME.selection);
    }

    #[tokio::test]
    async fn step_detail_renders_preview_notes_and_metadata() {
        let log = fixture();
        let props = FrictionStepDetailProps {
            log: &log,
            run: &log.runs[0],
            step_index: 0,
            image_index: 1,
            mode: StepMode::Page,
        };
        let buf = render_stateful(FrictionStepDetail::new(props), &mut step_state(), 60, 30);
        let expected = [
            " ‹ Steps                                   1 / 3   ← → step",
            "",
            "",
            "",
            "",
            "",
            "                No screenshot for this step",
            "",
            "",
            "",
            "",
            "",
            " ○ ● 2 of 2 · [ ] image",
            " 01  Open the page",
            " Description of Open the page",
            " https://example.com/signup",
            " Transcript",
            " Went to the page and looked around for a sign up button",
            " Looks good 1",
            " • Fast load",
            " Can improve",
            " No friction found for this step",
            " ──────────────────────────────────────────────────────────",
            " Log       signup",
            " Run       run-b",
            " Tree      /w",
            " File      b.png",
            "",
            "",
            "",
        ];
        assert_eq!(rows(&buf), expected);
        assert_eq!(fg_at(&buf, 1, 0), THEME.blue);
        assert_eq!(fg_at(&buf, 20, 6), THEME.muted); // picture placeholder
        assert_eq!(fg_at(&buf, 1, 13), THEME.purple);
        assert_eq!(modifier_at(&buf, 1, 13), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 1, 15), THEME.blue); // url
        assert_eq!(fg_at(&buf, 1, 16), THEME.purple); // Transcript
        assert_eq!(fg_at(&buf, 1, 18), THEME.green);
        // The count sits inside the bold title text: muted and bold.
        assert_eq!(fg_at(&buf, 12, 18), THEME.muted);
        assert_eq!(modifier_at(&buf, 12, 18), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 1, 20), THEME.amber);
        assert_eq!(fg_at(&buf, 1, 22), THEME.faint);
        assert_eq!(fg_at(&buf, 1, 23), THEME.muted);
    }

    #[tokio::test]
    async fn step_detail_without_notes_or_screenshots_uses_empty_copy() {
        let log = fixture();
        let props = FrictionStepDetailProps {
            log: &log,
            run: &log.runs[0],
            step_index: 2,
            image_index: 0,
            mode: StepMode::Page,
        };
        let buf = render_stateful(FrictionStepDetail::new(props), &mut step_state(), 40, 20);
        let expected = [
            " ‹ Steps               3 / 3   ← → step",
            "",
            "",
            "",
            "                No image",
            "",
            "",
            "",
            "",
            " 03  Submit",
            " Description of Submit",
            " Looks good",
            " No positives noted for this step",
            " Can improve",
            " No friction found for this step",
            " ──────────────────────────────────────",
            " Log       signup",
            " Run       run-b",
            " Tree      /w",
            " File      —",
        ];
        assert_eq!(rows(&buf), expected);
    }

    /// Ink shrinks and overlaps rows when the step does not fit; the port
    /// keeps every row intact and drops the ones past the bottom edge.
    #[tokio::test]
    async fn step_detail_clips_rows_that_do_not_fit() {
        let log = fixture();
        let props = FrictionStepDetailProps {
            log: &log,
            run: &log.runs[0],
            step_index: 2,
            image_index: 0,
            mode: StepMode::Page,
        };
        let buf = render_stateful(FrictionStepDetail::new(props), &mut step_state(), 40, 16);
        let expected = [
            " ‹ Steps               3 / 3   ← → step",
            "",
            "",
            "                No image",
            "",
            "",
            "",
            " 03  Submit",
            " Description of Submit",
            " Looks good",
            " No positives noted for this step",
            " Can improve",
            " No friction found for this step",
            " ──────────────────────────────────────",
            " Log       signup",
            " Run       run-b",
        ];
        assert_eq!(rows(&buf), expected);
    }

    #[tokio::test]
    async fn step_takeover_renders_stage_divider_and_notes_rail() {
        let log = fixture();
        let props = FrictionStepDetailProps {
            log: &log,
            run: &log.runs[0],
            step_index: 0,
            image_index: 1,
            mode: StepMode::Takeover,
        };
        let buf = render_stateful(FrictionStepTakeover::new(props), &mut step_state(), 100, 20);
        let expected = [
            "  01   Open the page                                                          ‹ 1 / 3 ›   esc close",
            " Title signup · w · run-b · https://example.com/signup",
            " ──────────────────────────────────────────────────────────────────────────────────────────────────",
            "                                                         │ Step notes",
            "                                                         │ Description of Open the page",
            "                                                         │ https://example.com/signup",
            "                                                         │ ────────────────────────────────────────",
            "                                                         │ Transcript",
            "                                                         │ Went to the page and looked around for a",
            "                                                         │  sign up button",
            "               No screenshot for this step               │ ────────────────────────────────────────",
            "                                                         │ Looks good 1",
            "                                                         │ • Fast load",
            "                                                         │ Can improve",
            "                                                         │ No friction found",
            "                                                         │",
            "                                                         │",
            "                                                         │",
            "                                                         │",
            "                Image 2 / 2 · [ ] switch                 │",
        ];
        assert_eq!(rows(&buf), expected);
        assert_eq!(bg_at(&buf, 1, 0), THEME.selection);
        assert_eq!(fg_at(&buf, 2, 0), THEME.purple);
        assert_eq!(fg_at(&buf, 78, 0), THEME.blue); // ‹
        assert_eq!(fg_at(&buf, 82, 0), THEME.muted); // 1 / 3
        assert_eq!(fg_at(&buf, 86, 0), THEME.blue); // ›
        assert_eq!(fg_at(&buf, 90, 0), THEME.muted); // esc close
        assert_eq!(fg_at(&buf, 1, 1), THEME.muted);
        assert_eq!(fg_at(&buf, 1, 2), THEME.faint);
        assert_eq!(fg_at(&buf, 57, 3), THEME.faint); // divider
        assert_eq!(fg_at(&buf, 57, 19), THEME.faint);
        assert_eq!(modifier_at(&buf, 59, 3), Modifier::BOLD);
        assert_eq!(fg_at(&buf, 59, 5), THEME.blue);
    }

    #[tokio::test]
    async fn step_takeover_uses_a_proportional_rail_on_narrow_terminals() {
        let log = fixture();
        let props = FrictionStepDetailProps {
            log: &log,
            run: &log.runs[0],
            step_index: 2,
            image_index: 0,
            mode: StepMode::Takeover,
        };
        // rail = max(28, floor(80 * 0.38)) = 30, stage = 80 - 30 - 1 = 49.
        let buf = render_stateful(FrictionStepTakeover::new(props), &mut step_state(), 80, 14);
        let expected = [
            "  03   Submit                                             ‹ 3 / 3 ›   esc close",
            " Title signup · w · run-b",
            " ──────────────────────────────────────────────────────────────────────────────",
            "                                                 │ Step notes",
            "                                                 │ Description of Submit",
            "                                                 │ ────────────────────────────",
            "                                                 │ Transcript",
            "                                                 │ —",
            "                    No image                     │ ────────────────────────────",
            "                                                 │ Looks good",
            "                                                 │ No positives noted",
            "                                                 │ Can improve",
            "                                                 │ No friction found",
            "                                                 │",
        ];
        assert_eq!(rows(&buf), expected);
    }

    #[tokio::test]
    async fn step_takeover_lists_four_notes_per_card_and_cuts_long_ones() {
        let notes = make_log(
            "notes",
            vec![run(
                "r",
                vec![step(7.0, "Many notes", |s| {
                    s.description = String::new();
                    s.good = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
                    s.improve = vec![
                        "An improvement note that is far too long to fit in the narrow rail at all"
                            .into(),
                    ];
                })],
            )],
        );
        let props = FrictionStepDetailProps {
            log: &notes,
            run: &notes.runs[0],
            step_index: 0,
            image_index: 0,
            mode: StepMode::Takeover,
        };
        let buf = render_stateful(FrictionStepTakeover::new(props), &mut step_state(), 80, 18);
        let expected = [
            "  07   Many notes                                         ‹ 1 / 1 ›   esc close",
            " Title notes · w · r",
            " ──────────────────────────────────────────────────────────────────────────────",
            "                                                 │ Step notes",
            "                                                 │ —",
            "                                                 │ ────────────────────────────",
            "                                                 │ Transcript",
            "                                                 │ —",
            "                                                 │ ────────────────────────────",
            "                                                 │ Looks good 5",
            "                    No image                     │ • a",
            "                                                 │ • b",
            "                                                 │ • c",
            "                                                 │ • d",
            "                                                 │ Can improve 1",
            "                                                 │ • An improvement note that …",
            "                                                 │",
            "                                                 │",
        ];
        assert_eq!(rows(&buf), expected);
    }

    /// Ink shrinks and overlaps the rail rows when they do not fit; the port
    /// drops the rows past the bottom edge.
    #[tokio::test]
    async fn step_takeover_clips_rail_rows_that_do_not_fit() {
        let log = fixture();
        let props = FrictionStepDetailProps {
            log: &log,
            run: &log.runs[0],
            step_index: 2,
            image_index: 0,
            mode: StepMode::Takeover,
        };
        let buf = render_stateful(FrictionStepTakeover::new(props), &mut step_state(), 80, 10);
        let expected = [
            "  03   Submit                                             ‹ 3 / 3 ›   esc close",
            " Title signup · w · run-b",
            " ──────────────────────────────────────────────────────────────────────────────",
            "                                                 │ Step notes",
            "                                                 │ Description of Submit",
            "                                                 │ ────────────────────────────",
            "                    No image                     │ Transcript",
            "                                                 │ —",
            "                                                 │ ────────────────────────────",
            "                                                 │ Looks good",
        ];
        assert_eq!(rows(&buf), expected);
    }

    fn log_named(slug: &str) -> FrictionLog {
        let mut other = make_log(slug, vec![]);
        other.status = Some("draft".into());
        other
    }
}
