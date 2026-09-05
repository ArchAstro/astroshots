/**
 * Kitty graphics protocol support for PTY captures.
 *
 * A headless xterm has no picture layer, so this tracker sits between the
 * child's output and the terminal: it answers the capability queries a
 * graphics-aware program sends, records every transmitted image and
 * placement at the cursor position the terminal reports, strips the APC
 * sequences from the text stream, and finally yields the pictures as
 * absolutely positioned overlays for the HTML renderer.
 */
import fs from "node:fs";
import zlib from "node:zlib";

import type { HeadlessTerminal } from "./terminal-html.js";
import { writeTerminal } from "./terminal-html.js";

const APC_START = "\x1b_G";
const ST = "\x1b\\";

export interface GraphicsOverlay {
  col: number;
  row: number;
  cols: number;
  rows: number;
  z: number;
  /** PNG bytes as a data URL. */
  dataUrl: string;
}

interface StoredImage {
  id: number;
  format: number;
  width?: number;
  height?: number;
  compressed: boolean;
  payload: string;
  medium: string;
  /** Lazily produced PNG bytes. */
  png?: Buffer;
}

interface StoredPlacement {
  imageId: number;
  placementId: number;
  col: number;
  row: number;
  cols: number;
  rows: number;
  z: number;
}

interface PendingTransmit {
  keys: Record<string, string>;
  payload: string;
}

export interface KittyTrackerOptions {
  terminal: HeadlessTerminal;
  cols: number;
  rows: number;
  cellWidth: number;
  cellHeight: number;
  /** Where terminal replies (query answers, device attributes) are sent. */
  reply: (data: string) => void;
}

function parseKeys(text: string): Record<string, string> {
  const keys: Record<string, string> = {};
  for (const pair of text.split(",")) {
    const equals = pair.indexOf("=");
    if (equals > 0) keys[pair.slice(0, equals)] = pair.slice(equals + 1);
  }
  return keys;
}

function crc32(buffer: Buffer): number {
  let crc = ~0;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return ~crc >>> 0;
}

function pngChunk(type: string, data: Buffer): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const typed = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(typed));
  return Buffer.concat([length, typed, crc]);
}

/** Encode raw RGB/RGBA pixels as a PNG (filter type 0 on every scanline). */
export function encodePng(pixels: Buffer, width: number, height: number, channels: 3 | 4): Buffer {
  const stride = width * channels;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0;
    pixels.copy(raw, y * (stride + 1) + 1, y * stride, y * stride + stride);
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = channels === 4 ? 6 : 2;
  header[10] = 0;
  header[11] = 0;
  header[12] = 0;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    pngChunk("IHDR", header),
    pngChunk("IDAT", zlib.deflateSync(raw)),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

export class KittyGraphicsTracker {
  private readonly images = new Map<number, StoredImage>();
  private readonly placements = new Map<string, StoredPlacement>();
  private pending: PendingTransmit | null = null;
  private carry = "";
  private queryLog: string[] = [];

  constructor(private readonly options: KittyTrackerOptions) {}

  /** Queries the child sent, for assertions. */
  get queries(): readonly string[] {
    return this.queryLog;
  }

  get imageCount(): number {
    return this.images.size;
  }

  /** Feed child output: text goes to the terminal, graphics are recorded. */
  async write(data: string): Promise<void> {
    let buffer = this.carry + data;
    this.carry = "";
    while (buffer.length > 0) {
      const start = buffer.indexOf(APC_START);
      if (start === -1) {
        await this.writeText(buffer);
        return;
      }
      if (start > 0) await this.writeText(buffer.slice(0, start));
      const end = buffer.indexOf(ST, start + APC_START.length);
      if (end === -1) {
        // Wait for the rest of the sequence.
        this.carry = buffer.slice(start);
        return;
      }
      this.handleCommand(buffer.slice(start + APC_START.length, end));
      buffer = buffer.slice(end + ST.length);
    }
  }

  private async writeText(text: string): Promise<void> {
    // Answer the size reports a graphics-aware program asks for. xterm itself
    // replies to device attributes.
    if (text.includes("\x1b[16t")) {
      this.options.reply(`\x1b[6;${this.options.cellHeight};${this.options.cellWidth}t`);
    }
    if (text.includes("\x1b[14t")) {
      this.options.reply(
        `\x1b[4;${this.options.cellHeight * this.options.rows};${this.options.cellWidth * this.options.cols}t`,
      );
    }
    await writeTerminal(this.options.terminal, text);
  }

  private handleCommand(body: string): void {
    const separator = body.indexOf(";");
    const keys = parseKeys(separator === -1 ? body : body.slice(0, separator));
    const payload = separator === -1 ? "" : body.slice(separator + 1);
    const action = keys.a ?? (this.pending ? "t" : "t");

    if (this.pending) {
      this.pending.payload += payload;
      if (keys.m !== "1") {
        const complete = this.pending;
        this.pending = null;
        this.storeImage(complete.keys, complete.payload);
        if (complete.keys.a === "T") this.place(complete.keys);
      }
      return;
    }

    switch (action) {
      case "q": {
        this.queryLog.push(body);
        const id = keys.i ?? "0";
        const supported = keys.t === "d" || keys.t === "f";
        this.options.reply(`\x1b_Gi=${id};${supported ? "OK" : "EINVAL:unsupported medium"}\x1b\\`);
        return;
      }
      case "t":
      case "T": {
        if (keys.m === "1") {
          this.pending = { keys, payload };
          return;
        }
        this.storeImage(keys, payload);
        if (action === "T") this.place(keys);
        return;
      }
      case "p":
        this.place(keys);
        return;
      case "d":
        this.delete(keys);
        return;
      default:
        return;
    }
  }

  private storeImage(keys: Record<string, string>, payload: string): void {
    const id = Number(keys.i ?? 0);
    this.images.set(id, {
      id,
      format: Number(keys.f ?? 32),
      width: keys.s ? Number(keys.s) : undefined,
      height: keys.v ? Number(keys.v) : undefined,
      compressed: keys.o === "z",
      payload,
      medium: keys.t ?? "d",
    });
    if (keys.q !== "1" && keys.q !== "2") {
      this.options.reply(`\x1b_Gi=${id};OK\x1b\\`);
    }
  }

  private place(keys: Record<string, string>): void {
    const imageId = Number(keys.i ?? 0);
    if (!this.images.has(imageId)) return;
    const buffer = this.options.terminal.buffer.active;
    const placementId = Number(keys.p ?? 0);
    const placement: StoredPlacement = {
      imageId,
      placementId,
      col: buffer.cursorX,
      row: buffer.cursorY,
      cols: Number(keys.c ?? 0),
      rows: Number(keys.r ?? 0),
      z: Number(keys.z ?? 0),
    };
    this.placements.set(`${imageId}:${placementId}`, placement);
  }

  private delete(keys: Record<string, string>): void {
    const mode = keys.d ?? "a";
    const freeData = mode === mode.toUpperCase();
    switch (mode.toLowerCase()) {
      case "a":
        this.placements.clear();
        if (freeData) this.images.clear();
        return;
      case "i": {
        const imageId = Number(keys.i ?? 0);
        const placementId = keys.p ? Number(keys.p) : null;
        for (const [key, placement] of this.placements) {
          if (placement.imageId !== imageId) continue;
          if (placementId !== null && placement.placementId !== placementId) continue;
          this.placements.delete(key);
        }
        if (freeData && placementId === null) this.images.delete(imageId);
        return;
      }
      default:
        return;
    }
  }

  private pngFor(image: StoredImage): Buffer | null {
    if (image.png) return image.png;
    let bytes: Buffer;
    if (image.medium === "f") {
      const filePath = Buffer.from(image.payload, "base64").toString("utf8");
      try {
        bytes = fs.readFileSync(filePath);
      } catch {
        return null;
      }
    } else {
      bytes = Buffer.from(image.payload, "base64");
    }
    if (image.compressed) bytes = zlib.inflateSync(bytes);
    if (image.format === 100) {
      image.png = bytes;
    } else if (image.width && image.height) {
      image.png = encodePng(bytes, image.width, image.height, image.format === 24 ? 3 : 4);
    } else {
      return null;
    }
    return image.png;
  }

  /** Pictures currently visible, ordered for painting. */
  overlays(): GraphicsOverlay[] {
    const result: GraphicsOverlay[] = [];
    for (const placement of this.placements.values()) {
      const image = this.images.get(placement.imageId);
      if (!image) continue;
      const png = this.pngFor(image);
      if (!png) continue;
      if (placement.cols <= 0 || placement.rows <= 0) continue;
      result.push({
        col: placement.col,
        row: placement.row,
        cols: placement.cols,
        rows: placement.rows,
        z: placement.z,
        dataUrl: `data:image/png;base64,${png.toString("base64")}`,
      });
    }
    return result.sort((a, b) => a.z - b.z);
  }
}

/** HTML for overlays, positioned in cell units so they track the font metrics. */
export function overlaysToHtml(overlays: GraphicsOverlay[], lineHeight: number): string {
  return overlays
    .map(
      (overlay) =>
        `<img class="tui-graphic" src="${overlay.dataUrl}" style="left:${overlay.col}ch;` +
        `top:${overlay.row * lineHeight}em;width:${overlay.cols}ch;height:${overlay.rows * lineHeight}em" alt="" />`,
    )
    .join("");
}
