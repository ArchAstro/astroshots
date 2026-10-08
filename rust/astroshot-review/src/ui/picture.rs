//! Port of `packages/astroshot-review/src/ui/picture.tsx`.
//!
//! Shows an image in a reserved cell box. With the Kitty protocol the graphics
//! layer paints real pixels; otherwise (mosh, tmux, herdr, or a plain terminal)
//! it renders the picture as truecolor half-block text, which survives any
//! transport. Falls back to a labeled placeholder when nothing can be drawn.
//!
//! # Mapping
//!
//! - `<Picture>` is the [`Picture`] `StatefulWidget`; its Ink `width`/`height`
//!   props are the `Rect` it renders into.
//! - The component's refs and state (`handleRef`, `failure`, `lines`, the
//!   effects) live in [`PictureState`]. One state per on-screen picture; the
//!   parent keeps it keyed like the React `key` and drops it when the picture
//!   unmounts (drop = effect cleanup: the layer handle unregisters and the
//!   tasks abort).
//! - Effects run inside `render` through [`PictureState::sync`], diffed against
//!   the last applied values, so they fire exactly when a React dependency
//!   array would have changed. They need a tokio runtime (decode and failure
//!   polling are tasks); `wake` is called when a task changed what the next
//!   frame should show.
//! - The `ref` + `measureElement` of the TS becomes the [`CellBox`] the widget
//!   rendered into: [`PictureState::cell_box`] reports it and, in graphics
//!   modes, the widget hands it to `ImageHandle::set_node` so the layer can
//!   place the kitty image.
//!
//! Divergence: with `border`, the TS reserves the outer box (border included)
//! for the layer; that is kept (`cell_box` is the outer area).

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, StatefulWidget, Widget};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

use crate::images::halfblocks::{CellArtOptions, rgb_to_half_block_lines};
use crate::images::png::ImageSize;
use crate::images::scale::ScaledFormat;
use crate::images::service::ImageService;
use crate::terminal::image_layer::{CellBox, ImageHandle, ImageLayer, ImageView};
use crate::terminal::probe::GraphicsProtocol;
use crate::ui::context::AppServices;
use crate::ui::hooks::Wake;
use crate::ui::theme::{THEME, truncate};
use crate::ui::{put_spans, text_width};

#[derive(Debug, Clone)]
pub struct PictureProps<'a> {
    pub src: Option<&'a str>,
    /// Changes when the file's bytes change (mtime), so a re-capture refreshes.
    pub version: f64,
    /// Text shown when the terminal cannot draw pictures.
    pub label: Option<&'a str>,
    pub border: bool,
    pub z: Option<i32>,
    /// Allow the image to scale up to this multiple of native to fill its box (default 1 = never upscale).
    pub max_upscale: f64,
    /// Magnification: 1 shows the whole image; >1 crops in.
    pub zoom: f64,
    /// Pan center as a fraction of the image, [0,1].
    pub pan_x: f64,
    pub pan_y: f64,
}

impl Default for PictureProps<'_> {
    fn default() -> Self {
        Self {
            src: None,
            version: 0.0,
            label: None,
            border: false,
            z: None,
            max_upscale: 1.0,
            zoom: 1.0,
            pan_x: 0.5,
            pan_y: 0.5,
        }
    }
}

#[derive(Default)]
struct Shared {
    failure: Option<String>,
    lines: Option<Vec<String>>,
    /// Bumped per half-block request; a finished decode from an older one is dropped.
    generation: u64,
}

type HalfblockKey = (Option<String>, u64, u32, u32);

/// Per-picture component state (`useRef`/`useState` slots and effect bookkeeping).
pub struct PictureState {
    layer: ImageLayer,
    service: Arc<dyn ImageService>,
    mode: GraphicsProtocol,
    wake: Wake,
    shared: Arc<Mutex<Shared>>,
    handle: Option<ImageHandle>,
    applied_source: Option<(Option<String>, u64)>,
    poll: Option<JoinHandle<()>>,
    decode: Option<JoinHandle<()>>,
    halfblock_key: Option<HalfblockKey>,
    cell_box: Option<CellBox>,
}

impl PictureState {
    pub fn new(services: &AppServices, wake: Wake) -> Self {
        Self {
            layer: services.layer.clone(),
            service: services.service.clone(),
            mode: services.capabilities.graphics,
            wake,
            shared: Arc::default(),
            handle: None,
            applied_source: None,
            poll: None,
            decode: None,
            halfblock_key: None,
            cell_box: None,
        }
    }

    /// The box the last render reserved, for the image layer (`measureElement` + position).
    pub fn cell_box(&self) -> Option<CellBox> {
        self.cell_box
    }

    /// The layer entry id while registered (kitty and herdr modes).
    pub fn image_id(&self) -> Option<u32> {
        self.handle.as_ref().map(|h| h.id)
    }

    fn kitty(&self) -> bool {
        matches!(self.mode, GraphicsProtocol::Kitty | GraphicsProtocol::Herdr)
    }

    fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run the TS effects against this frame's props and box.
    pub fn sync(&mut self, props: &PictureProps<'_>, cell_box: CellBox, inner: ImageSize) {
        let box_changed = self.cell_box != Some(cell_box);
        self.cell_box = Some(cell_box);
        if self.kitty() {
            self.sync_kitty(props, cell_box, box_changed);
        } else if self.mode == GraphicsProtocol::Halfblocks {
            self.sync_halfblocks(props, inner);
        }
    }

    fn sync_kitty(&mut self, props: &PictureProps<'_>, cell_box: CellBox, box_changed: bool) {
        let source = (props.src.map(str::to_string), props.version.to_bits());
        if self.handle.is_none() {
            let handle = self.layer.register(
                source.0.clone(),
                Some(props.version),
                props.z,
                Some(props.max_upscale),
                Some(props.zoom),
            );
            self.handle = Some(handle);
            self.applied_source = None;
            self.poll = Some(self.spawn_failure_poll());
        }
        let handle = self.handle.as_ref().expect("registered above");
        handle.set_node(Some(cell_box));
        if self.applied_source.as_ref() != Some(&source) {
            handle.set_source(source.0.clone(), props.version);
            self.shared().failure = None;
            self.applied_source = Some(source);
            self.layer.schedule_flush();
        } else if box_changed {
            // The layer measures at flush time; ask for one after a move or resize.
            self.layer.schedule_flush();
        }
        handle.set_view(ImageView {
            zoom: Some(props.zoom),
            pan_x: Some(props.pan_x),
            pan_y: Some(props.pan_y),
            max_upscale: Some(props.max_upscale),
        });
    }

    /// Mirrors the 400ms `setInterval` that copies `layer.failure(id)` into state.
    fn spawn_failure_poll(&self) -> JoinHandle<()> {
        let id = self.handle.as_ref().expect("registered").id;
        let layer = self.layer.clone();
        let shared = self.shared.clone();
        let wake = self.wake.clone();
        let period = Duration::from_millis(400);
        let mut ticker = interval_at(Instant::now() + period, period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        tokio::spawn(async move {
            loop {
                ticker.tick().await;
                let problem = layer.failure(id);
                let changed = {
                    let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                    let changed = shared.failure != problem;
                    shared.failure = problem;
                    changed
                };
                if changed {
                    wake();
                }
            }
        })
    }

    fn sync_halfblocks(&mut self, props: &PictureProps<'_>, inner: ImageSize) {
        let key: HalfblockKey = (
            props.src.map(str::to_string),
            props.version.to_bits(),
            inner.width,
            inner.height,
        );
        if self.halfblock_key.as_ref() == Some(&key) {
            return;
        }
        self.halfblock_key = Some(key);
        if let Some(task) = self.decode.take() {
            task.abort();
        }
        let generation = {
            let mut shared = self.shared();
            shared.generation += 1;
            shared.failure = None;
            if props.src.is_none() {
                shared.lines = None;
            }
            shared.generation
        };
        let Some(src) = props.src else { return };
        // One cell is one pixel wide and two pixels tall.
        let target = ImageSize {
            width: inner.width,
            height: inner.height * 2,
        };
        let future = self
            .service
            .prepare(src.to_string(), target, ScaledFormat::Rgb, None);
        let shared = self.shared.clone();
        let wake = self.wake.clone();
        self.decode = Some(tokio::spawn(async move {
            let result = future.await;
            {
                let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                if shared.generation != generation {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        shared.lines = Some(rgb_to_half_block_lines(&CellArtOptions {
                            width: prepared.width as usize,
                            height: prepared.height as usize,
                            rgb: &prepared.data,
                        }));
                    }
                    Err(error) => {
                        shared.lines = None;
                        shared.failure = Some(error.to_string());
                    }
                }
            }
            wake();
        }));
    }
}

impl Drop for PictureState {
    fn drop(&mut self) {
        if let Some(task) = self.poll.take() {
            task.abort();
        }
        if let Some(task) = self.decode.take() {
            task.abort();
        }
        if let Some(handle) = self.handle.take() {
            handle.unregister();
        }
    }
}

/// `<Picture>`: renders into the `Rect` it is given (the Ink `width`/`height`).
pub struct Picture<'a> {
    pub props: PictureProps<'a>,
}

impl<'a> Picture<'a> {
    pub fn new(props: PictureProps<'a>) -> Self {
        Self { props }
    }
}

/// Parse the SGR runs `rgb_to_half_block_lines` emits (reset, 38;2 and 48;2) into spans.
fn ansi_line_to_spans(line: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();
    let mut chars = line.chars().peekable();
    let flush = |text: &mut String, style: Style, spans: &mut Vec<Span<'static>>| {
        if !text.is_empty() {
            spans.push(Span::styled(std::mem::take(text), style));
        }
    };
    while let Some(c) = chars.next() {
        if c != '\u{1b}' || chars.peek() != Some(&'[') {
            text.push(c);
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut final_byte = None;
        for p in chars.by_ref() {
            if p.is_ascii_alphabetic() {
                final_byte = Some(p);
                break;
            }
            params.push(p);
        }
        if final_byte != Some('m') {
            continue;
        }
        flush(&mut text, style, &mut spans);
        let codes: Vec<u32> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
        let mut i = 0;
        while i < codes.len() {
            match codes[i] {
                0 => style = Style::default(),
                38 | 48 if codes.get(i + 1) == Some(&2) && i + 4 < codes.len() => {
                    let color =
                        Color::Rgb(codes[i + 2] as u8, codes[i + 3] as u8, codes[i + 4] as u8);
                    style = if codes[i] == 38 {
                        style.fg(color)
                    } else {
                        style.bg(color)
                    };
                    i += 4;
                }
                _ => {}
            }
            i += 1;
        }
    }
    flush(&mut text, style, &mut spans);
    spans
}

/// Row/column offset that centers `inner` in `outer`, floored like Yoga.
fn center(outer: u16, inner: u16) -> u16 {
    outer.saturating_sub(inner) / 2
}

impl StatefulWidget for Picture<'_> {
    type State = PictureState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut PictureState) {
        let props = &self.props;
        let b: u16 = if props.border { 1 } else { 0 };
        let inner_width = area.width.saturating_sub(2 * b).max(1);
        let inner_height = area.height.saturating_sub(2 * b).max(1);

        state.sync(
            props,
            CellBox {
                x: u32::from(area.x),
                y: u32::from(area.y),
                width: u32::from(area.width),
                height: u32::from(area.height),
            },
            ImageSize {
                width: u32::from(inner_width),
                height: u32::from(inner_height),
            },
        );
        if area.width == 0 || area.height == 0 {
            return;
        }

        if props.border {
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(THEME.faint))
                .render(area, buf);
        }
        let inner = Rect::new(area.x + b, area.y + b, inner_width, inner_height).intersection(area);
        if inner.is_empty() {
            return;
        }

        let (lines, failure) = {
            let shared = state.shared();
            (shared.lines.clone(), shared.failure.clone())
        };
        let kitty = state.kitty();
        let halfblocks = state.mode == GraphicsProtocol::Halfblocks;
        let art = halfblocks
            && props.src.is_some()
            && failure.is_none()
            && lines.as_ref().is_some_and(|l| !l.is_empty());
        if art {
            let lines = lines.expect("checked above");
            let shown = lines.len().min(usize::from(inner_height));
            let top = center(inner_height, shown as u16);
            let rows: Vec<Vec<Span<'static>>> = lines
                .iter()
                .take(shown)
                .map(|l| ansi_line_to_spans(l))
                .collect();
            let width = rows
                .iter()
                .map(|spans| spans.iter().map(|s| s.width()).sum::<usize>())
                .max()
                .unwrap_or(0)
                .min(usize::from(u16::MAX)) as u16;
            let left = center(inner_width, width);
            for (i, spans) in rows.iter().enumerate() {
                put_spans(buf, inner, inner.x + left, inner.y + top + i as u16, spans);
            }
            return;
        }

        let placeholder = !kitty || props.src.is_none() || failure.is_some();
        if !placeholder {
            return;
        }
        let iw = usize::from(inner_width);
        let caption = if let Some(failure) = &failure {
            truncate(&format!("Preview unavailable · {failure}"), iw)
        } else if props.src.is_none() {
            "No image".to_string()
        } else if halfblocks {
            "Rendering…".to_string()
        } else {
            truncate(
                props.label.unwrap_or("Preview needs a graphics terminal"),
                iw,
            )
        };
        let width = text_width(&caption).min(usize::from(u16::MAX)) as u16;
        put_spans(
            buf,
            inner,
            inner.x + center(inner_width, width),
            inner.y + center(inner_height, 1),
            &[Span::styled(caption, Style::default().fg(THEME.muted))],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::context::test_support::{FakeService, services};
    use crate::ui::testing::{bg_at, fg_at, render_stateful, row, row_raw, rows};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counter_wake() -> (Wake, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        (
            Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
            count,
        )
    }

    async fn until(mut done: impl FnMut() -> bool) {
        for _ in 0..400 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("condition never became true");
    }

    fn props(src: Option<&str>) -> PictureProps<'_> {
        PictureProps {
            src,
            ..PictureProps::default()
        }
    }

    #[tokio::test]
    async fn halfblocks_draw_two_pixels_per_cell_once_the_decode_finishes() {
        let service = FakeService::new(false);
        let services = services(GraphicsProtocol::Halfblocks, service.clone());
        let (wake, count) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        let p = props(Some("/a.png"));

        // First frame: decode requested, placeholder shown.
        let buf = render_stateful(Picture::new(p.clone()), &mut state, 4, 1);
        assert_eq!(
            row(&buf, 0),
            "Rendering…".chars().take(4).collect::<String>()
        );
        until(|| count.load(Ordering::SeqCst) > 0).await;
        // One cell is one pixel wide and two pixels tall.
        let requests = service.requests.lock().unwrap().clone();
        assert_eq!(
            requests,
            vec![(
                "/a.png".to_string(),
                ImageSize {
                    width: 4,
                    height: 2
                },
                ScaledFormat::Rgb
            )]
        );

        let buf = render_stateful(Picture::new(p), &mut state, 4, 1);
        assert_eq!(row(&buf, 0), "▀▀▀▀");
        for x in 0..4 {
            assert_eq!(fg_at(&buf, x, 0), Color::Rgb(255, 0, 0));
            assert_eq!(bg_at(&buf, x, 0), Color::Rgb(0, 0, 255));
        }
        assert_eq!(service.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn halfblock_art_narrower_than_the_box_is_centered() {
        let services = services(GraphicsProtocol::Halfblocks, FakeService::new(false));
        let (wake, count) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        // Inject a 2-cell-wide, 1-line picture.
        state.sync(
            &props(Some("/a.png")),
            CellBox {
                x: 0,
                y: 0,
                width: 6,
                height: 3,
            },
            ImageSize {
                width: 6,
                height: 3,
            },
        );
        until(|| count.load(Ordering::SeqCst) > 0).await;
        state.shared().lines = Some(rgb_to_half_block_lines(&CellArtOptions {
            width: 2,
            height: 2,
            rgb: &[9; 12],
        }));
        let buf = render_stateful(Picture::new(props(Some("/a.png"))), &mut state, 6, 3);
        assert_eq!(rows(&buf), vec!["", "  ▀▀", ""]);
    }

    #[tokio::test]
    async fn halfblock_placeholders_cover_no_image_and_unavailable_previews() {
        let services_ok = services(GraphicsProtocol::Halfblocks, FakeService::new(false));
        let (wake, _) = counter_wake();
        let mut state = PictureState::new(&services_ok, wake);
        let buf = render_stateful(Picture::new(props(None)), &mut state, 12, 3);
        assert_eq!(row_raw(&buf, 1), "  No image  ");
        assert_eq!(fg_at(&buf, 2, 1), THEME.muted);

        let services_bad = services(GraphicsProtocol::Halfblocks, FakeService::new(true));
        let (wake, count) = counter_wake();
        let mut state = PictureState::new(&services_bad, wake);
        let p = props(Some("/a.png"));
        render_stateful(Picture::new(p.clone()), &mut state, 40, 3);
        until(|| count.load(Ordering::SeqCst) > 0).await;
        let buf = render_stateful(Picture::new(p), &mut state, 40, 3);
        assert_eq!(
            row(&buf, 1).trim_start(),
            "Preview unavailable · decode failed"
        );
    }

    #[tokio::test]
    async fn clearing_src_drops_the_art_and_shows_no_image() {
        let services = services(GraphicsProtocol::Halfblocks, FakeService::new(false));
        let (wake, count) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        render_stateful(Picture::new(props(Some("/a.png"))), &mut state, 4, 1);
        until(|| count.load(Ordering::SeqCst) > 0).await;
        let buf = render_stateful(Picture::new(props(None)), &mut state, 8, 1);
        assert_eq!(row(&buf, 0), "No image");
    }

    #[tokio::test]
    async fn without_graphics_the_label_or_default_hint_is_shown() {
        let services = services(GraphicsProtocol::None, FakeService::new(false));
        let (wake, _) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        let p = PictureProps {
            src: Some("/a.png"),
            label: Some("still"),
            ..PictureProps::default()
        };
        let buf = render_stateful(Picture::new(p), &mut state, 9, 1);
        assert_eq!(row_raw(&buf, 0), "  still  ");
        let buf = render_stateful(Picture::new(props(Some("/a.png"))), &mut state, 40, 1);
        assert_eq!(
            row(&buf, 0).trim_start(),
            "Preview needs a graphics terminal"
        );
        // Narrow boxes truncate with an ellipsis.
        let buf = render_stateful(Picture::new(props(Some("/a.png"))), &mut state, 10, 1);
        assert_eq!(row(&buf, 0), "Preview n…");
    }

    #[tokio::test]
    async fn border_draws_a_rounded_frame_and_shrinks_the_inner_box() {
        let services = services(GraphicsProtocol::None, FakeService::new(false));
        let (wake, _) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        let p = PictureProps {
            src: None,
            border: true,
            ..PictureProps::default()
        };
        let buf = render_stateful(Picture::new(p), &mut state, 12, 3);
        assert_eq!(
            rows(&buf),
            vec!["╭──────────╮", "│ No image │", "╰──────────╯"]
        );
        assert_eq!(fg_at(&buf, 0, 0), THEME.faint);
    }

    #[tokio::test]
    async fn kitty_mode_reports_the_rendered_cell_box_and_reaches_the_layer() {
        let service = FakeService::new(false);
        let services = services(GraphicsProtocol::Kitty, service.clone());
        let (wake, _) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        assert_eq!(state.cell_box(), None);

        let area = Rect::new(2, 1, 10, 4);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 8));
        Picture::new(props(Some("/a.png"))).render(area, &mut buf, &mut state);
        assert_eq!(
            state.cell_box(),
            Some(CellBox {
                x: 2,
                y: 1,
                width: 10,
                height: 4
            })
        );
        assert!(state.image_id().is_some());
        // Kitty paints pixels out of band: nothing is drawn into the buffer.
        assert!(rows(&buf).iter().all(String::is_empty));

        // The layer received the box: its flush asks the service for a picture sized to it.
        until(|| !service.requests.lock().unwrap().is_empty()).await;
        assert_eq!(service.requests.lock().unwrap()[0].0, "/a.png");

        // A move is reported on the next render.
        Picture::new(props(Some("/a.png"))).render(Rect::new(0, 0, 6, 2), &mut buf, &mut state);
        assert_eq!(
            state.cell_box(),
            Some(CellBox {
                x: 0,
                y: 0,
                width: 6,
                height: 2
            })
        );
    }

    #[tokio::test]
    async fn kitty_mode_without_a_source_shows_no_image_and_failures_surface_by_polling() {
        let services = services(GraphicsProtocol::Kitty, FakeService::new(true));
        let (wake, count) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        let buf = render_stateful(Picture::new(props(None)), &mut state, 12, 1);
        assert_eq!(row(&buf, 0).trim_start(), "No image");

        let p = props(Some("/a.png"));
        render_stateful(Picture::new(p.clone()), &mut state, 40, 1);
        let id = state.image_id().unwrap();
        until(|| services.layer.failure(id).is_some()).await;
        until(|| count.load(Ordering::SeqCst) > 0).await;
        let buf = render_stateful(Picture::new(p), &mut state, 40, 1);
        assert_eq!(
            row(&buf, 0).trim_start(),
            "Preview unavailable · decode failed"
        );
    }

    #[tokio::test]
    async fn dropping_the_state_unregisters_the_image_from_the_layer() {
        let services = services(GraphicsProtocol::Kitty, FakeService::new(false));
        let (wake, _) = counter_wake();
        let mut state = PictureState::new(&services, wake);
        render_stateful(Picture::new(props(Some("/a.png"))), &mut state, 10, 2);
        let id = state.image_id().unwrap();
        services.layer.flush_now();
        drop(state);
        assert!(services.layer.prepared(id).is_none());
        assert!(services.layer.failure(id).is_none());
    }

    #[test]
    fn ansi_lines_parse_into_truecolor_spans() {
        let spans = ansi_line_to_spans("\x1b[38;2;1;2;3m\x1b[48;2;4;5;6m▀▀\x1b[0mx");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "▀▀");
        assert_eq!(
            spans[0].style,
            Style::default()
                .fg(Color::Rgb(1, 2, 3))
                .bg(Color::Rgb(4, 5, 6))
        );
        assert_eq!(spans[1].content, "x");
        assert_eq!(spans[1].style, Style::default());
    }
}
