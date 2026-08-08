import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";

import { describe, expect, it } from "vitest";

import {
  describeDesktopWindow,
  isNearlyBlankPng,
  matchDesktopWindow,
  type DesktopWindowInfo,
} from "./sources/desktop-macos.js";

function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i]!;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
  }
  return (c ^ 0xffffffff) >>> 0;
}

function pngChunk(type: string, data: Buffer): Buffer {
  const typeBuf = Buffer.from(type, "ascii");
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

function writeSolidRgbPng(
  filePath: string,
  width: number,
  height: number,
  rgb: [number, number, number],
): void {
  const [r, g, b] = rgb;
  const stride = 1 + width * 3;
  const raw = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const row = y * stride;
    raw[row] = 0; // filter None
    for (let x = 0; x < width; x++) {
      const i = row + 1 + x * 3;
      raw[i] = r;
      raw[i + 1] = g;
      raw[i + 2] = b;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const png = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", zlib.deflateSync(raw)),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
  fs.writeFileSync(filePath, png);
}

describe("desktop capture quality helpers", () => {
  it("describes windows for humans (not raw flag dumps)", () => {
    const text = describeDesktopWindow({
      id: 12,
      pid: 1,
      owner: "Astroshots",
      title: "",
      bundleId: "ai.archastro.Astroshots",
      width: 400,
      height: 640,
      x: 10,
      y: 20,
      onScreen: true,
      layer: 0,
    });
    expect(text).toContain("Astroshots");
    expect(text).toContain("400×640");
    expect(text).toContain("on-screen");
    expect(text).not.toMatch(/desktop\.window id=/);
  });

  it("detects nearly blank / black PNGs", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "blank-png-"));
    const black = path.join(dir, "black.png");
    const color = path.join(dir, "color.png");
    try {
      writeSolidRgbPng(black, 64, 48, [0, 0, 0]);
      writeSolidRgbPng(color, 64, 48, [120, 50, 160]);
      expect(isNearlyBlankPng(black)).toBe(true);
      expect(isNearlyBlankPng(color)).toBe(false);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("prefers on-screen windows when matching by bundle id", () => {
    const windows: DesktopWindowInfo[] = [
      {
        id: 1,
        pid: 1,
        owner: "Astroshots",
        title: "",
        bundleId: "ai.archastro.Astroshots",
        width: 1000,
        height: 1000,
        x: 0,
        y: 0,
        onScreen: false,
        layer: 0,
      },
      {
        id: 2,
        pid: 1,
        owner: "Astroshots",
        title: "Tray",
        bundleId: "ai.archastro.Astroshots",
        width: 400,
        height: 640,
        x: 10,
        y: 40,
        onScreen: true,
        layer: 5,
      },
    ];
    const matched = matchDesktopWindow(windows, {
      bundleId: "ai.archastro.Astroshots",
    });
    expect(matched.id).toBe(2);
  });
});
