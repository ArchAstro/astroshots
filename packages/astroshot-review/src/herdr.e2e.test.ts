/**
 * Drives the built tray against a fake herdr socket and proves it renders
 * through herdr's pane-graphics API: one info handshake, then pane.graphics.set
 * calls carrying real PNG bytes at 0-based pane-cell placements.
 */
import net from "node:net";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const demoFixtures = path.resolve(packageRoot, "../astroshot/fixtures/demo");
const executable = path.join(packageRoot, "bin", "astroshot-review.mjs");

let dir: string;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-e2e-"));
  const feature = path.join(dir, "root", "demo-app", ".astroshot", "checkout");
  fs.mkdirSync(feature, { recursive: true });
  fs.copyFileSync(path.join(demoFixtures, "welcome.png"), path.join(feature, "0001-welcome.png"));
  fs.copyFileSync(path.join(demoFixtures, "next-steps.png"), path.join(feature, "0002-next-steps.png"));
  fs.writeFileSync(
    path.join(feature, "manifest.json"),
    JSON.stringify({ version: 1, run_id: "r", status: "pass", shots: [
      { id: "0001", file: "0001-welcome.png", title: "Welcome" },
      { id: "0002", file: "0002-next-steps.png", title: "Next steps" },
    ] }),
  );
});

afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

describe("herdr pane-graphics transport", () => {
  it("streams real PNG frames to per-layer herdr streams at pane-cell placements", async () => {
    const socketPath = path.join(dir, "api.sock");
    const frames: Array<{ layer: string; placement: Record<string, number>; width: number; height: number; bytes: number }> = [];
    let infoRequests = 0;
    const server = net.createServer((socket) => {
      let layerId: string | null = null;
      let buf = Buffer.alloc(0);
      let expectBytes = 0;
      let header: { image_width: number; image_height: number; placement: Record<string, number> } | null = null;
      socket.on("data", (chunk) => {
        buf = Buffer.concat([buf, chunk]);
        for (;;) {
          if (expectBytes > 0) {
            if (buf.length < expectBytes) return;
            buf = buf.subarray(expectBytes);
            if (layerId && layerId !== "astroshot-review-probe" && header) {
              frames.push({ layer: layerId, placement: header.placement, width: header.image_width, height: header.image_height, bytes: expectBytes });
            }
            expectBytes = 0;
            header = null;
            continue;
          }
          const nl = buf.indexOf(0x0a);
          if (nl < 0) return;
          const line = buf.subarray(0, nl).toString();
          buf = buf.subarray(nl + 1);
          let message: { method?: string; params?: { layer_id?: string }; format?: string; data_length?: number; image_width?: number; image_height?: number; placement?: Record<string, number> };
          try {
            message = JSON.parse(line);
          } catch {
            continue;
          }
          if (message.method === "pane.graphics.info") {
            infoRequests += 1;
            socket.write(JSON.stringify({ result: { type: "pane_graphics_info", cell_width_px: 9, cell_height_px: 20, max_layers_per_pane: 16 } }) + "\n");
          } else if (message.method === "pane.graphics.stream") {
            layerId = message.params?.layer_id ?? null;
            socket.write(JSON.stringify({ result: { type: "ok" } }) + "\n");
          } else if (typeof message.format === "string" && typeof message.data_length === "number") {
            expectBytes = message.data_length;
            header = { image_width: message.image_width ?? 0, image_height: message.image_height ?? 0, placement: message.placement ?? {} };
          }
        }
      });
    });
    await new Promise<void>((resolve) => server.listen(socketPath, resolve));

    const { spawn } = await import("node-pty");
    const child = spawn(process.execPath, [executable, "--root", path.join(dir, "root"), "--no-index"], {
      name: "xterm-256color",
      cols: 140,
      rows: 40,
      cwd: dir,
      env: {
        ...process.env,
        HERDR_ENV: "1",
        HERDR_SOCKET_PATH: socketPath,
        HERDR_PANE_ID: "w9:p9",
        ASTROSHOT_REVIEW_CACHE_DIR: path.join(dir, "cache"),
        TERM: "xterm-256color",
        COLORTERM: "truecolor",
      },
    });
    let output = "";
    child.onData((data) => {
      output += data;
    });

    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline && frames.length < 3) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    child.kill();
    await new Promise((resolve) => setTimeout(resolve, 200));
    await new Promise<void>((resolve) => server.close(() => resolve()));

    expect(infoRequests).toBeGreaterThanOrEqual(1);
    expect(frames.length).toBeGreaterThanOrEqual(3);
    for (const frame of frames) {
      expect(frame.bytes).toBeGreaterThan(100);
      expect(frame.width).toBeGreaterThan(0);
      expect(frame.placement.viewport_col ?? -1).toBeGreaterThanOrEqual(0);
      expect(frame.placement.viewport_row ?? -1).toBeGreaterThanOrEqual(0);
      expect(frame.placement.grid_cols ?? 0).toBeGreaterThan(0);
      expect(frame.placement.grid_rows ?? 0).toBeGreaterThan(0);
    }
    // The detail preview (right pane) is wider than a left-rail thumbnail.
    expect(frames.some((f) => (f.placement.grid_cols ?? 0) >= 20)).toBe(true);
    expect(output).toContain("\x1b[?1049h");
  }, 30_000);
});
