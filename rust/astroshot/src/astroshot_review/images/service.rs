//! Port of `packages/astroshot-review/src/images/service.ts`.
//!
//! Types first: `image_layer` needs `ImageService` and `PreparedImage`
//! before the service implementation (worker pool, cache) is ported.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::astroshot_review::images::png::ImageSize;
use crate::astroshot_review::images::scale::{Rect, ScaledFormat};

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
