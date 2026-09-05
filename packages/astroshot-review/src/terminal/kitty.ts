/**
 * Kitty terminal graphics protocol encoder.
 *
 * Only the subset the review tray needs: transmit PNG or raw RGB data by id,
 * place a transmitted image over a cell box, and delete placements or image
 * data. Every command is emitted quietly (q=2) so the terminal never answers
 * on stdin while Ink owns it.
 *
 * Spec: https://sw.kovidgoyal.net/kitty/graphics-protocol/
 */

export const APC = "\x1b_G";
export const ST = "\x1b\\";

/** Payload chunk size; the protocol caps chunks at 4096 bytes. */
export const CHUNK_SIZE = 4096;

export type ImageFormat = 100 | 24 | 32;

export interface TransmitRequest {
  id: number;
  /** 100 = PNG bytes, 24 = RGB, 32 = RGBA. */
  format: ImageFormat;
  data: Buffer;
  /** Required for raw formats. */
  width?: number;
  height?: number;
  /** Raw payload is zlib-compressed (o=z). */
  compressed?: boolean;
  /**
   * When set the payload is a base64 file path the terminal reads itself
   * (t=f). Only valid when the terminal runs on this machine.
   */
  filePath?: string;
}

export interface PlaceRequest {
  id: number;
  placementId: number;
  cols: number;
  rows: number;
  /** Stacking order; images with z >= 0 draw above text. */
  z?: number;
}

export type DeleteRequest =
  | { kind: "placement"; id: number; placementId: number }
  | { kind: "image"; id: number }
  | { kind: "all-placements" }
  | { kind: "all" };

function control(pairs: Record<string, number | string | undefined>): string {
  return Object.entries(pairs)
    .filter(([, value]) => value !== undefined)
    .map(([key, value]) => `${key}=${value}`)
    .join(",");
}

export function chunkPayload(base64: string, size = CHUNK_SIZE): string[] {
  if (base64.length === 0) return [""];
  const chunks: string[] = [];
  for (let offset = 0; offset < base64.length; offset += size) {
    chunks.push(base64.slice(offset, offset + size));
  }
  return chunks;
}

/** Transmit image data without displaying it (a=t). */
export function encodeTransmit(request: TransmitRequest): string {
  const base = {
    a: "t",
    i: request.id,
    f: request.format,
    q: 2,
    s: request.format === 100 ? undefined : request.width,
    v: request.format === 100 ? undefined : request.height,
    o: request.compressed ? "z" : undefined,
  };
  if (request.filePath) {
    const payload = Buffer.from(request.filePath, "utf8").toString("base64");
    return `${APC}${control({ ...base, t: "f" })};${payload}${ST}`;
  }
  const chunks = chunkPayload(request.data.toString("base64"));
  return chunks
    .map((chunk, index) => {
      const more = index < chunks.length - 1 ? 1 : 0;
      const keys = index === 0 ? control({ ...base, t: "d", m: more }) : `m=${more}`;
      return `${APC}${keys};${chunk}${ST}`;
    })
    .join("");
}

/**
 * Display a transmitted image over a c×r cell box at the cursor (a=p). C=1
 * keeps the cursor where it was so Ink's own cursor bookkeeping stays valid.
 * A placement with the same (i, p) pair replaces the previous one.
 */
export function encodePlace(request: PlaceRequest): string {
  return `${APC}${control({
    a: "p",
    i: request.id,
    p: request.placementId,
    c: request.cols,
    r: request.rows,
    z: request.z ?? 0,
    C: 1,
    q: 2,
  })}${ST}`;
}

export function encodeDelete(request: DeleteRequest): string {
  switch (request.kind) {
    case "placement":
      return `${APC}${control({ a: "d", d: "i", i: request.id, p: request.placementId, q: 2 })}${ST}`;
    case "image":
      return `${APC}${control({ a: "d", d: "I", i: request.id, q: 2 })}${ST}`;
    case "all-placements":
      return `${APC}${control({ a: "d", d: "a", q: 2 })}${ST}`;
    case "all":
      return `${APC}${control({ a: "d", d: "A", q: 2 })}${ST}`;
  }
}

/** The 1×1 RGB query kitty documents for capability detection. */
export function encodeQuery(id: number): string {
  return `${APC}${control({ i: id, s: 1, v: 1, a: "q", t: "d", f: 24 })};AAAA${ST}`;
}

export function encodeFileQuery(id: number, filePath: string): string {
  const payload = Buffer.from(filePath, "utf8").toString("base64");
  return `${APC}${control({ i: id, a: "q", t: "f", f: 100 })};${payload}${ST}`;
}

/** Move the cursor to a 1-based row/column (CUP). */
export function cursorTo(row: number, col: number): string {
  return `\x1b[${row};${col}H`;
}

export const SAVE_CURSOR = "\x1b7";
export const RESTORE_CURSOR = "\x1b8";

export interface ParsedGraphicsCommand {
  keys: Record<string, string>;
  payload: string;
}

/** Parse one `ESC _ G <keys> ; <payload> ESC \` command body. */
export function parseGraphicsCommand(body: string): ParsedGraphicsCommand {
  const separator = body.indexOf(";");
  const keyText = separator === -1 ? body : body.slice(0, separator);
  const payload = separator === -1 ? "" : body.slice(separator + 1);
  const keys: Record<string, string> = {};
  for (const pair of keyText.split(",")) {
    if (!pair) continue;
    const equals = pair.indexOf("=");
    if (equals === -1) continue;
    keys[pair.slice(0, equals)] = pair.slice(equals + 1);
  }
  return { keys, payload };
}
