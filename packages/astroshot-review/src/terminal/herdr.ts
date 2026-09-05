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
 * Confirm herdr accepts `pane.graphics.set` (react-kitty only exercises the
 * whole-pane stream API). A missing ack is treated as success — some builds
 * ack, some stay silent — so only an explicit error reply rejects it.
 */
export async function probeHerdrSet(address: HerdrAddress): Promise<{ ok: boolean; reason?: string }> {
  try {
    const reply = await Promise.race([
      request(address.socket, "pane.graphics.set", {
        pane_id: address.pane,
        layer_id: "astroshot-review-probe",
        format: "png",
        image_width: 1,
        image_height: 1,
        data_base64: PROBE_PNG.toString("base64"),
        z_index: -1,
        placement: { viewport_col: 0, viewport_row: 0, grid_cols: 1, grid_rows: 1 },
      }),
      delay(600).then(() => ({ result: { type: "assumed-ok" } }) as Reply),
    ]);
    if (reply.error) return { ok: false, reason: reasonFor(reply) };
    // Best-effort cleanup; ignore the result.
    void request(address.socket, "pane.graphics.clear", { pane_id: address.pane, layer_id: "astroshot-review-probe" }).catch(() => undefined);
    return { ok: true };
  } catch {
    // A transport hiccup shouldn't downgrade a working setup.
    return { ok: true };
  }
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

interface QueuedOp {
  method: string;
  params: Record<string, unknown>;
}

/**
 * Places images on herdr layers over the pane. Uses one persistent connection
 * with newline-delimited requests; layers persist server-side, so a dropped
 * connection is reconnected on the next op without losing placements.
 */
export class HerdrSink {
  private client: Socket | null = null;
  private connecting = false;
  private queue: QueuedOp[] = [];
  private disposed = false;
  private readBuffer = "";
  private hasConnected = false;
  private generationCount = 0;

  /** Bumps whenever a NEW connection replaces a dropped one, so callers re-send layers. */
  get generation(): number {
    return this.generationCount;
  }

  constructor(
    private readonly address: HerdrAddress,
    private readonly onError: (error: Error) => void,
  ) {}

  private ensureClient(): void {
    if (this.client || this.connecting || this.disposed) return;
    this.connecting = true;
    const client = createConnection(this.address.socket);
    client.on("connect", () => {
      this.connecting = false;
      this.client = client;
      if (this.hasConnected) this.generationCount += 1;
      this.hasConnected = true;
      const pending = this.queue;
      this.queue = [];
      for (const op of pending) this.send(op);
    });
    client.on("data", (chunk) => {
      this.readBuffer += chunk.toString();
      let end: number;
      while ((end = this.readBuffer.indexOf("\n")) >= 0) {
        const line = this.readBuffer.slice(0, end);
        this.readBuffer = this.readBuffer.slice(end + 1);
        try {
          const reply = JSON.parse(line) as Reply;
          if (reply.error) this.onError(new Error(`herdr graphics: ${reply.error.code}`));
        } catch {
          // Ignore unparseable lines; herdr may interleave acks.
        }
      }
    });
    const drop = () => {
      if (this.client === client) this.client = null;
      this.connecting = false;
    };
    client.on("error", (error) => {
      drop();
      if (!this.disposed) this.onError(error);
    });
    client.on("close", drop);
  }

  private send(op: QueuedOp): void {
    if (this.disposed) return;
    if (!this.client) {
      this.queue.push(op);
      this.ensureClient();
      return;
    }
    try {
      this.client.write(JSON.stringify({ id: "astroshot-review", method: op.method, params: op.params }) + "\n");
    } catch (error) {
      this.queue.push(op);
      this.client = null;
      this.ensureClient();
      if (!this.disposed) this.onError(error instanceof Error ? error : new Error(String(error)));
    }
  }

  /** Place a PNG on `layerId` at a 0-based cell box within the pane. */
  set(
    layerId: string,
    png: Buffer,
    imageWidth: number,
    imageHeight: number,
    placement: { col: number; row: number; cols: number; rows: number; z: number },
  ): void {
    this.send({
      method: "pane.graphics.set",
      params: {
        pane_id: this.address.pane,
        layer_id: layerId,
        format: "png",
        image_width: imageWidth,
        image_height: imageHeight,
        data_base64: png.toString("base64"),
        z_index: placement.z,
        placement: {
          viewport_col: placement.col,
          viewport_row: placement.row,
          grid_cols: placement.cols,
          grid_rows: placement.rows,
        },
      },
    });
  }

  clear(layerId: string): void {
    this.send({ method: "pane.graphics.clear", params: { pane_id: this.address.pane, layer_id: layerId } });
  }

  clearAll(): void {
    this.send({ method: "pane.graphics.clear", params: { pane_id: this.address.pane } });
  }

  dispose(): void {
    if (this.disposed) return;
    this.clearAll();
    this.disposed = true;
    // Flush the clear before closing.
    const client = this.client;
    if (client) {
      try {
        client.end();
      } catch {
        client.destroy();
      }
    }
    this.client = null;
  }
}
