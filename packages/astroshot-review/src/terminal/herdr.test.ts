import net from "node:net";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";

import { afterEach, describe, expect, it } from "vitest";

import { HerdrSink, discoverHerdr, herdrAddress, probeHerdrSet } from "./herdr.js";

const ONE_PX_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  "base64",
);

/** A fake herdr that answers info per a plan and records stream layers/frames. */
function fakeHerdr(options: {
  info?: (count: number) => unknown;
  ackStreamOpen?: boolean;
} = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-fake-"));
  const socketPath = path.join(dir, "api.sock");
  let infoCount = 0;
  const openedLayers: string[] = [];
  const frames: Array<{ layer: string; placement: Record<string, number>; bytes: number }> = [];
  const closedLayers: string[] = [];

  const server = net.createServer((socket) => {
    let layerId: string | null = null;
    let buf = Buffer.alloc(0);
    let expectBytes = 0;
    let pendingPlacement: Record<string, number> | null = null;

    const onLine = (line: string) => {
      let message: { method?: string; params?: Record<string, unknown> } & Record<string, unknown>;
      try {
        message = JSON.parse(line);
      } catch {
        return;
      }
      if (message.method === "pane.graphics.info") {
        infoCount += 1;
        const reply = options.info ? options.info(infoCount) : { result: { type: "pane_graphics_info", cell_width_px: 8, cell_height_px: 16, max_layers_per_pane: 16 } };
        socket.write(JSON.stringify(reply) + "\n");
      } else if (message.method === "pane.graphics.stream") {
        layerId = String((message.params as { layer_id?: string })?.layer_id ?? "");
        openedLayers.push(layerId);
        if (options.ackStreamOpen !== false) socket.write(JSON.stringify({ result: { type: "ok" } }) + "\n");
      } else if (typeof message.format === "string" && typeof message.data_length === "number") {
        // A stream frame header; the raw bytes follow.
        expectBytes = message.data_length as number;
        pendingPlacement = message.placement as Record<string, number>;
      }
    };

    socket.on("data", (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      for (;;) {
        if (expectBytes > 0) {
          if (buf.length < expectBytes) return;
          buf = buf.subarray(expectBytes);
          frames.push({ layer: layerId ?? "?", placement: pendingPlacement ?? {}, bytes: expectBytes });
          expectBytes = 0;
          pendingPlacement = null;
          continue;
        }
        const nl = buf.indexOf(0x0a);
        if (nl < 0) return;
        const line = buf.subarray(0, nl).toString();
        buf = buf.subarray(nl + 1);
        onLine(line);
      }
    });
    socket.on("close", () => {
      if (layerId) closedLayers.push(layerId);
    });
  });

  return {
    socketPath,
    openedLayers,
    frames,
    closedLayers,
    listen: () => new Promise<void>((resolve) => server.listen(socketPath, resolve)),
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

let server: ReturnType<typeof fakeHerdr>;
afterEach(async () => {
  if (server) await server.close();
});

const flush = () => new Promise((resolve) => setTimeout(resolve, 120));

describe("herdr address", () => {
  it("requires HERDR_ENV and the pane context", () => {
    expect(herdrAddress({} as NodeJS.ProcessEnv)).toBeNull();
    expect(herdrAddress({ HERDR_ENV: "1" } as NodeJS.ProcessEnv)).toBeNull();
    expect(herdrAddress({ HERDR_ENV: "1", HERDR_SOCKET_PATH: "/s", HERDR_PANE_ID: "w1:p2" } as NodeJS.ProcessEnv)).toEqual({ socket: "/s", pane: "w1:p2" });
  });
});

describe("discoverHerdr", () => {
  it("retries while the cell size is negotiating, then returns it", async () => {
    server = fakeHerdr({ info: (count) => (count < 3 ? { error: { code: "cell_size_unavailable", message: "negotiating" } } : { result: { type: "pane_graphics_info", cell_width_px: 8, cell_height_px: 16 } }) });
    await server.listen();
    const result = await discoverHerdr({ socket: server.socketPath, pane: "w1:p2" }, { retryMs: 5, timeoutMs: 2000 });
    expect(result.ok).toBe(true);
    expect(result.cellWidth).toBe(8);
    expect(result.cellHeight).toBe(16);
  });

  it("reports an actionable reason when the feature is disabled", async () => {
    server = fakeHerdr({ info: () => ({ error: { code: "feature_disabled", message: "off" } }) });
    await server.listen();
    const result = await discoverHerdr({ socket: server.socketPath, pane: "w1:p2" }, { timeoutMs: 200 });
    expect(result.ok).toBe(false);
    expect(result.reason).toMatch(/kitty_graphics/);
  });

  it("reports the reattach hint when the cell size never arrives", async () => {
    server = fakeHerdr({ info: () => ({ error: { code: "cell_size_unavailable", message: "no size" } }) });
    await server.listen();
    const result = await discoverHerdr({ socket: server.socketPath, pane: "w1:p2" }, { retryMs: 5, timeoutMs: 60 });
    expect(result.ok).toBe(false);
    expect(result.reason).toMatch(/reattach/i);
  });
});

describe("probeHerdrSet", () => {
  it("accepts a stream that opens (silent ack included)", async () => {
    server = fakeHerdr({ ackStreamOpen: false });
    await server.listen();
    const result = await probeHerdrSet({ socket: server.socketPath, pane: "w1:p2" });
    expect(result.ok).toBe(true);
  });
});

describe("HerdrSink (per-layer streams)", () => {
  it("opens one stream per layer and pushes a raw frame with the placement", async () => {
    server = fakeHerdr();
    await server.listen();
    const sink = new HerdrSink({ socket: server.socketPath, pane: "w1:p2" }, () => undefined);
    sink.set("astro-1", ONE_PX_PNG, 1, 1, { col: 4, row: 7, cols: 10, rows: 5, z: 0 });
    await flush();
    expect(server.openedLayers).toContain("astro-1");
    const frame = server.frames.find((f) => f.layer === "astro-1");
    expect(frame).toBeTruthy();
    expect(frame!.bytes).toBe(ONE_PX_PNG.length);
    expect(frame!.placement).toEqual({ viewport_col: 4, viewport_row: 7, grid_cols: 10, grid_rows: 5 });
    sink.dispose();
    await flush();
  });

  it("removes a layer by closing its stream (herdr drops it)", async () => {
    server = fakeHerdr();
    await server.listen();
    const sink = new HerdrSink({ socket: server.socketPath, pane: "w1:p2" }, () => undefined);
    sink.set("astro-2", ONE_PX_PNG, 1, 1, { col: 0, row: 0, cols: 2, rows: 1, z: 0 });
    await flush();
    sink.clear("astro-2");
    await flush();
    expect(server.closedLayers).toContain("astro-2");
  });

  it("closes every stream on dispose", async () => {
    server = fakeHerdr();
    await server.listen();
    const sink = new HerdrSink({ socket: server.socketPath, pane: "w1:p2" }, () => undefined);
    sink.set("astro-3", ONE_PX_PNG, 1, 1, { col: 0, row: 0, cols: 2, rows: 1, z: 0 });
    sink.set("astro-4", ONE_PX_PNG, 1, 1, { col: 0, row: 4, cols: 2, rows: 1, z: 0 });
    await flush();
    sink.dispose();
    await flush();
    expect(server.closedLayers.sort()).toEqual(["astro-3", "astro-4"]);
  });
});
