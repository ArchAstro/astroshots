import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { closeSharedBrowser, takeTuiShot } from "./shot.js";

afterEach(closeSharedBrowser);

describe("tui-shot browser lifecycle", () => {
  it("waits for all queued captures before closeSharedBrowser resolves", async () => {
    const tempDirectory = fs.mkdtempSync(
      path.join(os.tmpdir(), "tui-shot-close-queue-"),
    );
    const firstOutput = path.join(tempDirectory, "first.png");
    const secondOutput = path.join(tempDirectory, "second.png");
    const fixturePath = path.resolve("fixtures/basic.tsx");

    try {
      // Queue both real Ink and Chromium captures without awaiting either.
      const firstCapture = takeTuiShot({
        fixturePath,
        outPath: firstOutput,
      });
      const secondCapture = takeTuiShot({
        fixturePath,
        outPath: secondOutput,
      });

      // Closing is a lifecycle barrier: when it resolves, earlier captures
      // must be complete and the shared browser must already be shut down.
      await closeSharedBrowser();
      expect(await firstCapture).toBe(firstOutput);
      expect(await secondCapture).toBe(secondOutput);
      expect(fs.statSync(firstOutput).size).toBeGreaterThan(4_000);
      expect(fs.statSync(secondOutput).size).toBeGreaterThan(4_000);
    } finally {
      fs.rmSync(tempDirectory, { recursive: true, force: true });
    }
  });
});
