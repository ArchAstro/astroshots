//! ffmpeg settings shared by every video the crate writes (frame movies in
//! `movie_harness::encode`, browser recordings in `browser::record`).
//!
//! The TS encoders left colour to ffmpeg's defaults: frames were converted
//! with the BT.601 matrix and written untagged, so players guessed, and
//! full-range screencast JPEGs reached the encoder as limited range, which
//! crushed dark UI backgrounds to black. Here every video is converted to
//! BT.709 limited range explicitly and tagged as such, and encoded at a
//! constant quality instead of a fixed low bitrate.

/// Tail of the `-vf` chain: convert to BT.709 limited-range 4:2:0. The source
/// range and matrix come from the decoded frames (full-range RGB for PNG,
/// full-range BT.601 for JPEG).
/// `setparams` marks the frames; current ffmpeg takes the stream's colour
/// description from them and ignores the output options in [`COLOR_TAGS`],
/// which older releases need instead.
pub const COLOR_FILTER: &str = "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,setparams=color_primaries=bt709:color_trc=iec61966-2-1";

/// Stream tags matching [`COLOR_FILTER`], so no player has to guess. The
/// transfer is sRGB (`iec61966-2-1`), which is what captured pixels are: a
/// colour-managed player (Chrome, Safari, QuickTime) applies the BT.709
/// curve to untagged or BT.709-tagged video, which darkens dark UI and
/// lifts mid greys.
pub const COLOR_TAGS: [&str; 8] = [
    "-color_range",
    "tv",
    "-colorspace",
    "bt709",
    "-color_primaries",
    "bt709",
    "-color_trc",
    "iec61966-2-1",
];

/// VP9 for WebM: constant quality, visually lossless on UI and terminal
/// frames (about 44 dB PSNR on text, the 4:2:0 ceiling).
pub const VP9_QUALITY: [&str; 8] = [
    "-c:v",
    "libvpx-vp9",
    "-crf",
    "15",
    "-b:v",
    "0",
    "-row-mt",
    "1",
];

/// H.264 for MP4, at the matching quality.
pub const H264_QUALITY: [&str; 6] = ["-c:v", "libx264", "-crf", "14", "-preset", "slow"];

/// 4:2:0 needs even dimensions; an odd one is stretched by a pixel.
pub fn even(dimension: u32) -> u32 {
    dimension + dimension % 2
}
