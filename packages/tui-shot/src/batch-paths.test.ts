import path from "node:path";

import { describe, expect, it } from "vitest";

import { resolveBatchOutputPaths } from "./batch-paths.js";

describe("resolveBatchOutputPaths", () => {
  it("preserves safe nested output paths under --out-dir", () => {
    expect(
      resolveBatchOutputPaths(
        [
          { fixture: "one.tsx", out: "first/screen.png" },
          { fixture: "two.tsx", out: "second/screen.png" },
        ],
        "/project/shots",
        "/artifacts",
      ),
    ).toEqual([
      path.resolve("/artifacts/first/screen.png"),
      path.resolve("/artifacts/second/screen.png"),
    ]);
  });

  it.each([
    "../escape.png",
    "nested/../../escape.png",
    "/absolute.png",
    "C:\\absolute.png",
    "..\\escape.png",
  ])("rejects unsafe --out-dir path %s", (output) => {
    expect(() =>
      resolveBatchOutputPaths(
        [{ fixture: "one.tsx", out: output }],
        "/project/shots",
        "/artifacts",
      ),
    ).toThrow(/safe relative path/);
  });

  it("rejects normalized and case-only destination collisions", () => {
    expect(() =>
      resolveBatchOutputPaths(
        [
          { fixture: "one.tsx", out: "flow/screen.png" },
          { fixture: "two.tsx", out: "FLOW/../flow/SCREEN.png" },
        ],
        "/project/shots",
        "/artifacts",
      ),
    ).toThrow(/same destination/);
  });
});
