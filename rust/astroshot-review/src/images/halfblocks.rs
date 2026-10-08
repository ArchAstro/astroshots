//! Port of `packages/astroshot-review/src/images/halfblocks.ts`.
//!
//! Fallback renderer for terminals without a graphics protocol: two pixels per
//! cell using the upper-half block and truecolor foreground/background.

#[derive(Debug, Clone)]
pub struct CellArtOptions<'a> {
    pub width: usize,
    pub height: usize,
    pub rgb: &'a [u8],
}

pub fn rgb_to_half_block_lines(options: &CellArtOptions<'_>) -> Vec<String> {
    use std::fmt::Write;
    let CellArtOptions { width, height, rgb } = *options;
    // TS interpolates `undefined` for out-of-range reads; Rust reads 0 instead.
    let at = |index: usize| rgb.get(index).copied().unwrap_or(0);
    let mut lines = Vec::new();
    let mut y = 0;
    while y < height {
        let mut line = String::new();
        for x in 0..width {
            let top = (y * width + x) * 3;
            let bottom_row = if y + 1 < height { y + 1 } else { y };
            let bottom = (bottom_row * width + x) * 3;
            let _ = write!(
                line,
                "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                at(top),
                at(top + 1),
                at(top + 2),
                at(bottom),
                at(bottom + 1),
                at(bottom + 2)
            );
        }
        line.push_str("\x1b[0m");
        lines.push(line);
        y += 2;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_two_vertical_pixels_into_one_cell_with_fg_bg_truecolor() {
        // 1x2: top red, bottom blue.
        let rgb = [255, 0, 0, 0, 0, 255];
        let lines = rgb_to_half_block_lines(&CellArtOptions {
            width: 1,
            height: 2,
            rgb: &rgb,
        });
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "\x1b[38;2;255;0;0m\x1b[48;2;0;0;255m▀\x1b[0m");
    }

    #[test]
    fn emits_one_line_per_two_rows_and_reuses_the_last_row_when_odd() {
        let rgb = [10u8; 3 * 3]; // 1x3
        let lines = rgb_to_half_block_lines(&CellArtOptions {
            width: 1,
            height: 3,
            rgb: &rgb,
        });
        assert_eq!(lines.len(), 2);
    }
}
