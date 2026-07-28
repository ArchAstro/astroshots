import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import sharp from "sharp";
import { describe, expect, it } from "vitest";

describe("arbitrary PTY capture boundary", () => {
  it("drives a full-screen terminal process and captures the selected state", async () => {
    // Arrange a disposable output while keeping the fixture beside its real
    // child executable, so relative command paths exercise fixture semantics.
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-shot-e2e-"));
    const outPath = path.join(tempDir, "selected.png");
    try {
      // Cross the published bin into a real pseudoterminal. The child enters
      // the alternate screen, receives Down and Enter, and redraws using ANSI.
      const stdout = execFileSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "pty",
          path.resolve("fixtures/interactive-pty.yaml"),
          "-o",
          outPath,
        ],
        { cwd: path.resolve("."), env: process.env, encoding: "utf8" },
      );

      // The fixture's final wait and expectText prove the externally visible
      // selected state; PNG pixels prove xterm and Chromium captured it.
      expect(stdout).toContain(`wrote ${outPath}`);
      const image = sharp(outPath);
      const [metadata, stats] = await Promise.all([
        image.metadata(),
        image.stats(),
      ]);
      expect(metadata.format).toBe("png");
      expect(metadata.width).toBeGreaterThan(400);
      expect(metadata.height).toBeGreaterThan(150);
      expect(fs.statSync(outPath).size).toBeGreaterThan(4_000);
      expect(stats.channels.some((channel) => channel.max > channel.min)).toBe(
        true,
      );
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it("rejects a crashed terminal process even when its expected text rendered", () => {
    // Launch a real PTY child that draws a plausible final frame and exits 7.
    // This guards documentation capture from certifying a crash as success.
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-crash-e2e-"));
    const outPath = path.join(tempDir, "crash.png");
    try {
      const result = spawnSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "pty",
          path.resolve("fixtures/crashing-pty.yaml"),
          "-o",
          outPath,
        ],
        { cwd: path.resolve("."), env: process.env, encoding: "utf8" },
      );

      // The visible assertion passes, but the process boundary is authoritative.
      expect(result.status).toBe(1);
      expect(result.stderr).toContain("exited with code 7 before capture");
      expect(fs.existsSync(outPath)).toBe(false);
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });
});
