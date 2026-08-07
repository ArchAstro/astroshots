import zlib from "node:zlib";

/**
 * Minimal truecolor PNG encoder (no deps). Used for synthetic frames in tests
 * and demos when a full browser paint is not needed.
 */
export function encodeSolidPng(
  width: number,
  height: number,
  rgb: [number, number, number],
): Buffer {
  const [r, g, b] = rgb;
  const stride = 1 + width * 3;
  const raw = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const row = y * stride;
    raw[row] = 0; // filter none
    for (let x = 0; x < width; x++) {
      const i = row + 1 + x * 3;
      raw[i] = r;
      raw[i + 1] = g;
      raw[i + 2] = b;
    }
  }
  return wrapPng(width, height, 2, raw); // 2 = truecolor
}

export function encodeRgbPng(
  width: number,
  height: number,
  /** Row-major RGB triples, length width*height*3 */
  pixels: Buffer | Uint8Array,
): Buffer {
  if (pixels.length < width * height * 3) {
    throw new Error("pixel buffer too small");
  }
  const stride = 1 + width * 3;
  const raw = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const row = y * stride;
    raw[row] = 0;
    const src = y * width * 3;
    for (let x = 0; x < width; x++) {
      const di = row + 1 + x * 3;
      const si = src + x * 3;
      raw[di] = pixels[si]!;
      raw[di + 1] = pixels[si + 1]!;
      raw[di + 2] = pixels[si + 2]!;
    }
  }
  return wrapPng(width, height, 2, raw);
}

function wrapPng(
  width: number,
  height: number,
  colorType: number,
  raw: Buffer,
): Buffer {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = colorType;
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  const compressed = zlib.deflateSync(raw);
  return Buffer.concat([
    signature,
    chunk("IHDR", ihdr),
    chunk("IDAT", compressed),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function chunk(type: string, data: Buffer): Buffer {
  const typeBuf = Buffer.from(type, "ascii");
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

function crc32(buf: Buffer): number {
  let c = 0xffff_ffff;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i]!;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb8_8320 ^ (c >>> 1) : c >>> 1;
    }
  }
  return (c ^ 0xffff_ffff) >>> 0;
}
