import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";

import sharp from "sharp";
import { describe, expect, it } from "vitest";

const testRequire = createRequire(import.meta.url);
const PROCESS_TIMEOUT_MS = 60_000;

function makeIsolatedInkConsumer(consumerDir: string): string {
  const installedInk = path.resolve(
    path.dirname(testRequire.resolve("ink")),
    "..",
  );
  const installedModules = path.dirname(installedInk);
  const consumerModules = path.join(consumerDir, "node_modules");
  fs.mkdirSync(consumerModules, { recursive: true });
  fs.writeFileSync(
    path.join(consumerDir, "package.json"),
    JSON.stringify({ private: true, type: "module" }),
  );

  const copiedPackages = [
    "ink",
    "react",
    "react-reconciler",
    "scheduler",
    "chalk",
  ];
  for (const packageName of copiedPackages) {
    fs.cpSync(
      path.join(installedModules, packageName),
      path.join(consumerModules, packageName),
      { recursive: true },
    );
  }

  const inkPackage = JSON.parse(
    fs.readFileSync(path.join(installedInk, "package.json"), "utf8"),
  ) as { dependencies: Record<string, string> };
  for (const packageName of Object.keys(inkPackage.dependencies)) {
    if (copiedPackages.includes(packageName)) continue;
    const destination = path.join(consumerModules, packageName);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.symlinkSync(path.join(installedModules, packageName), destination, "junction");
  }

  const fixturePath = path.join(consumerDir, "hook-fixture.tsx");
  fs.writeFileSync(
    fixturePath,
    `import React, { useState } from "react";
import { Box, Text } from "ink";

function HookScreen() {
  const [status] = useState("Isolated React hook rendered");
  return (
    <Box borderStyle="round" borderColor="#b9a8ff">
      <Text color="#b9a8ff">{status}</Text>
    </Box>
  );
}

export default {
  cols: 42,
  rows: 6,
  scale: 1,
  expectText: ["Isolated React hook rendered"],
  component: <HookScreen />,
};
`,
  );
  return fixturePath;
}

describe("tui-shot CLI boundary", () => {
  it("renders a separate consumer's hook-using Ink runtime through the published bin", async () => {
    // Setup a disposable destination outside the package, as an npx caller would.
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "tui-shot-e2e-"));
    const outPath = path.join(tempDir, "basic.png");
    try {
      const fixturePath = makeIsolatedInkConsumer(
        path.join(tempDir, "consumer"),
      );
      // Cross the actual bin, TSX fixture loader, Ink, xterm, and Chromium boundaries.
      const stdout = execFileSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "shot",
          fixturePath,
          "-o",
          outPath,
        ],
        {
          cwd: path.resolve("."),
          env: process.env,
          encoding: "utf8",
          timeout: PROCESS_TIMEOUT_MS,
        },
      );

      // The fixture's expectText validates semantic content before capture; inspect
      // the resulting pixels to prove the browser emitted a real, non-flat PNG.
      expect(stdout).toContain(`wrote ${outPath}`);
      const image = sharp(outPath);
      const [metadata, stats] = await Promise.all([
        image.metadata(),
        image.stats(),
      ]);
      expect(metadata.format).toBe("png");
      expect(metadata.width).toBeGreaterThan(300);
      expect(metadata.height).toBeGreaterThan(100);
      expect(fs.statSync(outPath).size).toBeGreaterThan(4_000);
      expect(stats.channels.some((channel) => channel.max > channel.min)).toBe(
        true,
      );
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it("keeps nested batch destinations distinct under --out-dir", () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "tui-shot-batch-"));
    const outDir = path.join(tempDir, "artifacts");
    const manifestPath = path.join(tempDir, "batch.json");
    fs.writeFileSync(
      manifestPath,
      JSON.stringify({
        shots: [
          {
            fixture: path.resolve("fixtures/basic.tsx"),
            out: "first/screen.png",
          },
          {
            fixture: path.resolve("fixtures/basic.tsx"),
            out: "second/screen.png",
          },
        ],
      }),
    );

    try {
      const stdout = execFileSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "batch",
          manifestPath,
          "--out-dir",
          outDir,
        ],
        {
          cwd: path.resolve("."),
          env: process.env,
          encoding: "utf8",
          timeout: PROCESS_TIMEOUT_MS,
        },
      );

      expect(stdout).toContain("done: 2/2 shots");
      expect(fs.statSync(path.join(outDir, "first/screen.png")).size).toBeGreaterThan(
        4_000,
      );
      expect(
        fs.statSync(path.join(outDir, "second/screen.png")).size,
      ).toBeGreaterThan(4_000);
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });
});
