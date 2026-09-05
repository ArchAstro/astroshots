import { describe, expect, it } from "vitest";
import { PNG } from "pngjs";

import { fitInside, readPngSize } from "./png.js";
import { resampleRgba, rgbaToRgb, scaleImage } from "./scale.js";

function solidPng(width: number, height: number, rgba: [number, number, number, number]): Buffer {
  const png = new PNG({ width, height });
  for (let index = 0; index < width * height; index += 1) {
    png.data[index * 4] = rgba[0];
    png.data[index * 4 + 1] = rgba[1];
    png.data[index * 4 + 2] = rgba[2];
    png.data[index * 4 + 3] = rgba[3];
  }
  return PNG.sync.write(png);
}

describe("image scaling", () => {
  it("fits inside bounds without upscaling", () => {
    expect(fitInside({ width: 400, height: 200 }, { width: 100, height: 100 })).toEqual({ width: 100, height: 50 });
    expect(fitInside({ width: 40, height: 20 }, { width: 100, height: 100 })).toEqual({ width: 40, height: 20 });
  });

  it("reads PNG dimensions from the header", () => {
    expect(readPngSize(solidPng(17, 5, [1, 2, 3, 255]))).toEqual({ width: 17, height: 5 });
    expect(readPngSize(Buffer.from("not a png"))).toBeNull();
  });

  it("area-averages colors", () => {
    const source = Buffer.from([255, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255, 0, 0, 255, 255]);
    const out = resampleRgba(source, { width: 2, height: 2 }, { width: 1, height: 1 });
    expect([...out]).toEqual([128, 0, 128, 255]);
  });

  it("flattens alpha onto the stage background", () => {
    const rgb = rgbaToRgb(Buffer.from([255, 255, 255, 0]), 1, [10, 20, 30]);
    expect([...rgb]).toEqual([10, 20, 30]);
  });

  it("downscales a PNG and re-encodes it", () => {
    const scaled = scaleImage({ bytes: solidPng(200, 100, [9, 8, 7, 255]), target: { width: 50, height: 50 }, format: "png" });
    expect(scaled.width).toBe(50);
    expect(scaled.height).toBe(25);
    expect(readPngSize(scaled.data)).toEqual({ width: 50, height: 25 });
    const rgb = scaleImage({ bytes: solidPng(200, 100, [9, 8, 7, 255]), target: { width: 4, height: 4 }, format: "rgb" });
    expect(rgb.data.length).toBe(4 * 2 * 3);
    expect([...rgb.data.subarray(0, 3)]).toEqual([9, 8, 7]);
  });
});
