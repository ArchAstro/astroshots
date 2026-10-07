//! Port of `packages/astroshot-review/src/images/scale.ts`.
//!
//! PNG decode, area-average downscale, and re-encode. Keeps the TS
//! algorithm (premultiplied area average) so output pixels match.

use anyhow::{Context, Result};

use super::png::{ImageSize, fit_inside};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaledFormat {
    Png,
    Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub struct ScaleRequest<'a> {
    pub bytes: &'a [u8],
    pub target: ImageSize,
    /// png keeps alpha; rgb returns packed 3-byte pixels for cell art.
    pub format: ScaledFormat,
    /// Optional source-pixel crop applied before scaling (for zoom).
    pub crop: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaledImage {
    pub width: u32,
    pub height: u32,
    pub format: ScaledFormat,
    pub data: Vec<u8>,
}

/// `Math.round` for the (possibly negative) finite values used here.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Copy a sub-rectangle out of an RGBA buffer, clamped to the image bounds.
pub fn crop_rgba(source: &[u8], size: ImageSize, rect: Rect) -> (Vec<u8>, ImageSize) {
    let sw = f64::from(size.width);
    let sh = f64::from(size.height);
    let x = 0f64.max((sw - 1.0).min(js_round(rect.x))) as usize;
    let y = 0f64.max((sh - 1.0).min(js_round(rect.y))) as usize;
    let width = 1f64.max((sw - x as f64).min(js_round(rect.width))) as usize;
    let height = 1f64.max((sh - y as f64).min(js_round(rect.height))) as usize;
    let src_width = size.width as usize;
    let mut out = vec![0u8; width * height * 4];
    for row in 0..height {
        let src_start = ((y + row) * src_width + x) * 4;
        out[row * width * 4..(row + 1) * width * 4]
            .copy_from_slice(&source[src_start..src_start + width * 4]);
    }
    (
        out,
        ImageSize {
            width: width as u32,
            height: height as u32,
        },
    )
}

/// Area-average RGBA resample. Exact for integer ratios, good enough otherwise.
pub fn resample_rgba(source: &[u8], source_size: ImageSize, target: ImageSize) -> Vec<u8> {
    let sw = source_size.width as usize;
    let sh = source_size.height as usize;
    let tw = target.width as usize;
    let th = target.height as usize;
    let mut out = vec![0u8; tw * th * 4];
    let x_ratio = sw as f64 / tw as f64;
    let y_ratio = sh as f64 / th as f64;
    for ty in 0..th {
        let y0 = (ty as f64 * y_ratio).floor() as usize;
        let y1 = (y0 + 1).max(((ty + 1) as f64 * y_ratio).floor() as usize);
        for tx in 0..tw {
            let x0 = (tx as f64 * x_ratio).floor() as usize;
            let x1 = (x0 + 1).max(((tx + 1) as f64 * x_ratio).floor() as usize);
            let (mut r, mut g, mut b, mut a, mut count) = (0u64, 0u64, 0u64, 0u64, 0u64);
            let mut y = y0;
            while y < y1 && y < sh {
                let mut offset = (y * sw + x0) * 4;
                let mut x = x0;
                while x < x1 && x < sw {
                    let alpha = u64::from(source[offset + 3]);
                    // Premultiply so transparent pixels do not bleed their color.
                    r += u64::from(source[offset]) * alpha;
                    g += u64::from(source[offset + 1]) * alpha;
                    b += u64::from(source[offset + 2]) * alpha;
                    a += alpha;
                    count += 1;
                    offset += 4;
                    x += 1;
                }
                y += 1;
            }
            let index = (ty * tw + tx) * 4;
            if a > 0 {
                let a_f = a as f64;
                out[index] = js_round(r as f64 / a_f) as u8;
                out[index + 1] = js_round(g as f64 / a_f) as u8;
                out[index + 2] = js_round(b as f64 / a_f) as u8;
                out[index + 3] = js_round(a_f / count as f64) as u8;
            }
        }
    }
    out
}

/// Default stage background used when flattening alpha.
pub const DEFAULT_RGB_BACKGROUND: [u8; 3] = [24, 24, 28];

pub fn rgba_to_rgb(rgba: &[u8], pixel_count: usize, background: [u8; 3]) -> Vec<u8> {
    let mut out = vec![0u8; pixel_count * 3];
    for index in 0..pixel_count {
        let alpha = f64::from(rgba[index * 4 + 3]) / 255.0;
        for channel in 0..3 {
            let value = f64::from(rgba[index * 4 + channel]);
            out[index * 3 + channel] =
                js_round(value * alpha + f64::from(background[channel]) * (1.0 - alpha)) as u8;
        }
    }
    out
}

/// Decode a PNG to 8-bit RGBA (what `PNG.sync.read` returns).
fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, ImageSize)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().context("Invalid PNG")?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).context("Invalid PNG")?;
    let data = &buffer[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => anyhow::bail!("Invalid PNG: unexpanded palette"),
    };
    Ok((
        rgba,
        ImageSize {
            width: info.width,
            height: info.height,
        },
    ))
}

fn encode_rgba_png(rgba: &[u8], size: ImageSize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Default);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    writer.finish()?;
    Ok(out)
}

pub fn scale_image(request: &ScaleRequest<'_>) -> Result<ScaledImage> {
    let (mut source_data, mut source_size) = decode_rgba(request.bytes)?;
    if let Some(crop) = request.crop {
        let (data, size) = crop_rgba(&source_data, source_size, crop);
        source_data = data;
        source_size = size;
    }
    let target = fit_inside(source_size, request.target);
    let rgba = if target == source_size {
        source_data
    } else {
        resample_rgba(&source_data, source_size, target)
    };
    if request.format == ScaledFormat::Rgb {
        return Ok(ScaledImage {
            width: target.width,
            height: target.height,
            format: ScaledFormat::Rgb,
            data: rgba_to_rgb(
                &rgba,
                (target.width * target.height) as usize,
                DEFAULT_RGB_BACKGROUND,
            ),
        });
    }
    Ok(ScaledImage {
        width: target.width,
        height: target.height,
        format: ScaledFormat::Png,
        data: encode_rgba_png(&rgba, target)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::images::png::read_png_size;

    fn size(width: u32, height: u32) -> ImageSize {
        ImageSize { width, height }
    }

    fn solid_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let data: Vec<u8> = (0..width * height).flat_map(|_| rgba).collect();
        encode_rgba_png(&data, size(width, height)).unwrap()
    }

    #[test]
    fn fits_inside_bounds_without_upscaling() {
        assert_eq!(fit_inside(size(400, 200), size(100, 100)), size(100, 50));
        assert_eq!(fit_inside(size(40, 20), size(100, 100)), size(40, 20));
    }

    #[test]
    fn reads_png_dimensions_from_the_header() {
        assert_eq!(
            read_png_size(&solid_png(17, 5, [1, 2, 3, 255])),
            Some(size(17, 5))
        );
        assert_eq!(read_png_size(b"not a png"), None);
    }

    #[test]
    fn area_averages_colors() {
        let source = [
            255, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255, 0, 0, 255, 255,
        ];
        let out = resample_rgba(&source, size(2, 2), size(1, 1));
        assert_eq!(out, vec![128, 0, 128, 255]);
    }

    #[test]
    fn flattens_alpha_onto_the_stage_background() {
        let rgb = rgba_to_rgb(&[255, 255, 255, 0], 1, [10, 20, 30]);
        assert_eq!(rgb, vec![10, 20, 30]);
    }

    #[test]
    fn downscales_a_png_and_re_encodes_it() {
        let scaled = scale_image(&ScaleRequest {
            bytes: &solid_png(200, 100, [9, 8, 7, 255]),
            target: size(50, 50),
            format: ScaledFormat::Png,
            crop: None,
        })
        .unwrap();
        assert_eq!(scaled.width, 50);
        assert_eq!(scaled.height, 25);
        assert_eq!(read_png_size(&scaled.data), Some(size(50, 25)));
        let rgb = scale_image(&ScaleRequest {
            bytes: &solid_png(200, 100, [9, 8, 7, 255]),
            target: size(4, 4),
            format: ScaledFormat::Rgb,
            crop: None,
        })
        .unwrap();
        assert_eq!(rgb.data.len(), 4 * 2 * 3);
        assert_eq!(&rgb.data[0..3], &[9, 8, 7]);
    }

    #[test]
    fn crops_a_sub_rectangle_out_of_an_rgba_buffer() {
        // 2x2: TL red, TR green, BL blue, BR white.
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let (data, out_size) = crop_rgba(
            &rgba,
            size(2, 2),
            Rect {
                x: 1.0,
                y: 0.0,
                width: 1.0,
                height: 2.0,
            },
        );
        assert_eq!(out_size, size(1, 2));
        assert_eq!(data, vec![0, 255, 0, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn clamps_a_crop_rect_to_the_image_bounds() {
        let rgba = [7u8; 16];
        let (_, out_size) = crop_rgba(
            &rgba,
            size(2, 2),
            Rect {
                x: 1.0,
                y: 1.0,
                width: 5.0,
                height: 5.0,
            },
        );
        assert_eq!(out_size, size(1, 1));
    }

    #[test]
    fn scale_image_crops_before_scaling_magnifying_the_region() {
        // Left half red, right half blue, 4x2.
        let mut data = Vec::new();
        for _y in 0..2 {
            for x in 0..4 {
                let blue = x >= 2;
                data.extend_from_slice(&[
                    if blue { 0 } else { 255 },
                    0,
                    if blue { 255 } else { 0 },
                    255,
                ]);
            }
        }
        let bytes = encode_rgba_png(&data, size(4, 2)).unwrap();
        // Crop the right (blue) half and scale up to 4x4 rgb.
        let out = scale_image(&ScaleRequest {
            bytes: &bytes,
            target: size(4, 4),
            format: ScaledFormat::Rgb,
            crop: Some(Rect {
                x: 2.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            }),
        })
        .unwrap();
        assert_eq!(&out.data[0..3], &[0, 0, 255]);
    }
}
