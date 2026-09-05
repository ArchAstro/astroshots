/**
 * Pure-JS PNG decode, area-average downscale, and re-encode. Runs either on
 * a worker thread or inline; it has no dependency on threads itself.
 */
import { PNG } from "pngjs";

import { fitInside, type ImageSize } from "./png.js";

export type ScaledFormat = "png" | "rgb";

export interface ScaleRequest {
  bytes: Buffer;
  target: ImageSize;
  /** png keeps alpha; rgb returns packed 3-byte pixels for cell art. */
  format: ScaledFormat;
}

export interface ScaledImage {
  width: number;
  height: number;
  format: ScaledFormat;
  data: Buffer;
}

/** Area-average RGBA resample. Exact for integer ratios, good enough otherwise. */
export function resampleRgba(
  source: Buffer,
  sourceSize: ImageSize,
  target: ImageSize,
): Buffer {
  const { width: sw, height: sh } = sourceSize;
  const { width: tw, height: th } = target;
  const out = Buffer.alloc(tw * th * 4);
  const xRatio = sw / tw;
  const yRatio = sh / th;
  for (let ty = 0; ty < th; ty += 1) {
    const y0 = Math.floor(ty * yRatio);
    const y1 = Math.max(y0 + 1, Math.floor((ty + 1) * yRatio));
    for (let tx = 0; tx < tw; tx += 1) {
      const x0 = Math.floor(tx * xRatio);
      const x1 = Math.max(x0 + 1, Math.floor((tx + 1) * xRatio));
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      let count = 0;
      for (let y = y0; y < y1 && y < sh; y += 1) {
        let offset = (y * sw + x0) * 4;
        for (let x = x0; x < x1 && x < sw; x += 1) {
          const alpha = source[offset + 3]!;
          // Premultiply so transparent pixels do not bleed their color.
          r += source[offset]! * alpha;
          g += source[offset + 1]! * alpha;
          b += source[offset + 2]! * alpha;
          a += alpha;
          count += 1;
          offset += 4;
        }
      }
      const index = (ty * tw + tx) * 4;
      if (a > 0) {
        out[index] = Math.round(r / a);
        out[index + 1] = Math.round(g / a);
        out[index + 2] = Math.round(b / a);
        out[index + 3] = Math.round(a / count);
      }
    }
  }
  return out;
}

export function rgbaToRgb(rgba: Buffer, pixelCount: number, background = [24, 24, 28]): Buffer {
  const out = Buffer.alloc(pixelCount * 3);
  for (let index = 0; index < pixelCount; index += 1) {
    const alpha = rgba[index * 4 + 3]! / 255;
    for (let channel = 0; channel < 3; channel += 1) {
      const value = rgba[index * 4 + channel]!;
      out[index * 3 + channel] = Math.round(value * alpha + background[channel]! * (1 - alpha));
    }
  }
  return out;
}

export function scaleImage(request: ScaleRequest): ScaledImage {
  const decoded = PNG.sync.read(request.bytes);
  const sourceSize = { width: decoded.width, height: decoded.height };
  const target = fitInside(sourceSize, request.target);
  const rgba =
    target.width === sourceSize.width && target.height === sourceSize.height
      ? decoded.data
      : resampleRgba(decoded.data, sourceSize, target);
  if (request.format === "rgb") {
    return {
      width: target.width,
      height: target.height,
      format: "rgb",
      data: rgbaToRgb(rgba, target.width * target.height),
    };
  }
  const png = new PNG({ width: target.width, height: target.height });
  png.data = rgba;
  return {
    width: target.width,
    height: target.height,
    format: "png",
    data: PNG.sync.write(png, { deflateLevel: 6 }),
  };
}
