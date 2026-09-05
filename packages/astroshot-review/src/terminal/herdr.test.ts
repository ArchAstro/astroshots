import net from "node:net";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { HerdrSink, discoverHerdr, herdrAddress, probeHerdrSet } from "./herdr.js";

interface Recorded {
  method: string;
  params: Record<string, unknown>;
}

/** A fake herdr socket that records requests and replies per a scripted plan. */
function fakeHerdr(reply: (method: string, count: number) => unknown) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-fake-"));
  const socketPath = path.join(dir, "api.sock");
  const requests: Recorded[] = [];
  const counts = new Map<string, number>();
  const server = net.createServer((socket) => {
    let buf = "";
    socket.on("data", (chunk) => {
      buf += chunk.toString();
      let end: number;
      while ((end = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, end);
        buf = buf.slice(end + 1);
        try {
          const message = JSON.parse(line) as Recorded;
          requests.push(message);
          const count = (counts.get(message.method) ?? 0) + 1;
          counts.set(message.method, count);
          const body = reply(message.method, count);
          if (body !== undefined) socket.write(JSON.stringify(body) + "\n");
        } catch {
          // ignore
        }
      }
    });
  });
  return {
    socketPath,
    requests,
    listen: () => new Promise<void>((resolve) => server.listen(socketPath, resolve)),
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

let server: ReturnType<typeof fakeHerdr>;

afterEach(async () => {
  if (server) await server.close();
});

describe("herdr address", () => {
  it("requires HERDR_ENV and the pane context", () => {
    expect(herdrAddress({} as NodeJS.ProcessEnv)).toBeNull();
    expect(herdrAddress({ HERDR_ENV: "1" } as NodeJS.ProcessEnv)).toBeNull();
    expect(herdrAddress({ HERDR_ENV: "1", HERDR_SOCKET_PATH: "/s", HERDR_PANE_ID: "w1:p2" } as NodeJS.ProcessEnv)).toEqual({
      socket: "/s",
      pane: "w1:p2",
    });
  });
});

describe("discoverHerdr", () => {
  it("retries while the cell size is negotiating, then returns it", async () => {
    server = fakeHerdr((method, count) => {
      if (method !== "pane.graphics.info") return { error: { code: "bad", message: "x" } };
      if (count < 3) return { error: { code: "cell_size_unavailable", message: "negotiating" } };
      return { result: { type: "pane_graphics_info", cell_width_px: 8, cell_height_px: 16, max_layers_per_pane: 16 } };
    });
    await server.listen();
    const result = await discoverHerdr({ socket: server.socketPath, pane: "w1:p2" }, { retryMs: 5, timeoutMs: 2000 });
    expect(result.ok).toBe(true);
    expect(result.cellWidth).toBe(8);
    expect(result.cellHeight).toBe(16);
  });

  it("reports an actionable reason when the feature is disabled", async () => {
    server = fakeHerdr(() => ({ error: { code: "feature_disabled", message: "off" } }));
    await server.listen();
    const result = await discoverHerdr({ socket: server.socketPath, pane: "w1:p2" }, { timeoutMs: 200 });
    expect(result.ok).toBe(false);
    expect(result.reason).toMatch(/kitty_graphics/);
  });

  it("reports the reattach hint when the cell size never arrives", async () => {
    server = fakeHerdr(() => ({ error: { code: "cell_size_unavailable", message: "no size" } }));
    await server.listen();
    const result = await discoverHerdr({ socket: server.socketPath, pane: "w1:p2" }, { retryMs: 5, timeoutMs: 60 });
    expect(result.ok).toBe(false);
    expect(result.reason).toMatch(/reattach/i);
  });
});

describe("probeHerdrSet", () => {
  it("accepts a silent set (no ack) as supported", async () => {
    server = fakeHerdr((method) => (method === "pane.graphics.set" ? undefined : { result: {} }));
    await server.listen();
    const result = await probeHerdrSet({ socket: server.socketPath, pane: "w1:p2" });
    expect(result.ok).toBe(true);
  });

  it("rejects an explicit error", async () => {
    server = fakeHerdr((method) => (method === "pane.graphics.set" ? { error: { code: "unknown_method", message: "no" } } : { result: {} }));
    await server.listen();
    const result = await probeHerdrSet({ socket: server.socketPath, pane: "w1:p2" });
    expect(result.ok).toBe(false);
  });
});

describe("HerdrSink", () => {
  it("emits pane.graphics.set with a 0-based cell placement and clears layers", async () => {
    server = fakeHerdr(() => undefined);
    await server.listen();
    const sink = new HerdrSink({ socket: server.socketPath, pane: "w1:p2" }, () => undefined);
    const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==", "base64");
    sink.set("astro-1", png, 40, 30, { col: 4, row: 7, cols: 10, rows: 5, z: 0 });
    sink.clear("astro-1");
    await new Promise((resolve) => setTimeout(resolve, 100));
    const set = server.requests.find((request) => request.method === "pane.graphics.set");
    expect(set).toBeTruthy();
    expect(set!.params).toMatchObject({
      pane_id: "w1:p2",
      layer_id: "astro-1",
      format: "png",
      image_width: 40,
      image_height: 30,
      z_index: 0,
      placement: { viewport_col: 4, viewport_row: 7, grid_cols: 10, grid_rows: 5 },
    });
    expect(Buffer.from(set!.params.data_base64 as string, "base64").equals(png)).toBe(true);
    expect(server.requests.some((request) => request.method === "pane.graphics.clear" && request.params.layer_id === "astro-1")).toBe(true);
    sink.dispose();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(server.requests.some((request) => request.method === "pane.graphics.clear" && request.params.layer_id === undefined)).toBe(true);
  });
});
