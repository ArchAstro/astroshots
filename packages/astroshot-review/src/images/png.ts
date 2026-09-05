import fs from "node:fs";

const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

export interface ImageSize {
  width: number;
  height: number;
}

/** Read width/height from a PNG's IHDR chunk without decoding pixels. */
export function readPngSize(bytes: Buffer): ImageSize | null {
  if (bytes.length < 24) return null;
  if (!bytes.subarray(0, 8).equals(PNG_SIGNATURE)) return null;
  if (bytes.toString("ascii", 12, 16) !== "IHDR") return null;
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  if (width === 0 || height === 0) return null;
  return { width, height };
}

/** Read just enough of a file to learn its pixel size. */
export async function readPngSizeFromFile(filePath: string): Promise<ImageSize | null> {
  const handle = await fs.promises.open(filePath, "r");
  try {
    const header = Buffer.alloc(32);
    const { bytesRead } = await handle.read(header, 0, 32, 0);
    return readPngSize(header.subarray(0, bytesRead));
  } finally {
    await handle.close();
  }
}

export function isPngPath(filePath: string): boolean {
  return /\.png$/i.test(filePath);
}

/**
 * Largest box that fits inside `bounds` while keeping `source`'s aspect
 * ratio. Never scales up: a small source keeps its own size.
 */
export function fitInside(source: ImageSize, bounds: ImageSize): ImageSize {
  if (source.width <= bounds.width && source.height <= bounds.height) {
    return { width: source.width, height: source.height };
  }
  const scale = Math.min(bounds.width / source.width, bounds.height / source.height);
  return {
    width: Math.max(1, Math.round(source.width * scale)),
    height: Math.max(1, Math.round(source.height * scale)),
  };
}
