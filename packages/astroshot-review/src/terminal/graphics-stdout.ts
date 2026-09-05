/**
 * The stdout Ink renders into. It forwards everything to the real stream and
 * splices kitty placements into each frame so pictures and text arrive in
 * the same synchronized update.
 */
import type { ImageLayer } from "./image-layer.js";

const ENTER_ALT_SCREEN = "\x1b[?1049h";
const END_SYNC = "\x1b[?2026l";

const ANSI_PATTERN = /\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b_[^\x1b]*\x1b\\|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-_]/g;

export function hasPrintableText(chunk: string): boolean {
  return chunk.replace(ANSI_PATTERN, "").trim().length > 0;
}

export function spliceGraphics(chunk: string, graphics: string): string {
  if (!graphics) return chunk;
  return chunk.endsWith(END_SYNC)
    ? chunk.slice(0, -END_SYNC.length) + graphics + END_SYNC
    : chunk + graphics;
}

export function createGraphicsStdout(
  real: NodeJS.WriteStream,
  layer: ImageLayer,
): NodeJS.WriteStream {
  const write = (chunk: string | Uint8Array, ...rest: unknown[]): boolean => {
    let text = typeof chunk === "string" ? chunk : Buffer.from(chunk).toString("utf8");
    if (text.includes(ENTER_ALT_SCREEN)) {
      // Ink enters the alternate screen without homing the cursor. Frames
      // must start at the top-left so layout coordinates map to screen cells.
      text = text.replace(ENTER_ALT_SCREEN, `${ENTER_ALT_SCREEN}\x1b[H\x1b[2J`);
      layer.invalidate();
    }
    if (hasPrintableText(text)) {
      text = spliceGraphics(text, layer.render());
    }
    return (real.write as (chunk: string, ...args: unknown[]) => boolean)(text, ...rest);
  };
  return new Proxy(real, {
    get(target, property, receiver) {
      if (property === "write") return write;
      const value = Reflect.get(target, property, receiver) as unknown;
      return typeof value === "function" ? (value as (...args: unknown[]) => unknown).bind(target) : value;
    },
  });
}
