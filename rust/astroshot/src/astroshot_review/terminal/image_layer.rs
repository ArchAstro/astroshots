//! Port of `packages/astroshot-review/src/terminal/image-layer.ts`.
//!
//! Bridges the UI's layout to kitty graphics placements.
//!
//! Components register a source path and the cell box that reserves room for
//! the picture. After every frame the layer fits the picture into that box,
//! transmits bytes the terminal has not seen yet, and (re)places every
//! visible image. Placements share the frame's synchronized-update block so
//! text and pictures land together.
//!
//! Differences from the TS, all forced by the language:
//!
//! - Ink's `DOMElement` + `measureElement` become [`CellBox`]: the UI measures
//!   the reserved box after layout and passes it to `set_node`.
//! - The layer is a cheap-to-clone handle (`Arc` inside); the async `prepare`
//!   completions and the deferred flush run on the tokio runtime that was
//!   current when the layer was created, so construct it inside one.
//! - `ImageService` and `PreparedImage` live in `images::service`
//!   (`images/service.ts`); the service implementation lands with that port.
//! - herdr is `cfg(unix)`; non-unix builds get an uninhabited `HerdrSink`.
//! - `ImageLayerOptions::image_id_base` pins the otherwise random first
//!   kitty image id (`1000 + random`) so tests can assert exact bytes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::runtime::Handle;

pub use crate::astroshot_review::images::service::{ImageService, PrepareFuture, PreparedImage};

use super::kitty::{
    DeleteRequest, ImageFormat, PlaceRequest, RESTORE_CURSOR, SAVE_CURSOR, TransmitRequest,
    cursor_to, encode_delete, encode_place, encode_transmit,
};
use super::probe::{GraphicsProtocol, TerminalCapabilities};
use crate::astroshot_review::images::png::{ImageSize, fit_inside};
use crate::astroshot_review::images::scale::{Rect, ScaledFormat};

#[cfg(unix)]
pub use super::herdr::{HerdrPlacement, HerdrSink};
#[cfg(not(unix))]
pub use non_unix::{HerdrPlacement, HerdrSink};

/// herdr is unix-only. Without it the layer never holds a sink, so this
/// stand-in is uninhabited and every method is unreachable.
#[cfg(not(unix))]
mod non_unix {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HerdrPlacement {
        pub col: u32,
        pub row: u32,
        pub cols: u32,
        pub rows: u32,
        pub z: i32,
    }

    pub enum HerdrSink {}

    impl HerdrSink {
        pub fn generation(&self) -> u64 {
            match *self {}
        }
        pub fn set(&self, _: &str, _: Vec<u8>, _: u32, _: u32, _: HerdrPlacement) {
            match *self {}
        }
        pub fn clear(&self, _: &str) {
            match *self {}
        }
        pub fn clear_all(&self) {
            match *self {}
        }
        pub fn dispose(&self) {
            match *self {}
        }
    }
}

/// The cell box the UI reserved for a picture (what `measureElement` returns
/// plus the element's position on screen, 0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub type ErrorCallback = Box<dyn Fn(&str, anyhow::Error) + Send + Sync>;
pub type DebugCallback = Box<dyn Fn(&str) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageView {
    pub zoom: Option<f64>,
    pub pan_x: Option<f64>,
    pub pan_y: Option<f64>,
    pub max_upscale: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Frames,
}

#[derive(Debug, Clone, Copy)]
struct FrameState {
    image_id: u32,
    width: u32,
    height: u32,
}

struct Entry {
    src: Option<String>,
    version: f64,
    z: i32,
    /// How far past native size the image may scale (1 = never upscale).
    max_upscale: f64,
    /// Magnification: 1 shows the whole image; >1 crops in.
    zoom: f64,
    /// Pan center as a fraction of the source image, [0,1].
    pan_x: f64,
    pan_y: f64,
    node: Option<CellBox>,
    ready: Option<Arc<PreparedImage>>,
    requested_key: Option<String>,
    failed: Option<String>,
    /// Live frame state, for frame entries.
    frame: Option<FrameState>,
    /// Last frame bytes, kept so herdr can re-place on a resize (the TS
    /// stores it and never reads it back).
    #[allow(dead_code)]
    frame_data: Option<Vec<u8>>,
    kind: EntryKind,
}

impl Entry {
    fn new(kind: EntryKind, src: Option<String>, z: i32) -> Self {
        Self {
            src,
            version: 0.0,
            z,
            max_upscale: 1.0,
            zoom: 1.0,
            pan_x: 0.5,
            pan_y: 0.5,
            node: None,
            ready: None,
            requested_key: None,
            failed: None,
            frame: None,
            frame_data: None,
            kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub placement_id: u32,
    pub image_id: u32,
    pub col: u32,
    pub row: u32,
    pub cols: u32,
    pub rows: u32,
    pub z: i32,
}

/// A JS `Map<number, Placement>`: insertion order, `set` keeps an existing
/// key's position.
#[derive(Debug, Clone, Default)]
struct PlacementMap(Vec<(u32, Placement)>);

impl PlacementMap {
    fn set(&mut self, id: u32, placement: Placement) {
        match self.0.iter_mut().find(|(key, _)| *key == id) {
            Some(slot) => slot.1 = placement,
            None => self.0.push((id, placement)),
        }
    }

    fn remove(&mut self, id: u32) {
        self.0.retain(|(key, _)| *key != id);
    }

    fn has(&self, id: u32) -> bool {
        self.0.iter().any(|(key, _)| *key == id)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn iter(&self) -> impl Iterator<Item = &(u32, Placement)> {
        self.0.iter()
    }
}

pub struct ImageLayerOptions {
    pub capabilities: TerminalCapabilities,
    pub service: Arc<dyn ImageService>,
    /// Writes straight to the terminal, bypassing the UI (used for async readiness).
    pub write: Box<dyn Fn(&str) + Send + Sync>,
    /// Called when an image finished preparing and the screen should refresh.
    pub on_ready: Option<Box<dyn Fn() + Send + Sync>>,
    pub on_error: Option<ErrorCallback>,
    /// Diagnostics sink (enabled by ASTROSHOT_REVIEW_DEBUG).
    pub on_debug: Option<DebugCallback>,
    /// When present, images are placed through herdr's socket API instead of Kitty escapes.
    pub herdr: Option<HerdrSink>,
    pub max_transmitted: Option<usize>,
    /// First kitty image id; random in `1000..101000` when `None`.
    pub image_id_base: Option<u32>,
}

impl ImageLayerOptions {
    pub fn new(
        capabilities: TerminalCapabilities,
        service: Arc<dyn ImageService>,
        write: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            capabilities,
            service,
            write: Box::new(write),
            on_ready: None,
            on_error: None,
            on_debug: None,
            herdr: None,
            max_transmitted: None,
            image_id_base: None,
        }
    }
}

struct TransmittedImage {
    id: u32,
    key: String,
    last_used: u64,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<u32, Entry>,
    /// Insertion-ordered (a JS `Map`); small, so linear lookup.
    transmitted: Vec<TransmittedImage>,
    last_placements: PlacementMap,
    next_entry_id: u32,
    next_image_id: u32,
    tick: u64,
    resync: bool,
    flush_scheduled: bool,
    herdr_layers: BTreeMap<u32, String>,
    herdr_generation: i64,
    last_summary: String,
    /// Row offset of the live region's first line, 1-based screen row minus 1.
    origin_row: u32,
    origin_col: u32,
}

struct Shared {
    capabilities: TerminalCapabilities,
    service: Arc<dyn ImageService>,
    write: Box<dyn Fn(&str) + Send + Sync>,
    on_ready: Option<Box<dyn Fn() + Send + Sync>>,
    on_error: Option<ErrorCallback>,
    on_debug: Option<DebugCallback>,
    herdr: Option<HerdrSink>,
    max_transmitted: usize,
    runtime: Handle,
    state: Mutex<State>,
}

/// Cheap to clone; all clones drive the same layer.
#[derive(Clone)]
pub struct ImageLayer {
    shared: Arc<Shared>,
}

fn herdr_layer_id(entry_id: u32) -> String {
    format!("astro-{entry_id}")
}

fn sync_block(output: &str) -> String {
    format!("\x1b[?2026h{output}\x1b[?2026l")
}

fn random_image_base() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    1000 + nanos.wrapping_mul(2_654_435_761) % 100_000
}

impl ImageLayer {
    /// Must be called inside a tokio runtime.
    pub fn new(options: ImageLayerOptions) -> Self {
        let state = State {
            next_entry_id: 1,
            next_image_id: options.image_id_base.unwrap_or_else(random_image_base),
            resync: true,
            herdr_generation: -1,
            ..State::default()
        };
        Self {
            shared: Arc::new(Shared {
                capabilities: options.capabilities,
                service: options.service,
                write: options.write,
                on_ready: options.on_ready,
                on_error: options.on_error,
                on_debug: options.on_debug,
                herdr: options.herdr,
                max_transmitted: options.max_transmitted.unwrap_or(48),
                runtime: Handle::current(),
                state: Mutex::new(state),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn enabled(&self) -> bool {
        self.shared.capabilities.graphics == GraphicsProtocol::Kitty || self.shared.herdr.is_some()
    }

    pub fn set_origin(&self, row: u32, col: u32) {
        let mut state = self.state();
        state.origin_row = row;
        state.origin_col = col;
    }

    pub fn origin(&self) -> (u32, u32) {
        let state = self.state();
        (state.origin_row, state.origin_col)
    }

    pub fn register(
        &self,
        src: Option<String>,
        version: Option<f64>,
        z: Option<i32>,
        max_upscale: Option<f64>,
        zoom: Option<f64>,
    ) -> ImageHandle {
        let mut state = self.state();
        let id = state.next_entry_id;
        state.next_entry_id += 1;
        let mut entry = Entry::new(EntryKind::File, src, z.unwrap_or(0));
        entry.version = version.unwrap_or(0.0);
        entry.max_upscale = max_upscale.unwrap_or(1.0);
        entry.zoom = zoom.unwrap_or(1.0);
        state.entries.insert(id, entry);
        ImageHandle {
            layer: self.clone(),
            id,
        }
    }

    pub fn register_frames(&self, z: Option<i32>) -> FrameHandle {
        let mut state = self.state();
        let id = state.next_entry_id;
        state.next_entry_id += 1;
        state
            .entries
            .insert(id, Entry::new(EntryKind::Frames, None, z.unwrap_or(1)));
        FrameHandle {
            layer: self.clone(),
            id,
        }
    }

    /// Where a frame entry's current picture lands, given the live layout.
    fn placement_for(&self, entry_id: u32, entry: &Entry) -> Option<Placement> {
        let node = entry.node?;
        let frame = entry.frame?;
        if node.width == 0 || node.height == 0 {
            return None;
        }
        let cell_width = self.shared.capabilities.cell_width;
        let cell_height = self.shared.capabilities.cell_height;
        let target_px = ImageSize {
            width: node.width * cell_width,
            height: node.height * cell_height,
        };
        let fitted = fit_inside(
            ImageSize {
                width: frame.width,
                height: frame.height,
            },
            target_px,
        );
        let cols = node
            .width
            .min(1.max(js_round(f64::from(fitted.width) / f64::from(cell_width))));
        let rows = node
            .height
            .min(1.max(js_round(f64::from(fitted.height) / f64::from(cell_height))));
        Some(Placement {
            placement_id: entry_id,
            image_id: frame.image_id,
            col: node.x + (node.width - cols) / 2,
            row: node.y + (node.height - rows) / 2,
            cols,
            rows,
            z: entry.z,
        })
    }

    fn place_command(state: &State, placement: &Placement) -> String {
        cursor_to(
            placement.row + 1 + state.origin_row,
            placement.col + 1 + state.origin_col,
        ) + &encode_place(&PlaceRequest {
            id: placement.image_id,
            placement_id: placement.placement_id,
            cols: placement.cols,
            rows: placement.rows,
            z: Some(placement.z),
        })
    }

    /// Prepared image for an entry, if any (lets components show dimensions).
    pub fn prepared(&self, id: u32) -> Option<Arc<PreparedImage>> {
        self.state()
            .entries
            .get(&id)
            .and_then(|entry| entry.ready.clone())
    }

    pub fn failure(&self, id: u32) -> Option<String> {
        self.state()
            .entries
            .get(&id)
            .and_then(|entry| entry.failed.clone())
    }

    /// After a resize or screen clear, every placement must be re-sent.
    pub fn invalidate(&self) {
        let mut state = self.state();
        state.resync = true;
        // Force herdr layers to be re-set at their new positions.
        state.herdr_layers.clear();
    }

    /// Reconcile the herdr layers with the current placement set: (re)place
    /// any image whose bytes or box changed, and clear layers no longer shown.
    fn sync_herdr(
        &self,
        state: &mut State,
        placements: &PlacementMap,
        ready_by_entry: &BTreeMap<u32, Arc<PreparedImage>>,
    ) {
        let Some(sink) = self.shared.herdr.as_ref() else {
            return;
        };
        // A dropped-and-restored connection loses server-side layers; re-send all.
        let generation = sink.generation() as i64;
        if generation != state.herdr_generation {
            state.herdr_generation = generation;
            state.herdr_layers.clear();
        }
        for (entry_id, placement) in placements.iter() {
            let Some(ready) = ready_by_entry.get(entry_id) else {
                continue; // frame entries place themselves in push_frame
            };
            let signature = format!(
                "{}|{},{},{},{},{}",
                ready.key,
                placement.col,
                placement.row,
                placement.cols,
                placement.rows,
                placement.z
            );
            if state.herdr_layers.get(entry_id) == Some(&signature) {
                continue;
            }
            sink.set(
                &herdr_layer_id(*entry_id),
                ready.data.clone(),
                ready.width,
                ready.height,
                herdr_placement(placement),
            );
            state.herdr_layers.insert(*entry_id, signature);
        }
        let layer_ids: Vec<u32> = state.herdr_layers.keys().copied().collect();
        for entry_id in layer_ids {
            // Keep active file placements and live frame entries; clear the rest.
            if placements.has(entry_id) {
                continue;
            }
            if state
                .entries
                .get(&entry_id)
                .is_some_and(|entry| entry.kind == EntryKind::Frames && entry.frame.is_some())
            {
                continue;
            }
            sink.clear(&herdr_layer_id(entry_id));
            state.herdr_layers.remove(&entry_id);
        }
    }

    /// Escape sequences that bring the terminal in line with the current layout.
    pub fn render(&self) -> String {
        if !self.enabled() {
            return String::new();
        }
        let (output, debug) = {
            let mut state = self.state();
            self.render_locked(&mut state)
        };
        if let (Some(message), Some(on_debug)) = (debug, self.shared.on_debug.as_ref()) {
            on_debug(&message);
        }
        output
    }

    fn render_locked(&self, state: &mut State) -> (String, Option<String>) {
        state.tick += 1;
        let herdr = self.shared.herdr.is_some();
        let mut placements = PlacementMap::default();
        let mut ready_by_entry: BTreeMap<u32, Arc<PreparedImage>> = BTreeMap::new();
        let mut output = String::new();
        let cell_width = self.shared.capabilities.cell_width;
        let cell_height = self.shared.capabilities.cell_height;

        let entry_ids: Vec<u32> = state.entries.keys().copied().collect();
        for entry_id in entry_ids {
            let entry = &state.entries[&entry_id];
            if entry.kind == EntryKind::Frames {
                // Frame entries place themselves in push_frame; on herdr they
                // re-place there too. Only the kitty escape path needs them
                // re-emitted here.
                if !herdr && let Some(placement) = self.placement_for(entry_id, entry) {
                    placements.set(entry_id, placement);
                }
                continue;
            }
            let (Some(src), Some(node)) = (entry.src.clone(), entry.node) else {
                continue;
            };
            if src.is_empty() {
                continue;
            }
            if node.width == 0 || node.height == 0 {
                continue;
            }
            let box_px_w = f64::from(node.width * cell_width);
            let box_px_h = f64::from(node.height * cell_height);
            // Prepare above the cell box so the compositor only ever
            // downscales (downscaling stays sharp; upscaling blurs). herdr may
            // render the box at more physical pixels than its reported cell
            // size implies, so oversample.
            let supersample = if herdr { 2 } else { 1 };
            let target_px = ImageSize {
                width: node.width * cell_width * supersample,
                height: node.height * cell_height * supersample,
            };
            // Zoom > 1 shows a centered sub-rectangle of the source (a real
            // crop, so it magnifies instead of squishing); pan slides that
            // rectangle. The on-screen footprint stays fixed; only the visible
            // region changes.
            let mut crop: Option<Rect> = None;
            if entry.zoom > 1.0
                && let Some(ready) = entry.ready.as_ref()
            {
                let source_w = f64::from(ready.source_width);
                let source_h = f64::from(ready.source_height);
                let crop_w = source_w / entry.zoom;
                let crop_h = source_h / entry.zoom;
                let center_x =
                    (crop_w / 2.0).max((source_w - crop_w / 2.0).min(entry.pan_x * source_w));
                let center_y =
                    (crop_h / 2.0).max((source_h - crop_h / 2.0).min(entry.pan_y * source_h));
                crop = Some(Rect {
                    x: center_x - crop_w / 2.0,
                    y: center_y - crop_h / 2.0,
                    width: crop_w,
                    height: crop_h,
                });
            }
            let crop_key = match crop {
                Some(crop) => format!(
                    "|z{}|{},{},{},{}",
                    to_fixed2(entry.zoom),
                    js_round_f(crop.x),
                    js_round_f(crop.y),
                    js_round_f(crop.width),
                    js_round_f(crop.height)
                ),
                None => String::new(),
            };
            let request_key = format!(
                "{}|{}|{}x{}{}",
                src, entry.version, target_px.width, target_px.height, crop_key
            );
            if entry.requested_key.as_deref() != Some(request_key.as_str()) {
                let future =
                    self.shared
                        .service
                        .prepare(src.clone(), target_px, ScaledFormat::Png, crop);
                let layer = self.clone();
                let key = request_key.clone();
                self.shared.runtime.spawn(async move {
                    let result = future.await;
                    layer.finish_prepare(entry_id, key, src, result);
                });
                let entry = state.entries.get_mut(&entry_id).expect("entry exists");
                entry.requested_key = Some(request_key);
                entry.failed = None;
            }
            let entry = &state.entries[&entry_id];
            let Some(ready) = entry.ready.clone() else {
                continue;
            };
            ready_by_entry.insert(entry_id, ready.clone());

            // On-screen footprint from the FULL image aspect (constant across
            // zoom, so zooming magnifies in place without moving the
            // picture). Fits the box, upscaling small sources up to
            // `max_upscale` times to fill.
            let full_width = f64::from(ready.source_width);
            let full_height = f64::from(ready.source_height);
            let contain_scale = (box_px_w / full_width).min(box_px_h / full_height);
            let scale = contain_scale.min(1.0_f64.max(entry.max_upscale));
            let cols = node
                .width
                .min(1.max(js_round(full_width * scale / f64::from(cell_width))));
            let rows = node
                .height
                .min(1.max(js_round(full_height * scale / f64::from(cell_height))));
            let col = node.x + (node.width - cols) / 2;
            let row = node.y + (node.height - rows) / 2;
            let z = entry.z;

            let mut image_id = 0;
            if !herdr {
                let tick = state.tick;
                let position = state
                    .transmitted
                    .iter()
                    .position(|image| image.key == ready.key);
                let index = match position {
                    Some(index) => index,
                    None => {
                        let id = state.next_image_id;
                        state.next_image_id += 1;
                        state.transmitted.push(TransmittedImage {
                            id,
                            key: ready.key.clone(),
                            last_used: tick,
                        });
                        let mut request = TransmitRequest::new(id, ImageFormat::Png, &ready.data);
                        if self.shared.capabilities.file_medium && ready.is_original {
                            request.file_path = Some(ready.path.clone());
                        }
                        output += &encode_transmit(&request);
                        state.transmitted.len() - 1
                    }
                };
                let image = &mut state.transmitted[index];
                image.last_used = tick;
                image_id = image.id;
            }
            placements.set(
                entry_id,
                Placement {
                    placement_id: entry_id,
                    image_id,
                    col,
                    row,
                    cols,
                    rows,
                    z,
                },
            );
        }

        // herdr composites images on named layers over the pane's text; drive
        // its socket API instead of writing Kitty escapes the multiplexer
        // would drop.
        if herdr {
            self.sync_herdr(state, &placements, &ready_by_entry);
            state.last_placements = placements.clone();
            let mut debug = None;
            if self.shared.on_debug.is_some() {
                let summary = format!(
                    "herdr layers={} placed={}",
                    state.herdr_layers.len(),
                    placements.len()
                );
                if summary != state.last_summary {
                    state.last_summary = summary.clone();
                    debug = Some(summary);
                }
            }
            return (String::new(), debug);
        }

        if state.resync {
            output += &encode_delete(&DeleteRequest::AllPlacements);
        } else {
            for (placement_id, previous) in state.last_placements.iter() {
                if !placements.has(*placement_id) {
                    output += &encode_delete(&DeleteRequest::Placement {
                        id: previous.image_id,
                        placement_id: *placement_id,
                    });
                }
            }
        }

        if placements.len() > 0 {
            output += SAVE_CURSOR;
            for (_, placement) in placements.iter() {
                output += &Self::place_command(state, placement);
            }
            output += RESTORE_CURSOR;
        }

        output += &self.evict_transmitted(state, &placements);
        state.last_placements = placements;
        state.resync = false;
        let mut debug = None;
        if self.shared.on_debug.is_some() {
            let summary = format!(
                "entries={} placed={} transmitted={} bytes={}",
                state.entries.len(),
                state.last_placements.len(),
                state.transmitted.len(),
                // JS `.length` counts UTF-16 units.
                output.encode_utf16().count()
            );
            if summary != state.last_summary {
                state.last_summary = summary.clone();
                debug = Some(summary);
            }
        }
        (output, debug)
    }

    fn evict_transmitted(&self, state: &mut State, active: &PlacementMap) -> String {
        let max = self.shared.max_transmitted;
        if state.transmitted.len() <= max {
            return String::new();
        }
        let in_use: Vec<u32> = active.iter().map(|(_, p)| p.image_id).collect();
        let mut candidates: Vec<(String, u32, u64)> = state
            .transmitted
            .iter()
            .filter(|image| !in_use.contains(&image.id))
            .map(|image| (image.key.clone(), image.id, image.last_used))
            .collect();
        candidates.sort_by_key(|(_, _, last_used)| *last_used); // stable, like Array.sort
        let mut candidates = candidates.into_iter();
        let mut output = String::new();
        while state.transmitted.len() > max {
            let Some((key, id, _)) = candidates.next() else {
                break;
            };
            state.transmitted.retain(|image| image.key != key);
            output += &encode_delete(&DeleteRequest::Image { id });
        }
        output
    }

    /// Completion of a `service.prepare` request started by `render`.
    fn finish_prepare(
        &self,
        entry_id: u32,
        request_key: String,
        src: String,
        result: anyhow::Result<Arc<PreparedImage>>,
    ) {
        {
            let mut state = self.state();
            // The TS compares against the entry object it captured, which
            // survives `unregister`; a missing entry therefore still counts
            // as current.
            if let Some(entry) = state.entries.get_mut(&entry_id) {
                if entry.requested_key.as_deref() != Some(request_key.as_str()) {
                    return;
                }
                match &result {
                    Ok(prepared) => entry.ready = Some(prepared.clone()),
                    Err(error) => entry.failed = Some(error.to_string()),
                }
            }
        }
        match result {
            Ok(_) => {
                self.schedule_flush();
                self.notify_ready();
            }
            Err(error) => {
                if let Some(on_error) = self.shared.on_error.as_ref() {
                    on_error(&src, error);
                }
                self.notify_ready();
            }
        }
    }

    fn notify_ready(&self) {
        if let Some(on_ready) = self.shared.on_ready.as_ref() {
            on_ready();
        }
    }

    /// Write placements now, outside a UI frame. Safe for the cursor.
    pub fn flush_now(&self) {
        self.state().flush_scheduled = false;
        if !self.enabled() {
            return;
        }
        let output = self.render();
        if !output.is_empty() {
            (self.shared.write)(&sync_block(&output));
        }
    }

    pub fn schedule_flush(&self) {
        {
            let mut state = self.state();
            if state.flush_scheduled || !self.enabled() {
                return;
            }
            state.flush_scheduled = true;
        }
        let layer = self.clone();
        // `setImmediate`: run after the current task yields.
        self.shared.runtime.spawn(async move {
            tokio::task::yield_now().await;
            let scheduled = layer.state().flush_scheduled;
            if scheduled {
                layer.flush_now();
            }
        });
    }

    /// `herdrSink.dispose()`: close every herdr stream for good, including
    /// ones still opening, so nothing re-places a layer after the tray quits.
    pub fn dispose_herdr(&self) {
        if let Some(sink) = self.shared.herdr.as_ref() {
            sink.dispose();
        }
    }

    /// Remove every placement and free image data in the terminal.
    pub fn clear(&self) -> String {
        let mut state = self.state();
        state.last_placements = PlacementMap::default();
        state.transmitted.clear();
        state.resync = true;
        if let Some(sink) = self.shared.herdr.as_ref() {
            sink.clear_all();
            state.herdr_layers.clear();
            return String::new();
        }
        if self.enabled() {
            encode_delete(&DeleteRequest::All)
        } else {
            String::new()
        }
    }
}

fn herdr_placement(placement: &Placement) -> HerdrPlacement {
    HerdrPlacement {
        col: placement.col,
        row: placement.row,
        cols: placement.cols,
        rows: placement.rows,
        z: placement.z,
    }
}

/// `Math.round` for non-negative values, as a cell count.
fn js_round(value: f64) -> u32 {
    (value + 0.5).floor() as u32
}

fn js_round_f(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// `Number.prototype.toFixed(2)`.
fn to_fixed2(value: f64) -> String {
    format!("{value:.2}")
}

/// A file picture registered with the layer (`ImageHandle`).
#[derive(Clone)]
pub struct ImageHandle {
    layer: ImageLayer,
    pub id: u32,
}

impl ImageHandle {
    pub fn set_node(&self, node: Option<CellBox>) {
        if let Some(entry) = self.layer.state().entries.get_mut(&self.id) {
            entry.node = node;
        }
    }

    /// `version` (for example the file's mtime) forces a fresh decode when the bytes change.
    pub fn set_source(&self, src: Option<String>, version: f64) {
        {
            let mut state = self.layer.state();
            let Some(entry) = state.entries.get_mut(&self.id) else {
                return;
            };
            if entry.src == src && entry.version == version {
                return;
            }
            entry.src = src;
            entry.version = version;
            entry.ready = None;
            entry.requested_key = None;
            entry.failed = None;
        }
        self.layer.schedule_flush();
    }

    /// Update the view: zoom (1 = whole image), pan center in [0,1], and the fill cap.
    pub fn set_view(&self, view: ImageView) {
        let changed = {
            let mut state = self.layer.state();
            let Some(entry) = state.entries.get_mut(&self.id) else {
                return;
            };
            let mut changed = false;
            if let Some(zoom) = view.zoom.filter(|zoom| *zoom != entry.zoom) {
                entry.zoom = zoom;
                changed = true;
            }
            if let Some(pan_x) = view.pan_x.filter(|pan| *pan != entry.pan_x) {
                entry.pan_x = pan_x;
                changed = true;
            }
            if let Some(pan_y) = view.pan_y.filter(|pan| *pan != entry.pan_y) {
                entry.pan_y = pan_y;
                changed = true;
            }
            if let Some(max_upscale) = view.max_upscale.filter(|max| *max != entry.max_upscale) {
                entry.max_upscale = max_upscale;
                changed = true;
            }
            changed
        };
        if changed {
            self.layer.schedule_flush();
        }
    }

    pub fn unregister(&self) {
        {
            let mut state = self.layer.state();
            state.entries.remove(&self.id);
            if let Some(sink) = self.layer.shared.herdr.as_ref() {
                sink.clear(&herdr_layer_id(self.id));
                state.herdr_layers.remove(&self.id);
            }
        }
        self.layer.schedule_flush();
    }
}

/// A picture that changes every few milliseconds (movie playback).
#[derive(Clone)]
pub struct FrameHandle {
    layer: ImageLayer,
    pub id: u32,
}

impl FrameHandle {
    pub fn set_node(&self, node: Option<CellBox>) {
        if let Some(entry) = self.layer.state().entries.get_mut(&self.id) {
            entry.node = node;
        }
    }

    /// Transmit and show a new PNG frame immediately.
    pub fn push_frame(&self, png: Vec<u8>, width: u32, height: u32) {
        let layer = &self.layer;
        if !layer.enabled() {
            return;
        }
        let output = {
            let mut state = layer.state();
            let image_id = state.next_image_id;
            state.next_image_id += 1;
            let Some(entry) = state.entries.get_mut(&self.id) else {
                return;
            };
            entry.frame = Some(FrameState {
                image_id,
                width,
                height,
            });
            entry.frame_data = Some(png.clone());
            if let Some(sink) = layer.shared.herdr.as_ref() {
                let entry = &state.entries[&self.id];
                if let Some(placement) = layer.placement_for(self.id, entry) {
                    sink.set(
                        &herdr_layer_id(self.id),
                        png,
                        width,
                        height,
                        herdr_placement(&placement),
                    );
                    state
                        .herdr_layers
                        .insert(self.id, format!("frame-{image_id}"));
                }
                return;
            }
            let mut output =
                encode_transmit(&TransmitRequest::new(image_id, ImageFormat::Png, &png));
            let placement = layer.placement_for(self.id, &state.entries[&self.id]);
            if let Some(placement) = placement {
                output += SAVE_CURSOR;
                output += &ImageLayer::place_command(&state, &placement);
                output += RESTORE_CURSOR;
                state.last_placements.set(self.id, placement);
            }
            output
        };
        (layer.shared.write)(&sync_block(&output));
    }

    /// Drop the current frame (the poster shows through again).
    ///
    /// The TS clears `entry.frame` and only then calls `dropFrame()`, which
    /// returns "" when there is no frame. So no kitty delete is ever written
    /// here; the picture goes away at the next `render` (placement removed)
    /// or eviction. Kept byte-for-byte.
    pub fn clear_frame(&self) {
        let mut state = self.layer.state();
        if let Some(entry) = state.entries.get_mut(&self.id) {
            entry.frame = None;
            entry.frame_data = None;
        }
        if let Some(sink) = self.layer.shared.herdr.as_ref() {
            sink.clear(&herdr_layer_id(self.id));
            state.herdr_layers.remove(&self.id);
            return;
        }
        state.last_placements.remove(self.id);
    }

    /// Same `dropFrame()` quirk as `clear_frame`: nothing is written.
    pub fn unregister(&self) {
        let mut state = self.layer.state();
        if let Some(sink) = self.layer.shared.herdr.as_ref() {
            sink.clear(&herdr_layer_id(self.id));
            state.herdr_layers.remove(&self.id);
            state.entries.remove(&self.id);
            return;
        }
        state.entries.remove(&self.id);
        state.last_placements.remove(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::terminal::graphics_stdout::create_graphics_stdout;
    use crate::astroshot_review::terminal::probe::CellSource;
    use std::io::Write;

    const BASE: u32 = 5000;

    fn capabilities(graphics: GraphicsProtocol, file_medium: bool) -> TerminalCapabilities {
        TerminalCapabilities {
            graphics,
            file_medium,
            cell_width: 10,
            cell_height: 20,
            cell_source: CellSource::Env,
            reason: None,
            inside_tmux: false,
            inside_ssh: false,
            inside_mosh: false,
            inside_herdr: false,
            intercepted: None,
        }
    }

    type Request = (String, ImageSize, Option<Rect>);

    /// Resolves immediately with a canned image (source size fixed per test).
    struct FakeService {
        source: ImageSize,
        is_original: bool,
        fail: bool,
        requests: Mutex<Vec<Request>>,
    }

    impl FakeService {
        fn new(width: u32, height: u32) -> Arc<Self> {
            Arc::new(Self {
                source: ImageSize { width, height },
                is_original: false,
                fail: false,
                requests: Mutex::new(Vec::new()),
            })
        }

        fn requests(&self) -> Vec<Request> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl ImageService for FakeService {
        fn prepare(
            &self,
            file_path: String,
            target: ImageSize,
            _format: ScaledFormat,
            crop: Option<Rect>,
        ) -> PrepareFuture {
            self.requests
                .lock()
                .unwrap()
                .push((file_path.clone(), target, crop));
            let result = if self.fail {
                Err(anyhow::anyhow!("decode failed"))
            } else {
                Ok(Arc::new(PreparedImage {
                    key: format!("{file_path}@{}x{}", target.width, target.height),
                    path: file_path,
                    width: target.width,
                    height: target.height,
                    source_width: self.source.width,
                    source_height: self.source.height,
                    format: ScaledFormat::Png,
                    data: vec![1, 2, 3],
                    is_original: self.is_original,
                    mtime_ms: 0.0,
                    size: 3,
                }))
            };
            Box::pin(async move { result })
        }
    }

    type Writes = Arc<Mutex<Vec<String>>>;

    fn layer_with(
        graphics: GraphicsProtocol,
        file_medium: bool,
        service: Arc<FakeService>,
    ) -> (ImageLayer, Writes) {
        let writes: Writes = Arc::default();
        let sink = writes.clone();
        let mut options =
            ImageLayerOptions::new(capabilities(graphics, file_medium), service, move |data| {
                sink.lock().unwrap().push(data.to_string())
            });
        options.image_id_base = Some(BASE);
        (ImageLayer::new(options), writes)
    }

    fn kitty_layer(service: Arc<FakeService>) -> (ImageLayer, Writes) {
        layer_with(GraphicsProtocol::Kitty, false, service)
    }

    /// Waits for the image to be prepared, then for the flush that readiness
    /// schedules (the TS `setImmediate`) to have written `writes` entries.
    async fn until_flushed(layer: &ImageLayer, id: u32, writes: &Writes, count: usize) -> String {
        for _ in 0..400 {
            if layer.prepared(id).is_some() && writes.lock().unwrap().len() >= count {
                return writes.lock().unwrap()[count - 1].clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("image never became ready and flushed");
    }

    const DELETE_ALL_PLACEMENTS: &str = "\x1b_Ga=d,d=a,q=2\x1b\\";
    const TRANSMIT: &str = "\x1b_Ga=t,i=5000,f=100,q=2,t=d,m=0;AQID\x1b\\";

    fn boxed(x: u32, y: u32, width: u32, height: u32) -> Option<CellBox> {
        Some(CellBox {
            x,
            y,
            width,
            height,
        })
    }

    #[tokio::test]
    async fn places_a_picture_centered_in_its_box_after_transmitting_it_once() {
        let service = FakeService::new(200, 100);
        let (layer, writes) = kitty_layer(service.clone());
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(2, 3, 10, 5));

        // First render: nothing is prepared yet, so only the resync delete goes out.
        assert_eq!(layer.render(), DELETE_ALL_PLACEMENTS);
        // The request is for the box in pixels (10x5 cells of 10x20).
        assert_eq!(
            service.requests(),
            vec![(
                "/a.png".to_string(),
                ImageSize {
                    width: 100,
                    height: 100
                },
                None
            )]
        );

        // Readiness flushes on its own, inside one synchronized update.
        // 200x100 source in a 100x100px box scales by 0.5: 100x50px = 10 cols x
        // round(2.5)=3 rows, centered vertically (row 3 + floor((5-3)/2)).
        let place = format!(
            "\x1b7\x1b[5;3H\x1b_Ga=p,i=5000,p={},c=10,r=3,z=0,C=1,q=2\x1b\\\x1b8",
            handle.id
        );
        assert_eq!(
            until_flushed(&layer, handle.id, &writes, 1).await,
            format!("\x1b[?2026h{TRANSMIT}{place}\x1b[?2026l")
        );
        // Same layout again: the picture is placed again but not retransmitted,
        // and no new request goes out.
        assert_eq!(layer.render(), place);
        assert_eq!(service.requests().len(), 1);
    }

    #[tokio::test]
    async fn offsets_placements_by_the_live_region_origin() {
        let (layer, writes) = kitty_layer(FakeService::new(100, 100));
        layer.set_origin(10, 4);
        let handle = layer.register(Some("/a.png".into()), None, Some(2), None, None);
        handle.set_node(boxed(0, 0, 5, 5));
        layer.render();
        let output = until_flushed(&layer, handle.id, &writes, 1).await;
        assert!(output.contains("\x1b[12;5H\x1b_Ga=p,i=5000,"), "{output:?}");
        assert!(output.contains(",z=2,C=1"), "{output:?}");
    }

    #[tokio::test]
    async fn transmits_by_file_path_only_for_untouched_originals_when_the_terminal_allows_it() {
        let service = Arc::new(FakeService {
            source: ImageSize {
                width: 100,
                height: 100,
            },
            is_original: true,
            fail: false,
            requests: Mutex::new(Vec::new()),
        });
        let (layer, writes) = layer_with(GraphicsProtocol::Kitty, true, service);
        let handle = layer.register(Some("/orig.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 4, 2));
        layer.render();
        let output = until_flushed(&layer, handle.id, &writes, 1).await;
        // base64("/orig.png")
        assert!(
            output.starts_with("\x1b[?2026h\x1b_Ga=t,i=5000,f=100,q=2,t=f;L29yaWcucG5n\x1b\\"),
            "{output:?}"
        );
    }

    #[tokio::test]
    async fn never_upscales_past_max_upscale() {
        // 10x10 source in a 100x100px box: native size is 1x1 cells, 3x fill is 3x2 cells.
        let (layer, writes) = kitty_layer(FakeService::new(10, 10));
        let handle = layer.register(Some("/s.png".into()), None, None, Some(3.0), None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.render();
        let output = until_flushed(&layer, handle.id, &writes, 1).await;
        assert!(output.contains(",c=3,r=2,"), "{output:?}");
        handle.set_view(ImageView {
            max_upscale: Some(1.0),
            ..ImageView::default()
        });
        let output = layer.render();
        assert!(output.contains(",c=1,r=1,"), "{output:?}");
    }

    #[tokio::test]
    async fn zooming_requests_a_centered_crop_and_pan_slides_it() {
        let service = FakeService::new(200, 100);
        let (layer, writes) = kitty_layer(service.clone());
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.render();
        until_flushed(&layer, handle.id, &writes, 1).await;
        handle.set_view(ImageView {
            zoom: Some(2.0),
            pan_x: Some(0.0),
            ..ImageView::default()
        });
        layer.render();
        let requests = service.requests();
        let crop = requests.last().unwrap().2.unwrap();
        // Half the source: panned to the left edge (center clamped to cropW/2)
        // and centered vertically.
        assert_eq!(
            crop,
            Rect {
                x: 0.0,
                y: 25.0,
                width: 100.0,
                height: 50.0
            }
        );
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn resize_invalidates_and_resends_every_placement_without_retransmitting() {
        let service = FakeService::new(100, 100);
        let (layer, writes) = kitty_layer(service.clone());
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.render();
        until_flushed(&layer, handle.id, &writes, 1).await;

        // Screen cleared: delete everything, place again, no second transmit.
        layer.invalidate();
        let output = layer.render();
        assert!(output.starts_with(DELETE_ALL_PLACEMENTS), "{output:?}");
        assert!(!output.contains("a=t"), "{output:?}");
        assert!(output.contains("a=p,i=5000"), "{output:?}");

        // The box shrinks: the layout moves the picture and a new size is requested.
        handle.set_node(boxed(1, 1, 4, 2));
        layer.render();
        assert_eq!(
            service.requests().last().unwrap().1,
            ImageSize {
                width: 40,
                height: 40
            }
        );
        let output = until_flushed(&layer, handle.id, &writes, 2).await;
        assert!(output.contains("\x1b[2;2H"), "{output:?}");
        assert!(output.contains("a=t,i=5001"), "{output:?}");
    }

    #[tokio::test]
    async fn removes_the_placement_of_an_unregistered_picture_on_the_next_render() {
        let (layer, writes) = kitty_layer(FakeService::new(100, 100));
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.render();
        until_flushed(&layer, handle.id, &writes, 1).await;
        handle.unregister();
        assert_eq!(
            layer.render(),
            format!("\x1b_Ga=d,d=i,i=5000,p={},q=2\x1b\\", handle.id)
        );
        assert_eq!(layer.render(), "");
    }

    #[tokio::test]
    async fn clear_frees_every_image_and_forces_a_resync() {
        let (layer, writes) = kitty_layer(FakeService::new(100, 100));
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.render();
        until_flushed(&layer, handle.id, &writes, 1).await;
        assert_eq!(layer.clear(), "\x1b_Ga=d,d=A,q=2\x1b\\");
        // The next render transmits again because the terminal forgot the data.
        let output = layer.render();
        assert!(output.starts_with("\x1b_Ga=t,i=5001"), "{output:?}");
        assert!(output.contains(DELETE_ALL_PLACEMENTS), "{output:?}");
    }

    #[tokio::test]
    async fn evicts_the_least_recently_used_unplaced_images_over_the_cap() {
        let service = FakeService::new(100, 100);
        let writes: Writes = Arc::default();
        let sink = writes.clone();
        let mut options = ImageLayerOptions::new(
            capabilities(GraphicsProtocol::Kitty, false),
            service,
            move |data| sink.lock().unwrap().push(data.to_string()),
        );
        options.image_id_base = Some(BASE);
        options.max_transmitted = Some(1);
        let layer = ImageLayer::new(options);
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.render();
        until_flushed(&layer, handle.id, &writes, 1).await;
        // A new source replaces the transmitted image; the old id is deleted
        // once the new one is placed.
        handle.set_source(Some("/b.png".into()), 0.0);
        let output = until_flushed(&layer, handle.id, &writes, 3).await;
        assert!(output.contains("a=t,i=5001"), "{output:?}");
        assert!(
            output.ends_with("\x1b_Ga=d,d=I,i=5000,q=2\x1b\\\x1b[?2026l"),
            "{output:?}"
        );
    }

    #[tokio::test]
    async fn disabled_layers_render_nothing() {
        let service = FakeService::new(100, 100);
        let (layer, writes) = layer_with(GraphicsProtocol::Halfblocks, false, service.clone());
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        assert!(!layer.enabled());
        assert_eq!(layer.render(), "");
        assert_eq!(layer.clear(), "");
        layer.flush_now();
        assert!(writes.lock().unwrap().is_empty());
        assert!(service.requests().is_empty());
    }

    #[tokio::test]
    async fn reports_failed_preparation_and_skips_the_picture() {
        let service = Arc::new(FakeService {
            source: ImageSize {
                width: 1,
                height: 1,
            },
            is_original: false,
            fail: true,
            requests: Mutex::new(Vec::new()),
        });
        let errors: Arc<Mutex<Vec<String>>> = Arc::default();
        let seen = errors.clone();
        let mut options = ImageLayerOptions::new(
            capabilities(GraphicsProtocol::Kitty, false),
            service,
            |_| {},
        );
        options.on_error = Some(Box::new(move |src, error| {
            seen.lock().unwrap().push(format!("{src}: {error}"));
        }));
        let layer = ImageLayer::new(options);
        let handle = layer.register(Some("/bad.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 4, 2));
        layer.render();
        for _ in 0..200 {
            if layer.failure(handle.id).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(layer.failure(handle.id).as_deref(), Some("decode failed"));
        assert_eq!(*errors.lock().unwrap(), vec!["/bad.png: decode failed"]);
        assert_eq!(layer.render(), "");
    }

    #[tokio::test]
    async fn flush_writes_the_placements_inside_a_synchronized_update() {
        let (layer, writes) = kitty_layer(FakeService::new(100, 100));
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.flush_now();
        assert_eq!(
            *writes.lock().unwrap(),
            vec![format!("\x1b[?2026h{DELETE_ALL_PLACEMENTS}\x1b[?2026l")]
        );
        // Readiness schedules a flush on its own.
        until_flushed(&layer, handle.id, &writes, 2).await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 2);
        assert!(writes[1].starts_with("\x1b[?2026h\x1b_Ga=t,i=5000"));
        assert!(writes[1].ends_with("\x1b8\x1b[?2026l"));
    }

    #[tokio::test]
    async fn frames_are_transmitted_and_placed_immediately() {
        let (layer, writes) = kitty_layer(FakeService::new(1, 1));
        let frames = layer.register_frames(None);
        frames.set_node(boxed(1, 1, 10, 5));
        // 100x100 frame in a 100x100px box: 10 cols x 5 rows.
        frames.push_frame(vec![1, 2, 3], 100, 100);
        assert_eq!(
            *writes.lock().unwrap(),
            vec![format!(
                "\x1b[?2026h\x1b_Ga=t,i=5000,f=100,q=2,t=d,m=0;AQID\x1b\\\x1b7\x1b[2;2H\x1b_Ga=p,i=5000,p={},c=10,r=5,z=1,C=1,q=2\x1b\\\x1b8\x1b[?2026l",
                frames.id
            )]
        );
        // Each frame gets a fresh image id.
        frames.push_frame(vec![1, 2, 3], 100, 100);
        assert!(writes.lock().unwrap()[1].contains("i=5001,f=100"));
        // The kitty path re-emits the frame placement on a UI render.
        let output = layer.render();
        assert!(output.contains("a=p,i=5001"), "{output:?}");
    }

    #[tokio::test]
    async fn clearing_a_frame_drops_its_placement_without_writing_a_delete() {
        let (layer, writes) = kitty_layer(FakeService::new(1, 1));
        let frames = layer.register_frames(Some(3));
        frames.set_node(boxed(0, 0, 10, 5));
        frames.push_frame(vec![1, 2, 3], 100, 100);
        let before = writes.lock().unwrap().len();
        frames.clear_frame();
        // Matches the TS: dropFrame() runs after the frame was nulled.
        assert_eq!(writes.lock().unwrap().len(), before);
        // The stale placement is deleted on the next render.
        assert_eq!(layer.render(), format!("{DELETE_ALL_PLACEMENTS}"));
        frames.unregister();
        assert_eq!(writes.lock().unwrap().len(), before);
    }

    #[tokio::test]
    async fn frames_do_nothing_when_graphics_are_off() {
        let (layer, writes) = layer_with(GraphicsProtocol::None, false, FakeService::new(1, 1));
        let frames = layer.register_frames(None);
        frames.set_node(boxed(0, 0, 10, 5));
        frames.push_frame(vec![1], 10, 10);
        assert!(writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn debug_summary_is_reported_only_when_it_changes() {
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = seen.clone();
        let mut options = ImageLayerOptions::new(
            capabilities(GraphicsProtocol::Kitty, false),
            FakeService::new(1, 1),
            |_| {},
        );
        options.on_debug = Some(Box::new(move |message| {
            sink.lock().unwrap().push(message.to_string())
        }));
        let layer = ImageLayer::new(options);
        layer.render();
        layer.render();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "entries=0 placed=0 transmitted=0 bytes=16".to_string(),
                "entries=0 placed=0 transmitted=0 bytes=0".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn graphics_stdout_splices_placements_into_frames_and_homes_the_alt_screen() {
        let (layer, writes) = kitty_layer(FakeService::new(100, 100));
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        layer.render();
        until_flushed(&layer, handle.id, &writes, 1).await;

        let mut out = create_graphics_stdout(Vec::<u8>::new(), layer.clone());
        // Control-only writes pass through untouched.
        out.write_all(b"\x1b[?2026h").unwrap();
        // Entering the alternate screen homes and clears, and invalidates the layer.
        out.write_all(b"\x1b[?1049h").unwrap();
        // A frame with text gets the resync and placements before the end of
        // the sync block.
        out.write_all("hello\x1b[?2026l".as_bytes()).unwrap();
        let written = String::from_utf8(out.into_inner()).unwrap();
        assert_eq!(
            written,
            format!(
                "\x1b[?2026h\x1b[?1049h\x1b[H\x1b[2Jhello{DELETE_ALL_PLACEMENTS}\x1b7\x1b[1;1H\x1b_Ga=p,i=5000,p={},c=10,r=5,z=0,C=1,q=2\x1b\\\x1b8\x1b[?2026l",
                handle.id
            )
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn herdr_places_through_one_stream_per_layer_instead_of_kitty_escapes() {
        use crate::astroshot_review::terminal::herdr::HerdrAddress;
        use tokio::io::AsyncReadExt;
        use tokio::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("api.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let accepted = tokio::spawn(async move { listener.accept().await.unwrap().0 });
        let sink = HerdrSink::new(
            HerdrAddress {
                socket: socket.to_string_lossy().into_owned(),
                pane: "w1:p2".to_string(),
            },
            |_| {},
        );
        let writes: Writes = Arc::default();
        let captured = writes.clone();
        let mut options = ImageLayerOptions::new(
            capabilities(GraphicsProtocol::Halfblocks, false),
            FakeService::new(100, 100),
            move |data| captured.lock().unwrap().push(data.to_string()),
        );
        options.herdr = Some(sink);
        options.image_id_base = Some(BASE);
        let layer = ImageLayer::new(options);
        // herdr alone enables the layer, whatever the kitty capability says.
        assert!(layer.enabled());
        let handle = layer.register(Some("/a.png".into()), None, None, None, None);
        handle.set_node(boxed(0, 0, 10, 5));
        assert_eq!(layer.render(), "");
        for _ in 0..400 {
            if layer.prepared(handle.id).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        // herdr oversamples 2x, and no kitty bytes are produced on any path.
        assert_eq!(layer.render(), "");
        assert!(writes.lock().unwrap().iter().all(|w| w.is_empty()));

        // The layer opened a stream connection for its picture...
        let mut stream = tokio::time::timeout(std::time::Duration::from_secs(3), accepted)
            .await
            .expect("herdr stream opened")
            .unwrap();
        // ...and unregistering closes it, which is how herdr drops the layer.
        handle.unregister();
        let mut buf = [0u8; 65536];
        let closed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while stream.read(&mut buf).await.unwrap_or(0) > 0 {}
        })
        .await;
        assert!(closed.is_ok(), "stream should be closed");
    }
}
