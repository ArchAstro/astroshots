/** A PNG of the tray through `astroshot pty` with graphics emulation; needs Chromium. */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { takePtyShot } from "@archastro/tui-shot";
import { describe, expect, it } from "vitest";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const demoFixtures = path.resolve(packageRoot, "../astroshot/fixtures/demo");

describe("astroshot pty captures the review tray with pictures", () => {
  it("renders thumbnails into the PNG", async () => {
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-review-capture-"));
    try {
      const feature = path.join(temp, "root", "demo-app", ".astroshot", "welcome");
      fs.mkdirSync(feature, { recursive: true });
      fs.copyFileSync(path.join(demoFixtures, "welcome.png"), path.join(feature, "0001-welcome.png"));
      const fixture = path.join(temp, "review.yaml");
      fs.writeFileSync(
        fixture,
        [
          "version: 1",
          "command: node",
          `args: [${JSON.stringify(path.join(packageRoot, "bin", "astroshot-review.mjs"))}, --root, ${JSON.stringify(path.join(temp, "root"))}, --no-index]`,
          "cols: 120",
          "rows: 30",
          "scale: 1",
          "graphics: kitty",
          "timeoutMs: 40000",
          "settleMs: 1500",
          "env:",
          `  ASTROSHOT_REVIEW_CACHE_DIR: ${JSON.stringify(path.join(temp, "cache"))}`,
          "actions:",
          "  - waitFor: Unseen (1)",
          "  - pauseMs: 2000",
          "expectText: [Astroshots, welcome · Welcome]",
        ].join("\n"),
      );
      const outPath = path.join(temp, "review.png");
      let written: string;
      try {
        written = await takePtyShot({ fixturePath: fixture, outPath });
      } catch (error) {
        if (error instanceof Error && /Chromium is not installed/.test(error.message)) return;
        throw error;
      }
      expect(fs.existsSync(written)).toBe(true);
      // A frame with a real thumbnail and preview is far larger than text alone.
      expect(fs.statSync(written).size).toBeGreaterThan(40_000);
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
  }, 90_000);
});
