//! Port of `packages/astroshot-review/src/images/service.ts`.
//!
//! Prepares image bytes for the terminal: reads the file, decides whether the
//! original PNG is already small enough, otherwise downsamples through
//! [`run_job_async`], and caches the result by file identity and target size.
//!
//! Divergences from TS:
//! - The TS class is [`ImageServiceImpl`]; [`ImageService`] is the trait
//!   `image_layer` consumes (declared types-first).
//! - The worker pool is `images::worker::run_job_async` (tokio blocking pool).
//!   `workers` only bounds how many jobs run at once (semaphore); 0 runs the
//!   scale inline on the calling task, like the TS test mode.
//! - In-flight dedupe is a `Shared` future per cache key. A detached task
//!   also drives it so bookkeeping happens even if every caller drops.
//! - A failed or still-pending entry that gets evicted never subtracts bytes
//!   it did not add (TS subtracts on resolve regardless).
//! - `dispose` has no threads to terminate; it only clears the cache.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use anyhow::anyhow;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::sync::Semaphore;

use crate::astroshot_review::images::png::{ImageSize, fit_inside, read_png_size};
use crate::astroshot_review::images::scale::{Rect, ScaleRequest, ScaledFormat, scale_image};
use crate::astroshot_review::images::worker::{WorkerRequest, run_job_async};

/// Result of `ImageService::prepare` (`PreparedImage` in `images/service.ts`).
#[derive(Debug, Clone)]
pub struct PreparedImage {
    pub key: String,
    pub path: String,
    /// Pixel size of the prepared payload.
    pub width: u32,
    pub height: u32,
    /// Pixel size of the source file.
    pub source_width: u32,
    pub source_height: u32,
    pub format: ScaledFormat,
    pub data: Vec<u8>,
    /// True when `data` is the untouched file, so a file-path transmission is valid.
    pub is_original: bool,
    pub mtime_ms: f64,
    pub size: u64,
}

pub type PrepareFuture = Pin<Box<dyn Future<Output = anyhow::Result<Arc<PreparedImage>>> + Send>>;

/// `ImageService.prepare(filePath, target, format, crop)`.
pub trait ImageService: Send + Sync {
    fn prepare(
        &self,
        file_path: String,
        target: ImageSize,
        format: ScaledFormat,
        crop: Option<Rect>,
    ) -> PrepareFuture;
}

/// `ImageServiceOptions`.
#[derive(Debug, Clone, Default)]
pub struct ImageServiceOptions {
    /// 0 runs decode inline (tests); default is min(2, cpus-1).
    pub workers: Option<usize>,
    /// Memory budget for prepared payloads.
    pub cache_bytes: Option<usize>,
    /// A source at most this many times larger than the target is sent as-is.
    pub passthrough_ratio: Option<f64>,
}

pub const DEFAULT_CACHE_BYTES: usize = 96 * 1024 * 1024;
pub const DEFAULT_PASSTHROUGH_RATIO: f64 = 1.35;

/// File identity used in cache keys (`fs.Stats` subset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileStat {
    pub mtime_ms: f64,
    pub size: u64,
}

type JobResult = Result<Arc<PreparedImage>, Arc<str>>;
type Job = Shared<BoxFuture<'static, JobResult>>;

struct Entry {
    id: u64,
    job: Job,
    accounted: Option<usize>,
}

#[derive(Default)]
struct State {
    cache: HashMap<String, Entry>,
    /// Least recently used first.
    order: Vec<String>,
    cached_bytes: usize,
}

struct Inner {
    state: Mutex<State>,
    cache_bytes: usize,
    passthrough_ratio: f64,
    worker_count: usize,
    permits: Semaphore,
    next_entry: AtomicU64,
    scale_jobs: AtomicUsize,
    disposed: AtomicBool,
}

/// The caching image preparer (`ImageService` class in TS).
#[derive(Clone)]
pub struct ImageServiceImpl {
    inner: Arc<Inner>,
}

impl Default for ImageServiceImpl {
    fn default() -> Self {
        Self::new(ImageServiceOptions::default())
    }
}

fn js_round(value: f64) -> i64 {
    (value + 0.5).floor() as i64
}

fn format_name(format: ScaledFormat) -> &'static str {
    match format {
        ScaledFormat::Png => "png",
        ScaledFormat::Rgb => "rgb",
    }
}

impl ImageServiceImpl {
    pub fn new(options: ImageServiceOptions) -> Self {
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        let worker_count = options.workers.unwrap_or_else(|| 2.min(cpus - 1));
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                cache_bytes: options.cache_bytes.unwrap_or(DEFAULT_CACHE_BYTES),
                passthrough_ratio: options
                    .passthrough_ratio
                    .unwrap_or(DEFAULT_PASSTHROUGH_RATIO),
                worker_count,
                permits: Semaphore::new(worker_count.max(1)),
                next_entry: AtomicU64::new(1),
                scale_jobs: AtomicUsize::new(0),
                disposed: AtomicBool::new(false),
            }),
        }
    }

    /// Identity + target size key. Callers can use it for placement bookkeeping.
    pub fn cache_key(
        file_path: &str,
        stat: FileStat,
        target: ImageSize,
        format: ScaledFormat,
        crop: Option<Rect>,
    ) -> String {
        let crop_key = crop.map_or_else(String::new, |c| {
            format!(
                "|{},{},{},{}",
                js_round(c.x),
                js_round(c.y),
                js_round(c.width),
                js_round(c.height)
            )
        });
        format!(
            "{file_path}|{}|{}|{}x{}|{}{crop_key}",
            js_round(stat.mtime_ms),
            stat.size,
            target.width,
            target.height,
            format_name(format)
        )
    }

    /// Number of downsample jobs actually run (passthroughs and cache hits excluded).
    pub fn scale_job_count(&self) -> usize {
        self.inner.scale_jobs.load(Ordering::SeqCst)
    }

    /// Bytes currently accounted against the cache budget.
    pub fn cached_bytes(&self) -> usize {
        self.inner.state.lock().unwrap().cached_bytes
    }

    /// Number of cache entries (pending included).
    pub fn cache_len(&self) -> usize {
        self.inner.state.lock().unwrap().cache.len()
    }

    /// `ImageService.prepare`. The stat happens when the future is first polled.
    pub async fn prepare_image(
        &self,
        file_path: String,
        target: ImageSize,
        format: ScaledFormat,
        crop: Option<Rect>,
    ) -> anyhow::Result<Arc<PreparedImage>> {
        let metadata = tokio::fs::metadata(&file_path)
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let stat = FileStat {
            mtime_ms,
            size: metadata.len(),
        };
        let key = Self::cache_key(&file_path, stat, target, format, crop);

        let (id, job) = {
            let mut state = self.inner.state.lock().unwrap();
            if let Some(entry) = state.cache.get(&key) {
                let (id, job) = (entry.id, entry.job.clone());
                touch(&mut state, &key);
                (id, job)
            } else {
                let id = self.inner.next_entry.fetch_add(1, Ordering::SeqCst);
                let inner = self.inner.clone();
                let build_key = key.clone();
                let job: Job = async move {
                    build(&inner, file_path, stat, target, format, build_key, crop)
                        .await
                        .map(Arc::new)
                        .map_err(|error| Arc::<str>::from(error.to_string()))
                }
                .boxed()
                .shared();
                state.cache.insert(
                    key.clone(),
                    Entry {
                        id,
                        job: job.clone(),
                        accounted: None,
                    },
                );
                state.order.push(key.clone());
                // Drive the job even if every caller drops, so the cache
                // never holds a stalled future.
                let driver = job.clone();
                let inner = self.inner.clone();
                let driver_key = key.clone();
                tokio::spawn(async move {
                    let result = driver.await;
                    finish(&inner, &driver_key, id, &result);
                });
                (id, job)
            }
        };
        let result = job.await;
        finish(&self.inner, &key, id, &result);
        result.map_err(|message| anyhow!(message.to_string()))
    }

    /// Drops every cached payload. Later `prepare` calls still work.
    pub fn dispose(&self) {
        if self.inner.disposed.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut state = self.inner.state.lock().unwrap();
        state.cache.clear();
        state.order.clear();
        state.cached_bytes = 0;
    }
}

impl ImageService for ImageServiceImpl {
    fn prepare(
        &self,
        file_path: String,
        target: ImageSize,
        format: ScaledFormat,
        crop: Option<Rect>,
    ) -> PrepareFuture {
        let service = self.clone();
        Box::pin(async move { service.prepare_image(file_path, target, format, crop).await })
    }
}

async fn build(
    inner: &Inner,
    file_path: String,
    stat: FileStat,
    target: ImageSize,
    format: ScaledFormat,
    key: String,
    crop: Option<Rect>,
) -> anyhow::Result<PreparedImage> {
    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let source = read_png_size(&bytes).ok_or_else(|| anyhow!("Not a PNG image: {file_path}"))?;
    if crop.is_none() {
        let fitted = fit_inside(source, target);
        let ratio = (f64::from(source.width) / f64::from(fitted.width))
            .max(f64::from(source.height) / f64::from(fitted.height));
        if format == ScaledFormat::Png && ratio <= inner.passthrough_ratio {
            return Ok(PreparedImage {
                key,
                path: file_path,
                width: source.width,
                height: source.height,
                source_width: source.width,
                source_height: source.height,
                format: ScaledFormat::Png,
                data: bytes,
                is_original: true,
                mtime_ms: stat.mtime_ms,
                size: stat.size,
            });
        }
    }
    let scaled = scale(inner, bytes, target, format, crop).await?;
    Ok(PreparedImage {
        key,
        path: file_path,
        width: scaled.width,
        height: scaled.height,
        source_width: source.width,
        source_height: source.height,
        format: scaled.format,
        data: scaled.data,
        is_original: false,
        mtime_ms: stat.mtime_ms,
        size: stat.size,
    })
}

struct Scaled {
    width: u32,
    height: u32,
    format: ScaledFormat,
    data: Vec<u8>,
}

async fn scale(
    inner: &Inner,
    bytes: Vec<u8>,
    target: ImageSize,
    format: ScaledFormat,
    crop: Option<Rect>,
) -> anyhow::Result<Scaled> {
    inner.scale_jobs.fetch_add(1, Ordering::SeqCst);
    if inner.worker_count == 0 {
        let scaled = scale_image(&ScaleRequest {
            bytes: &bytes,
            target,
            format,
            crop,
        })?;
        return Ok(Scaled {
            width: scaled.width,
            height: scaled.height,
            format: scaled.format,
            data: scaled.data,
        });
    }
    let _permit = inner
        .permits
        .acquire()
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let response = run_job_async(WorkerRequest {
        bytes,
        target_width: target.width,
        target_height: target.height,
        format,
        crop,
    })
    .await
    .map_err(|error| anyhow!(error.error))?;
    Ok(Scaled {
        width: response.width,
        height: response.height,
        format: response.format,
        data: response.data,
    })
}

fn touch(state: &mut State, key: &str) {
    if let Some(index) = state.order.iter().position(|k| k == key) {
        let moved = state.order.remove(index);
        state.order.push(moved);
    }
}

/// Idempotent bookkeeping once a job settles: account bytes on success,
/// evict on failure. Ignores entries that were already replaced or cleared.
fn finish(inner: &Inner, key: &str, id: u64, result: &JobResult) {
    let mut state = inner.state.lock().unwrap();
    let Some(entry) = state.cache.get_mut(key) else {
        return;
    };
    if entry.id != id || entry.accounted.is_some() {
        return;
    }
    match result {
        Ok(prepared) => {
            entry.accounted = Some(prepared.data.len());
            state.cached_bytes += prepared.data.len();
            while state.cached_bytes > inner.cache_bytes && state.order.len() > 1 {
                let oldest = state.order[0].clone();
                if oldest == key {
                    break;
                }
                evict(&mut state, &oldest);
            }
        }
        Err(_) => evict(&mut state, key),
    }
}

fn evict(state: &mut State, key: &str) {
    let Some(entry) = state.cache.remove(key) else {
        return;
    };
    if let Some(index) = state.order.iter().position(|k| k == key) {
        state.order.remove(index);
    }
    if let Some(bytes) = entry.accounted {
        state.cached_bytes -= bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use std::path::Path;
    use std::time::Duration;

    fn solid_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let image = ImageBuffer::from_pixel(width, height, Rgba(rgba));
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn write_png(dir: &Path, name: &str, width: u32, height: u32) -> String {
        let path = dir.join(name);
        std::fs::write(&path, solid_png(width, height, [10, 20, 30, 255])).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn size(width: u32, height: u32) -> ImageSize {
        ImageSize { width, height }
    }

    fn inline() -> ImageServiceImpl {
        ImageServiceImpl::new(ImageServiceOptions {
            workers: Some(0),
            ..Default::default()
        })
    }

    fn set_mtime(path: &str, secs: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn cache_key_rounds_mtime_and_crop() {
        let stat = FileStat {
            mtime_ms: 1234.6,
            size: 9,
        };
        let key =
            ImageServiceImpl::cache_key("/a.png", stat, size(10, 20), ScaledFormat::Png, None);
        assert_eq!(key, "/a.png|1235|9|10x20|png");
        let crop = Rect {
            x: 1.4,
            y: 2.5,
            width: 3.6,
            height: 4.0,
        };
        let key = ImageServiceImpl::cache_key(
            "/a.png",
            stat,
            size(10, 20),
            ScaledFormat::Rgb,
            Some(crop),
        );
        assert_eq!(key, "/a.png|1235|9|10x20|rgb|1,3,4,4");
    }

    #[tokio::test]
    async fn passes_a_small_enough_png_through_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 40, 20);
        let service = inline();
        // 40x20 into 32x32 fits at ratio 1.25, under the 1.35 passthrough.
        let prepared = service
            .prepare_image(path.clone(), size(32, 32), ScaledFormat::Png, None)
            .await
            .unwrap();
        assert!(prepared.is_original);
        assert_eq!((prepared.width, prepared.height), (40, 20));
        assert_eq!((prepared.source_width, prepared.source_height), (40, 20));
        assert_eq!(prepared.data, std::fs::read(&path).unwrap());
        assert_eq!(prepared.path, path);
        assert_eq!(prepared.size, prepared.data.len() as u64);
        assert_eq!(service.scale_job_count(), 0);
    }

    #[tokio::test]
    async fn downsamples_a_large_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 100, 50);
        let service = inline();
        let prepared = service
            .prepare_image(path, size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        assert!(!prepared.is_original);
        assert_eq!((prepared.width, prepared.height), (20, 10));
        assert_eq!((prepared.source_width, prepared.source_height), (100, 50));
        assert_eq!(prepared.format, ScaledFormat::Png);
        assert_eq!(service.scale_job_count(), 1);
    }

    #[tokio::test]
    async fn rgb_format_never_passes_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 10, 10);
        let service = inline();
        let prepared = service
            .prepare_image(path, size(10, 10), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        assert!(!prepared.is_original);
        assert_eq!(prepared.format, ScaledFormat::Rgb);
        assert_eq!(prepared.data.len(), 10 * 10 * 3);
    }

    #[tokio::test]
    async fn crop_always_scales_and_applies_the_rect() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 40, 40);
        let service = inline();
        let crop = Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 10.0,
        };
        let prepared = service
            .prepare_image(path, size(100, 100), ScaledFormat::Png, Some(crop))
            .await
            .unwrap();
        assert!(!prepared.is_original);
        assert_eq!((prepared.width, prepared.height), (20, 10));
        assert_eq!((prepared.source_width, prepared.source_height), (40, 40));
    }

    #[tokio::test]
    async fn variants_get_separate_cache_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 100, 100);
        let service = inline();
        let crop = Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        };
        let a = service
            .prepare_image(path.clone(), size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        let b = service
            .prepare_image(path.clone(), size(30, 30), ScaledFormat::Png, None)
            .await
            .unwrap();
        let c = service
            .prepare_image(path.clone(), size(20, 20), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        let d = service
            .prepare_image(path.clone(), size(20, 20), ScaledFormat::Png, Some(crop))
            .await
            .unwrap();
        let keys: std::collections::HashSet<_> =
            [&a, &b, &c, &d].iter().map(|p| p.key.clone()).collect();
        assert_eq!(keys.len(), 4);
        assert_eq!(service.cache_len(), 4);
        assert_eq!(service.scale_job_count(), 4);
        assert_eq!((b.width, b.height), (30, 30));
    }

    #[tokio::test]
    async fn repeat_prepare_is_a_cache_hit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 100, 100);
        let service = inline();
        let first = service
            .prepare_image(path.clone(), size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        let second = service
            .prepare_image(path, size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(service.scale_job_count(), 1);
    }

    #[tokio::test]
    async fn mtime_change_misses_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 100, 100);
        set_mtime(&path, 1_000_000);
        let service = inline();
        let first = service
            .prepare_image(path.clone(), size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        set_mtime(&path, 2_000_000);
        let second = service
            .prepare_image(path, size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_ne!(first.key, second.key);
        assert_eq!(second.mtime_ms, 2_000_000_000.0);
        assert_eq!(service.scale_job_count(), 2);
    }

    #[tokio::test]
    async fn size_change_misses_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 100, 100);
        set_mtime(&path, 1_000_000);
        let service = inline();
        let first = service
            .prepare_image(path.clone(), size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        // Different content (so a different byte length), same mtime.
        std::fs::write(&path, solid_png(200, 200, [200, 100, 50, 255])).unwrap();
        set_mtime(&path, 1_000_000);
        let second = service
            .prepare_image(path, size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        assert_ne!(first.size, second.size);
        assert_eq!((second.source_width, second.source_height), (200, 200));
        assert_eq!(service.scale_job_count(), 2);
    }

    #[tokio::test]
    async fn concurrent_prepare_of_one_key_shares_one_job() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 300, 300);
        let service = ImageServiceImpl::new(ImageServiceOptions {
            workers: Some(2),
            ..Default::default()
        });
        let calls = (0..8).map(|_| {
            let service = service.clone();
            let path = path.clone();
            tokio::spawn(async move {
                service
                    .prepare(path, size(30, 30), ScaledFormat::Png, None)
                    .await
                    .unwrap()
            })
        });
        let results = futures::future::join_all(calls).await;
        let first = results[0].as_ref().unwrap();
        for result in &results {
            assert!(Arc::ptr_eq(first, result.as_ref().unwrap()));
        }
        assert_eq!(service.scale_job_count(), 1);
        assert_eq!(service.cache_len(), 1);
    }

    #[tokio::test]
    async fn worker_pool_output_matches_inline() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 90, 60);
        let pooled = ImageServiceImpl::new(ImageServiceOptions {
            workers: Some(1),
            ..Default::default()
        });
        let a = pooled
            .prepare_image(path.clone(), size(30, 30), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        let b = inline()
            .prepare_image(path, size(30, 30), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        assert_eq!(a.data, b.data);
        assert_eq!((a.width, a.height), (b.width, b.height));
    }

    #[tokio::test]
    async fn rejects_a_file_that_is_not_a_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, b"definitely not a png").unwrap();
        let path = path.to_string_lossy().into_owned();
        let service = inline();
        let error = service
            .prepare_image(path.clone(), size(10, 10), ScaledFormat::Png, None)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), format!("Not a PNG image: {path}"));
    }

    #[tokio::test]
    async fn rejects_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("missing.png")
            .to_string_lossy()
            .into_owned();
        let error = inline()
            .prepare_image(path, size(10, 10), ScaledFormat::Png, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("No such file"), "{error}");
    }

    #[tokio::test]
    async fn propagates_scale_errors_and_evicts_the_failed_entry() {
        let dir = tempfile::tempdir().unwrap();
        // Valid IHDR, truncated body: passes readPngSize, fails decode.
        let mut broken = solid_png(100, 100, [1, 2, 3, 255]);
        broken.truncate(40);
        let path = dir.path().join("a.png");
        std::fs::write(&path, &broken).unwrap();
        let path = path.to_string_lossy().into_owned();
        let service = ImageServiceImpl::new(ImageServiceOptions {
            workers: Some(1),
            ..Default::default()
        });
        let first = service
            .prepare_image(path.clone(), size(20, 20), ScaledFormat::Png, None)
            .await;
        assert!(first.is_err());
        assert_eq!(service.cache_len(), 0);
        assert_eq!(service.cached_bytes(), 0);
        // A retry runs a new job instead of replaying the cached failure.
        let second = service
            .prepare_image(path, size(20, 20), ScaledFormat::Png, None)
            .await;
        assert!(second.is_err());
        assert_eq!(service.scale_job_count(), 2);
    }

    #[tokio::test]
    async fn concurrent_callers_all_see_the_same_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, b"nope").unwrap();
        let path = path.to_string_lossy().into_owned();
        let service = inline();
        let (a, b) = futures::join!(
            service.prepare_image(path.clone(), size(10, 10), ScaledFormat::Png, None),
            service.prepare_image(path.clone(), size(10, 10), ScaledFormat::Png, None),
        );
        let expected = format!("Not a PNG image: {path}");
        assert_eq!(a.unwrap_err().to_string(), expected);
        assert_eq!(b.unwrap_err().to_string(), expected);
    }

    #[tokio::test]
    async fn evicts_the_least_recently_used_entries_over_budget() {
        let dir = tempfile::tempdir().unwrap();
        let service = ImageServiceImpl::new(ImageServiceOptions {
            workers: Some(0),
            // Each 10x10 rgb payload is 300 bytes; room for two.
            cache_bytes: Some(700),
            ..Default::default()
        });
        let mut prepared = Vec::new();
        for name in ["a.png", "b.png", "c.png"] {
            let path = write_png(dir.path(), name, 10, 10);
            prepared.push(
                service
                    .prepare_image(path, size(10, 10), ScaledFormat::Rgb, None)
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(service.cache_len(), 2);
        assert_eq!(service.cached_bytes(), 600);
        // `a` was evicted, so it is rebuilt; `c` is still cached.
        let again_c = service
            .prepare_image(
                prepared[2].path.clone(),
                size(10, 10),
                ScaledFormat::Rgb,
                None,
            )
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&prepared[2], &again_c));
        let again_a = service
            .prepare_image(
                prepared[0].path.clone(),
                size(10, 10),
                ScaledFormat::Rgb,
                None,
            )
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&prepared[0], &again_a));
    }

    #[tokio::test]
    async fn touching_an_entry_protects_it_from_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let service = ImageServiceImpl::new(ImageServiceOptions {
            workers: Some(0),
            cache_bytes: Some(700),
            ..Default::default()
        });
        let a_path = write_png(dir.path(), "a.png", 10, 10);
        let b_path = write_png(dir.path(), "b.png", 10, 10);
        let c_path = write_png(dir.path(), "c.png", 10, 10);
        let a = service
            .prepare_image(a_path.clone(), size(10, 10), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        service
            .prepare_image(b_path, size(10, 10), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        // Re-read `a` so `b` becomes the oldest.
        service
            .prepare_image(a_path.clone(), size(10, 10), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        service
            .prepare_image(c_path, size(10, 10), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        let a_again = service
            .prepare_image(a_path, size(10, 10), ScaledFormat::Rgb, None)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&a, &a_again));
    }

    #[tokio::test]
    async fn dispose_clears_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 100, 100);
        let service = inline();
        service
            .prepare_image(path, size(20, 20), ScaledFormat::Png, None)
            .await
            .unwrap();
        assert_eq!(service.cache_len(), 1);
        service.dispose();
        service.dispose();
        assert_eq!(service.cache_len(), 0);
        assert_eq!(service.cached_bytes(), 0);
    }
}
