import { describe, expect, it } from "vitest";
import { PNG } from "pngjs";

import { fitInside, readPngSize } from "./png.js";
import { cropRgba, resampleRgba, rgbaToRgb, scaleImage } from "./scale.js";

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

  it("crops a sub-rectangle out of an RGBA buffer", () => {
    // 2x2: TL red, TR green, BL blue, BR white.
    const rgba = Buffer.from([255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,255,255]);
    const { data, size } = cropRgba(rgba, { width: 2, height: 2 }, { x: 1, y: 0, width: 1, height: 2 });
    expect(size).toEqual({ width: 1, height: 2 });
    expect([...data]).toEqual([0,255,0,255, 255,255,255,255]);
  });

  it("clamps a crop rect to the image bounds", () => {
    const rgba = Buffer.alloc(4 * 4, 7);
    const { size } = cropRgba(rgba, { width: 2, height: 2 }, { x: 1, y: 1, width: 5, height: 5 });
    expect(size).toEqual({ width: 1, height: 1 });
  });

  it("scaleImage crops before scaling, magnifying the region", () => {
    const solid = (w: number, h: number, rgba: [number,number,number,number]) => {
      const png = new PNG({ width: w, height: h });
      for (let i = 0; i < w * h; i += 1) { png.data[i*4]=rgba[0]; png.data[i*4+1]=rgba[1]; png.data[i*4+2]=rgba[2]; png.data[i*4+3]=rgba[3]; }
      return PNG.sync.write(png);
    };
    // Left half red, right half blue, 4x2.
    const png = new PNG({ width: 4, height: 2 });
    for (let y = 0; y < 2; y += 1) for (let x = 0; x < 4; x += 1) {
      const i = (y*4+x)*4; const blue = x >= 2;
      png.data[i]=blue?0:255; png.data[i+1]=0; png.data[i+2]=blue?255:0; png.data[i+3]=255;
    }
    const bytes = PNG.sync.write(png);
    // Crop the right (blue) half and scale up to 4x4 rgb.
    const out = scaleImage({ bytes, target: { width: 4, height: 4 }, format: "rgb", crop: { x: 2, y: 0, width: 2, height: 2 } });
    expect([...out.data.subarray(0, 3)]).toEqual([0, 0, 255]);
    void solid;
  });

});