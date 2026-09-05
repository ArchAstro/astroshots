/**
 * herdr pane-graphics transport.
 *
 * herdr (an agent terminal multiplexer) emulates the terminal itself and does
 * not forward a program's raw Kitty escapes. Instead it exposes a unix-socket
 * JSON API — `pane.graphics.set` / `pane.graphics.clear` place and remove
 * images on named layers composited over the pane's text. This module speaks
 * that API so the review tray shows pixel-perfect images inside herdr.
 *
 * Enabling it needs `[experimental] kitty_graphics = true` in herdr's
 * config.toml AND a client reattach: herdr 0.8.2 latches the client's graphics
 * setting at startup, so a client started with it off reports the host cell
 * size as unavailable until it is detached and reattached once.
 *
 * Wire format: newline-delimited JSON over the socket named by
 * HERDR_SOCKET_PATH, targeting HERDR_PANE_ID.
 */
import { createConnection, type Socket } from "node:net";
import { setTimeout as delay } from "node:timers/promises";

interface Reply {
  error?: { code: string; message: string };
  result?: { type?: string; cell_width_px?: number; cell_height_px?: number; max_layers_per_pane?: number; pane_visible?: boolean };
}

export interface HerdrAddress {
  socket: string;
  pane: string;
}

export function herdrAddress(env: NodeJS.ProcessEnv = process.env): HerdrAddress | null {
  if (env.HERDR_ENV !== "1") return null;
  if (!env.HERDR_SOCKET_PATH || !env.HERDR_PANE_ID) return null;
  return { socket: env.HERDR_SOCKET_PATH, pane: env.HERDR_PANE_ID };
}

/** A short-lived request/response over the socket. */
function request(socket: string, method: string, params: Record<string, unknown>): Promise<Reply> {
  return new Promise((resolve, reject) => {
    const client = createConnection(socket);
    let text = "";
    const fail = (error: Error) => {
      client.removeAllListeners();
      client.destroy();
      reject(error);
    };
    client.on("connect", () => client.write(JSON.stringify({ id: "astroshot-review", method, params }) + "\n"));
    client.on("data", (chunk) => {
      text += chunk.toString();
      const end = text.indexOf("\n");
      if (end < 0) {
        if (text.length > 1024 * 1024) fail(new Error("herdr response too large"));
        return;
      }
      try {
        const reply = JSON.parse(text.slice(0, end)) as Reply;
        client.removeAllListeners();
        client.destroy();
        resolve(reply);
      } catch {
        fail(new Error("invalid herdr response"));
      }
    });
    client.on("error", fail);
    client.setTimeout(2500, () => fail(new Error("herdr request timed out")));
  });
}

// 1x1 transparent PNG, for probing whether pane.graphics.set is accepted.
const PROBE_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  "base64",
);

/**
 * Confirm herdr accepts a `pane.graphics.stream` layer. A missing ack is
 * treated as success — the open reply may be silent — so only an explicit
 * error reply rejects it. The probe stream is closed immediately, which makes
 * herdr drop its layer.
 */
export async function probeHerdrSet(address: HerdrAddress): Promise<{ ok: boolean; reason?: string }> {
  return new Promise((resolve) => {
    const client = createConnection(address.socket);
    let text = "";
    let settled = false;
    const finish = (result: { ok: boolean; reason?: string }) => {
      if (settled) return;
      settled = true;
      client.destroy();
      resolve(result);
    };
    client.on("connect", () =>
      client.write(JSON.stringify({ id: "astroshot-review", method: "pane.graphics.stream", params: { pane_id: address.pane, layer_id: "astroshot-review-probe", z_index: -1 } }) + "\n"),
    );
    client.on("data", (chunk) => {
      text += chunk.toString();
      const end = text.indexOf("\n");
      if (end < 0) return;
      try {
        const reply = JSON.parse(text.slice(0, end)) as Reply;
        finish(reply.error ? { ok: false, reason: reasonFor(reply) } : { ok: true });
      } catch {
        finish({ ok: true });
      }
    });
    client.on("error", () => finish({ ok: true }));
    setTimeout(() => finish({ ok: true }), 600);
  });
}

export interface HerdrDiscovery {
  ok: boolean;
  cellWidth?: number;
  cellHeight?: number;
  maxLayers?: number;
  /** Actionable reason when graphics can't be used yet. */
  reason?: string;
}

function reasonFor(reply: Reply): string {
  if (reply.error?.code === "feature_disabled") {
    return "herdr image rendering is off — set [experimental] kitty_graphics = true in ~/.config/herdr/config.toml, run `herdr server reload-config`, then reattach the herdr client";
  }
  if (reply.error?.code === "cell_size_unavailable") {
    return "herdr hasn't reported pixel size — detach and reattach the herdr client once (its graphics setting latches at startup)";
  }
  if (reply.error) return `herdr graphics unavailable (${reply.error.code})`;
  return "herdr returned no pixel dimensions for this pane";
}

/** Poll `pane.graphics.info` until herdr reports the cell size, or give up. */
export async function discoverHerdr(
  address: HerdrAddress,
  options: { timeoutMs?: number; retryMs?: number; onWaiting?: () => void } = {},
): Promise<HerdrDiscovery> {
  const deadline = Date.now() + (options.timeoutMs ?? 5000);
  let announced = false;
  for (;;) {
    let reply: Reply;
    try {
      reply = await request(address.socket, "pane.graphics.info", { pane_id: address.pane });
    } catch (error) {
      return { ok: false, reason: error instanceof Error ? error.message : String(error) };
    }
    if (reply.error?.code === "cell_size_unavailable" && Date.now() < deadline) {
      if (!announced) {
        announced = true;
        options.onWaiting?.();
      }
      await delay(Math.min(options.retryMs ?? 150, Math.max(1, deadline - Date.now())));
      continue;
    }
    if (reply.error || !reply.result?.cell_width_px || !reply.result?.cell_height_px) {
      return { ok: false, reason: reasonFor(reply) };
    }
    return {
      ok: true,
      cellWidth: reply.result.cell_width_px,
      cellHeight: reply.result.cell_height_px,
      maxLayers: reply.result.max_layers_per_pane || 16,
    };
  }
}

/** Extract PNG pixel dimensions from the IHDR chunk. */
function pngSize(png: Buffer): { width: number; height: number } {
  return { width: png.readUInt32BE(16), height: png.readUInt32BE(20) };
}

export interface HerdrPlacement {
  col: number;
  row: number;
  cols: number;
  rows: number;
  z: number;
}

interface LayerStream {
  socket: Socket | null;
  opening: boolean;
  ready: boolean;
  z: number;
  /** Latest frame to send once the stream is ready or drained. */
  pending: { png: Buffer; placement: HerdrPlacement } | null;
  draining: boolean;
}

/**
 * Places images on herdr via one `pane.graphics.stream` connection PER layer.
 * herdr removes a stream's layer the moment its socket closes, so closing a
 * layer's connection — or the whole process dying — cleans up automatically,
 * with no persistent server-side state to leak (the trap of pane.graphics.set).
 */
export class HerdrSink {
  private readonly layers = new Map<string, LayerStream>();
  /** Every socket opened and not yet closed, whether or not its layer is still mapped. */
  private readonly connections = new Set<Socket>();
  private disposed = false;
  private generationCount = 0;

  constructor(
    private readonly address: HerdrAddress,
    private readonly onError: (error: Error) => void,
  ) {}

  /** Bumps when a layer stream drops unexpectedly, so callers re-send. */
  get generation(): number {
    return this.generationCount;
  }

  set(layerId: string, png: Buffer, _imageWidth: number, _imageHeight: number, placement: HerdrPlacement): void {
    if (this.disposed) return;
    let layer = this.layers.get(layerId);
    if (!layer) {
      layer = { socket: null, opening: false, ready: false, z: placement.z, pending: null, draining: false };
      this.layers.set(layerId, layer);
    }
    layer.pending = { png, placement };
    if (layer.ready && !layer.draining) {
      this.flush(layerId, layer);
    } else if (!layer.opening && !layer.socket) {
      this.open(layerId, layer);
    }
  }

  clear(layerId: string): void {
    const layer = this.layers.get(layerId);
    if (!layer) return;
    this.layers.delete(layerId);
    layer.pending = null;
    layer.opening = false;
    layer.ready = false;
    const socket = layer.socket;
    layer.socket = null;
    // Closing the connection makes herdr drop this layer. This must happen
    // even mid-handshake: a connection that outlives its layer keeps a ghost
    // layer in herdr and keeps this process alive after the tray quits.
    if (socket) this.close(socket);
  }

  private close(socket: Socket): void {
    this.connections.delete(socket);
    try {
      socket.destroy();
    } catch {
      // already gone
    }
  }

  clearAll(): void {
    for (const layerId of [...this.layers.keys()]) this.clear(layerId);
  }

  private open(layerId: string, layer: LayerStream): void {
    const client = createConnection(this.address.socket);
    // The layer owns its socket from the first moment, not from the open ack:
    // clear() and dispose() close whatever is here, handshake or not.
    layer.opening = true;
    layer.ready = false;
    layer.socket = client;
    this.connections.add(client);
    let text = "";
    let opened = false;
    // Still the mapped layer's current socket, on a live sink.
    const current = () => !this.disposed && this.layers.get(layerId) === layer && layer.socket === client;
    const markOpen = () => {
      if (opened) return;
      opened = true;
      if (!current()) {
        this.close(client);
        return;
      }
      layer.opening = false;
      layer.ready = true;
      this.flush(layerId, layer);
    };
    client.on("connect", () => {
      client.write(JSON.stringify({ id: "astroshot-review", method: "pane.graphics.stream", params: { pane_id: this.address.pane, layer_id: layerId, z_index: layer.z } }) + "\n");
    });
    client.on("data", (chunk) => {
      text += chunk.toString();
      let end: number;
      while ((end = text.indexOf("\n")) >= 0) {
        const line = text.slice(0, end);
        text = text.slice(end + 1);
        let reply: Reply;
        try {
          reply = JSON.parse(line) as Reply;
        } catch {
          continue;
        }
        if (reply.error) {
          this.onError(new Error(`herdr stream ${layerId}: ${reply.error.code}`));
          continue;
        }
        markOpen();
      }
    });
    const drop = (reason: string) => {
      clearTimeout(grace);
      this.connections.delete(client);
      // A socket that clear() already detached says nothing about the layer.
      if (layer.socket !== client) return;
      layer.socket = null;
      layer.ready = false;
      layer.opening = false;
      // Unexpected drop (not from clear/dispose): let callers re-send everything.
      if (this.layers.get(layerId) === layer && !this.disposed) {
        this.generationCount += 1;
        this.onError(new Error(`herdr stream ${layerId} ${reason}`));
      }
    };
    client.on("error", () => drop("error"));
    client.on("close", () => drop("closed"));
    // If the open reply never comes, assume success after a short grace so a
    // silent-ack build still renders.
    const grace = setTimeout(markOpen, 400);
  }

  private flush(layerId: string, layer: LayerStream): void {
    const socket = layer.socket;
    const frame = layer.pending;
    if (!socket || !frame) return;
    layer.pending = null;
    const size = pngSize(frame.png);
    const header = {
      format: "png",
      image_width: size.width,
      image_height: size.height,
      data_length: frame.png.length,
      placement: {
        viewport_col: frame.placement.col,
        viewport_row: frame.placement.row,
        grid_cols: frame.placement.cols,
        grid_rows: frame.placement.rows,
      },
    };
    let ok = false;
    try {
      ok = socket.write(Buffer.concat([Buffer.from(JSON.stringify(header) + "\n"), frame.png]));
    } catch (error) {
      this.onError(error instanceof Error ? error : new Error(String(error)));
      return;
    }
    if (!ok) {
      // Backpressure: wait for drain, then send only the newest pending frame.
      layer.draining = true;
      socket.once("drain", () => {
        layer.draining = false;
        if (layer.pending) this.flush(layerId, layer);
      });
    } else if (layer.pending) {
      this.flush(layerId, layer);
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const socket of [...this.connections]) this.close(socket);
    this.layers.clear();
  }
}
