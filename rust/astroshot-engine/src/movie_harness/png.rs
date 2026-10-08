//! Minimal truecolor PNG encoder, used for synthetic frames in tests and
//! demos when a full paint is not needed.
//!
//! Port of `packages/movie-harness/src/png.ts`. The pixel data and chunk
//! layout match TS; the deflate stream comes from `flate2`, so the compressed
//! bytes can differ from Node's zlib while decoding to identical pixels.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PngError {
    #[error("pixel buffer too small")]
    PixelBufferTooSmall,
}

/// Solid-color truecolor PNG.
pub fn encode_solid_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let stride = 1 + width as usize * 3;
    let mut raw = vec![0u8; stride * height as usize];
    for row in raw.chunks_exact_mut(stride) {
        // row[0] is the "none" filter byte.
        for pixel in row[1..].as_chunks_mut::<3>().0 {
            pixel.copy_from_slice(&rgb);
        }
    }
    wrap_png(width, height, 2, &raw) // 2 = truecolor
}

/// `pixels` is row-major RGB triples, at least `width * height * 3` bytes.
pub fn encode_rgb_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, PngError> {
    let row_bytes = width as usize * 3;
    if pixels.len() < row_bytes * height as usize {
        return Err(PngError::PixelBufferTooSmall);
    }
    let stride = 1 + row_bytes;
    let mut raw = vec![0u8; stride * height as usize];
    for (y, row) in raw.chunks_exact_mut(stride).enumerate() {
        row[1..].copy_from_slice(&pixels[y * row_bytes..(y + 1) * row_bytes]);
    }
    Ok(wrap_png(width, height, 2, &raw))
}

fn wrap_png(width: u32, height: u32, color_type: u8, raw: &[u8]) -> Vec<u8> {
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]); // bit depth, color type, compression, filter, interlace
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(raw).expect("write to Vec");
    let compressed = encoder.finish().expect("finish zlib stream");

    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &compressed);
    push_chunk(&mut out, b"IEND", &[]);
    out
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(buf: &[u8]) -> u32 {
    let mut c: u32 = 0xffff_ffff;
    for &byte in buf {
        c ^= u32::from(byte);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    c ^ 0xffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info, buf)
    }

    #[test]
    fn crc32_matches_known_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn solid_png_decodes_to_the_requested_color() {
        let bytes = encode_solid_png(3, 2, [10, 20, 30]);
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let (info, pixels) = decode(&bytes);
        assert_eq!((info.width, info.height), (3, 2));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(pixels, [10, 20, 30].repeat(6));
    }

    #[test]
    fn rgb_png_roundtrips_pixels() {
        let pixels: Vec<u8> = (0..2 * 2 * 3).collect();
        let bytes = encode_rgb_png(2, 2, &pixels).unwrap();
        let (_, decoded) = decode(&bytes);
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn rgb_png_rejects_short_buffer() {
        assert_eq!(
            encode_rgb_png(2, 2, &[0; 11]).unwrap_err().to_string(),
            "pixel buffer too small"
        );
    }
}
