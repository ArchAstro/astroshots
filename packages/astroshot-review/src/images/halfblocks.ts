/**
 * Fallback renderer for terminals without a graphics protocol: two pixels per
 * cell using the upper-half block and truecolor foreground/background.
 */

export interface CellArtOptions {
  width: number;
  height: number;
  rgb: Buffer;
}

export function rgbToHalfBlockLines(options: CellArtOptions): string[] {
  const { width, height, rgb } = options;
  const lines: string[] = [];
  for (let y = 0; y < height; y += 2) {
    let line = "";
    for (let x = 0; x < width; x += 1) {
      const top = (y * width + x) * 3;
      const bottomRow = y + 1 < height ? y + 1 : y;
      const bottom = (bottomRow * width + x) * 3;
      line += `\x1b[38;2;${rgb[top]};${rgb[top + 1]};${rgb[top + 2]}m` +
        `\x1b[48;2;${rgb[bottom]};${rgb[bottom + 1]};${rgb[bottom + 2]}m▀`;
    }
    line += "\x1b[0m";
    lines.push(line);
  }
  return lines;
}
