import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const packageRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

describe("published react-shot CLI", () => {
  it("renders a TSX release card through Vite and Chromium into a cropped RGBA PNG", () => {
    // Setup: use the same compiled bin that npm exposes, not an in-process API.
    const outputDirectory = fs.mkdtempSync(
      path.join(os.tmpdir(), "react-shot-cli-e2e-"),
    );
    const outputPath = path.join(outputDirectory, "release-card.png");
    const fixturePath = path.join(packageRoot, "fixtures", "cli-e2e.tsx");
    const binPath = path.join(packageRoot, "bin", "react-shot.mjs");

    // Boundary crossing: a child Node process starts Vite, which serves the
    // TSX module to a real Playwright-managed Chromium process.
    try {
      const result = spawnSync(
        process.execPath,
        [binPath, "shot", fixturePath, "--out", outputPath],
        {
          cwd: packageRoot,
          encoding: "utf8",
          timeout: 75_000,
        },
      );

      // Observable outcomes: the public command succeeds and writes a genuine
      // element-cropped, alpha-capable PNG rather than the 800x600 dimmer.
      expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0);
      expect(result.stdout).toContain(`wrote ${outputPath}`);
      const png = fs.readFileSync(outputPath);
      expect(png.subarray(0, 8)).toEqual(
        Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
      );
      expect(png.readUInt32BE(16)).toBe(400);
      expect(png.readUInt32BE(20)).toBeGreaterThan(60);
      expect(png.readUInt32BE(20)).toBeLessThan(180);
      expect(png[25]).toBe(6);
    } finally {
      fs.rmSync(outputDirectory, { recursive: true, force: true });
    }
  });

  it("rejects colliding batch outputs before capturing the first entry", () => {
    const tempDirectory = fs.mkdtempSync(
      path.join(os.tmpdir(), "react-shot-batch-preflight-"),
    );
    const fixturePath = path.join(packageRoot, "fixtures", "cli-e2e.tsx");
    const manifestPath = path.join(tempDirectory, "batch.json");
    const firstOutput = path.join(tempDirectory, "result", "screen.png");
    fs.writeFileSync(
      manifestPath,
      JSON.stringify({
        shots: [
          { fixture: fixturePath, out: "result/screen.png" },
          { fixture: fixturePath, out: "RESULT/SCREEN.PNG" },
        ],
      }),
    );

    try {
      const result = spawnSync(
        process.execPath,
        [
          path.join(packageRoot, "bin", "react-shot.mjs"),
          "batch",
          manifestPath,
        ],
        {
          cwd: packageRoot,
          encoding: "utf8",
          timeout: 75_000,
        },
      );

      expect(result.status).toBe(1);
      expect(result.stderr).toContain("same destination");
      expect(fs.existsSync(firstOutput)).toBe(false);
    } finally {
      fs.rmSync(tempDirectory, { recursive: true, force: true });
    }
  });
});
