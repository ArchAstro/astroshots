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

interface SetParams {
  layer_id: string;
  image_width: number;
  image_height: number;
  data_base64: string;
  z_index: number;
  placement: { viewport_col: number; viewport_row: number; grid_cols: number; grid_rows: number };
}

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
  it("hands herdr real PNG layers at pane-cell placements", async () => {
    const socketPath = path.join(dir, "api.sock");
    const sets: SetParams[] = [];
    let infoRequests = 0;
    const server = net.createServer((socket) => {
      let buf = "";
      socket.on("data", (chunk) => {
        buf += chunk.toString();
        let end: number;
        while ((end = buf.indexOf("\n")) >= 0) {
          const line = buf.slice(0, end);
          buf = buf.slice(end + 1);
          let message: { method: string; params: SetParams };
          try {
            message = JSON.parse(line);
          } catch {
            continue;
          }
          if (message.method === "pane.graphics.info") {
            infoRequests += 1;
            socket.write(JSON.stringify({ result: { type: "pane_graphics_info", cell_width_px: 9, cell_height_px: 20, max_layers_per_pane: 16 } }) + "\n");
          } else if (message.method === "pane.graphics.set" && message.params.layer_id !== "astroshot-review-probe") {
            sets.push(message.params);
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
    while (Date.now() < deadline && sets.filter((s) => s.data_base64?.length > 100).length < 3) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    child.kill();
    await new Promise((resolve) => setTimeout(resolve, 200));
    await new Promise<void>((resolve) => server.close(() => resolve()));

    expect(infoRequests).toBeGreaterThanOrEqual(1);
    const withImages = sets.filter((s) => s.data_base64 && s.data_base64.length > 100);
    expect(withImages.length).toBeGreaterThanOrEqual(3);
    for (const params of withImages) {
      const png = Buffer.from(params.data_base64, "base64");
      expect(png.subarray(0, 8)).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
      expect(params.image_width).toBeGreaterThan(0);
      expect(params.placement.viewport_col).toBeGreaterThanOrEqual(0);
      expect(params.placement.viewport_row).toBeGreaterThanOrEqual(0);
      expect(params.placement.grid_cols).toBeGreaterThan(0);
      expect(params.placement.grid_rows).toBeGreaterThan(0);
    }
    // The detail preview (right pane) is wider than a left-rail thumbnail.
    expect(withImages.some((s) => s.placement.grid_cols >= 20)).toBe(true);
    expect(output).toContain("\x1b[?1049h");
  }, 30_000);
});
