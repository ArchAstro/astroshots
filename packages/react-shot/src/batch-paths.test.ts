import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { describe, expect, it } from "vitest";

import { resolveBatchOutputPaths } from "./batch-paths.js";

describe("resolveBatchOutputPaths", () => {
  it("resolves distinct manifest outputs", () => {
    expect(
      resolveBatchOutputPaths(
        [
          { fixture: "one.tsx", out: "first/screen.png" },
          { fixture: "two.tsx", out: "second/screen.png" },
        ],
        "/shots",
      ),
    ).toEqual([
      path.resolve("/shots/first/screen.png"),
      path.resolve("/shots/second/screen.png"),
    ]);
  });

  it("rejects normalized duplicate destinations", () => {
    expect(() =>
      resolveBatchOutputPaths(
        [
          { fixture: "one.tsx", out: "result/screen.png" },
          { fixture: "two.tsx", out: "result/nested/../screen.png" },
        ],
        "/shots",
      ),
    ).toThrow(/same destination/);
  });

  it("rejects case-only destination collisions on every platform", () => {
    expect(() =>
      resolveBatchOutputPaths(
        [
          { fixture: "one.tsx", out: "result/screen.png" },
          { fixture: "two.tsx", out: "RESULT/SCREEN.PNG" },
        ],
        "/shots",
      ),
    ).toThrow(/same destination/);
  });

  it("rejects destinations aliased through a symlinked directory", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "react-shot-paths-"));
    const real = path.join(root, "real");
    const alias = path.join(root, "alias");
    fs.mkdirSync(real);
    fs.symlinkSync(real, alias, "dir");
    try {
      expect(() =>
        resolveBatchOutputPaths(
          [
            { fixture: "one.tsx", out: "real/screen.png" },
            { fixture: "two.tsx", out: "alias/screen.png" },
          ],
          root,
        ),
      ).toThrow(/same destination/);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});
