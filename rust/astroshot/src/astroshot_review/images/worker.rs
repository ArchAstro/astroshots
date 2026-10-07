//! Port of `packages/astroshot-review/src/images/worker.ts`.
//!
//! The TS file is a `node:worker_threads` message loop: `{id, bytes, target,
//! format, crop}` in, `{id, ok, ...}` out. In Rust there are no worker
//! threads or messages; `ImageService` calls [`run_job`] (blocking) or
//! [`run_job_async`] (on tokio's blocking pool) and gets the same
//! success/error result back. The request ids existed only to match replies
//! to callers; futures make them unnecessary, so they are dropped, as is the
//! `ArrayBuffer` transfer.
//!
//! Divergence: `error` is the `anyhow` error's message (outermost context,
//! matching `Error.message`), not a serialized string.

use rayon::prelude::*;

use super::png::ImageSize;
use super::scale::{Rect, ScaleRequest, ScaledFormat, scale_image};

/// `WorkerRequest` minus `id`, with owned bytes so it can cross threads.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerRequest {
    pub bytes: Vec<u8>,
    pub target_width: u32,
    pub target_height: u32,
    pub format: ScaledFormat,
    pub crop: Option<Rect>,
}

/// `WorkerResponse`. The `ok: false` arm is `Err(WorkerError)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSuccess {
    pub width: u32,
    pub height: u32,
    pub format: ScaledFormat,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{error}")]
pub struct WorkerError {
    pub error: String,
}

pub type WorkerResponse = Result<WorkerSuccess, WorkerError>;

/// Handles one request, like the worker's `message` handler.
pub fn run_job(request: &WorkerRequest) -> WorkerResponse {
    let scaled = scale_image(&ScaleRequest {
        bytes: &request.bytes,
        target: ImageSize {
            width: request.target_width,
            height: request.target_height,
        },
        format: request.format,
        crop: request.crop,
    })
    .map_err(|error| WorkerError {
        error: error.to_string(),
    })?;
    Ok(WorkerSuccess {
        width: scaled.width,
        height: scaled.height,
        format: scaled.format,
        data: scaled.data,
    })
}

/// Runs one job on tokio's blocking pool so the async runtime stays free.
pub async fn run_job_async(request: WorkerRequest) -> WorkerResponse {
    match tokio::task::spawn_blocking(move || run_job(&request)).await {
        Ok(response) => response,
        Err(error) => Err(WorkerError {
            error: error.to_string(),
        }),
    }
}

/// Runs many jobs in parallel on rayon; results keep the request order.
pub fn run_jobs(requests: &[WorkerRequest]) -> Vec<WorkerResponse> {
    requests.par_iter().map(run_job).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::images::png::read_png_size;
    use image::{ImageBuffer, Rgba};

    fn solid_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let image = ImageBuffer::from_pixel(width, height, Rgba(rgba));
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn request(bytes: Vec<u8>, width: u32, height: u32, format: ScaledFormat) -> WorkerRequest {
        WorkerRequest {
            bytes,
            target_width: width,
            target_height: height,
            format,
            crop: None,
        }
    }

    #[test]
    fn scales_a_png_and_reports_its_size() {
        let response = run_job(&request(
            solid_png(40, 20, [10, 20, 30, 255]),
            10,
            10,
            ScaledFormat::Png,
        ))
        .unwrap();
        assert_eq!((response.width, response.height), (10, 5));
        assert_eq!(response.format, ScaledFormat::Png);
        let size = read_png_size(&response.data).unwrap();
        assert_eq!((size.width, size.height), (10, 5));
    }

    #[test]
    fn returns_packed_rgb_pixels() {
        let response = run_job(&request(
            solid_png(8, 8, [1, 2, 3, 255]),
            4,
            4,
            ScaledFormat::Rgb,
        ))
        .unwrap();
        assert_eq!(response.format, ScaledFormat::Rgb);
        assert_eq!(response.data.len(), 4 * 4 * 3);
        assert_eq!(&response.data[..3], &[1, 2, 3]);
    }

    #[test]
    fn applies_the_crop_before_scaling() {
        let mut job = request(
            solid_png(40, 40, [9, 9, 9, 255]),
            100,
            100,
            ScaledFormat::Png,
        );
        job.crop = Some(Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 10.0,
        });
        let response = run_job(&job).unwrap();
        // Crop is 20x10 and never upscales.
        assert_eq!((response.width, response.height), (20, 10));
    }

    #[test]
    fn reports_failures_as_an_error_response_instead_of_panicking() {
        let response = run_job(&request(b"not a png".to_vec(), 10, 10, ScaledFormat::Png));
        let error = response.unwrap_err();
        assert!(!error.error.is_empty());
    }

    #[tokio::test]
    async fn async_jobs_match_the_blocking_result() {
        let job = request(solid_png(30, 30, [5, 6, 7, 255]), 10, 10, ScaledFormat::Png);
        let expected = run_job(&job).unwrap();
        assert_eq!(run_job_async(job).await.unwrap(), expected);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_jobs_each_get_their_own_result() {
        let mut handles = Vec::new();
        for index in 1..=12u32 {
            // Distinct source sizes make each job's result identifiable.
            let size = 20 * index;
            let job = request(
                solid_png(size, size, [index as u8, 0, 0, 255]),
                10,
                10,
                ScaledFormat::Png,
            );
            handles.push((index, tokio::spawn(run_job_async(job))));
        }
        for (_, handle) in handles {
            let response = handle.await.unwrap().unwrap();
            assert_eq!((response.width, response.height), (10, 10));
        }
        // A failing job in the mix does not disturb the others.
        let (bad, good) = tokio::join!(
            run_job_async(request(vec![0, 1, 2], 10, 10, ScaledFormat::Png)),
            run_job_async(request(
                solid_png(20, 10, [0, 0, 0, 255]),
                10,
                10,
                ScaledFormat::Png
            )),
        );
        assert!(bad.is_err());
        assert_eq!((good.unwrap().width, 10), (10, 10));
    }

    #[test]
    fn run_jobs_keeps_request_order_across_the_rayon_pool() {
        let requests: Vec<WorkerRequest> = (1..=16u32)
            .map(|index| {
                if index.is_multiple_of(5) {
                    request(vec![0], 10, 10, ScaledFormat::Png)
                } else {
                    request(
                        solid_png(index * 10, 10, [0, 0, 0, 255]),
                        1000,
                        1000,
                        ScaledFormat::Png,
                    )
                }
            })
            .collect();
        let responses = run_jobs(&requests);
        assert_eq!(responses.len(), 16);
        for (offset, response) in responses.iter().enumerate() {
            let index = offset as u32 + 1;
            if index.is_multiple_of(5) {
                assert!(response.is_err());
            } else {
                assert_eq!(response.as_ref().unwrap().width, index * 10);
            }
        }
    }
}
