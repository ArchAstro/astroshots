//! Port of `packages/astroshot-review/src/images/png.ts`.

use std::path::Path;

use tokio::io::AsyncReadExt;

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

/// Read width/height from a PNG's IHDR chunk without decoding pixels.
pub fn read_png_size(bytes: &[u8]) -> Option<ImageSize> {
    if bytes.len() < 24 {
        return None;
    }
    if bytes[0..8] != PNG_SIGNATURE {
        return None;
    }
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    Some(ImageSize { width, height })
}

/// Read just enough of a file to learn its pixel size.
pub async fn read_png_size_from_file(file_path: &Path) -> std::io::Result<Option<ImageSize>> {
    let file = tokio::fs::File::open(file_path).await?;
    let mut header = Vec::with_capacity(32);
    file.take(32).read_to_end(&mut header).await?;
    Ok(read_png_size(&header))
}

pub fn is_png_path(file_path: &str) -> bool {
    file_path
        .get(file_path.len().saturating_sub(4)..)
        .is_some_and(|tail| tail.eq_ignore_ascii_case(".png"))
}

/// Largest box that fits inside `bounds` while keeping `source`'s aspect
/// ratio. Never scales up: a small source keeps its own size.
pub fn fit_inside(source: ImageSize, bounds: ImageSize) -> ImageSize {
    if source.width <= bounds.width && source.height <= bounds.height {
        return source;
    }
    let scale = (f64::from(bounds.width) / f64::from(source.width))
        .min(f64::from(bounds.height) / f64::from(source.height));
    ImageSize {
        width: ((f64::from(source.width) * scale).round() as u32).max(1),
        height: ((f64::from(source.height) * scale).round() as u32).max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn reads_size_from_ihdr_and_rejects_bad_input() {
        assert_eq!(
            read_png_size(&header(640, 480)),
            Some(ImageSize {
                width: 640,
                height: 480
            })
        );
        assert_eq!(read_png_size(&header(0, 480)), None);
        assert_eq!(read_png_size(&header(640, 480)[..23]), None);
        let mut bad = header(1, 1);
        bad[0] = 0;
        assert_eq!(read_png_size(&bad), None);
    }

    #[tokio::test]
    async fn reads_size_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, header(7, 9)).unwrap();
        assert_eq!(
            read_png_size_from_file(&path).await.unwrap(),
            Some(ImageSize {
                width: 7,
                height: 9
            })
        );
        assert!(
            read_png_size_from_file(&dir.path().join("none.png"))
                .await
                .is_err()
        );
    }

    #[test]
    fn detects_png_paths_case_insensitively() {
        assert!(is_png_path("a/b.PNG"));
        assert!(!is_png_path("a/b.jpg"));
        assert!(!is_png_path("png"));
    }

    #[test]
    fn fits_without_scaling_up() {
        let bounds = ImageSize {
            width: 100,
            height: 50,
        };
        assert_eq!(
            fit_inside(
                ImageSize {
                    width: 40,
                    height: 20
                },
                bounds
            ),
            ImageSize {
                width: 40,
                height: 20
            }
        );
        assert_eq!(
            fit_inside(
                ImageSize {
                    width: 400,
                    height: 100
                },
                bounds
            ),
            ImageSize {
                width: 100,
                height: 25
            }
        );
        assert_eq!(
            fit_inside(
                ImageSize {
                    width: 1000,
                    height: 1
                },
                bounds
            ),
            ImageSize {
                width: 100,
                height: 1
            }
        );
    }
}
