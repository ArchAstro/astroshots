import { describe, expect, it } from "vitest";

import { classifyPath, eventKey } from "./watcher.js";

describe("watch event routing", () => {
  it("routes images, sidecars, videos, friction logs, and trees", () => {
    expect(classifyPath("/w/.astroshot/f/0001-a.png")).toEqual({ kind: "shot", path: "/w/.astroshot/f/0001-a.png", featureDir: "/w/.astroshot/f", astroshotDir: "/w/.astroshot" });
    expect(classifyPath("/w/.astroshot/f/manifest.json")).toMatchObject({ kind: "feature", featureDir: "/w/.astroshot/f" });
    expect(classifyPath("/w/.astroshot/f/0001-a.webm")).toMatchObject({ kind: "feature" });
    expect(classifyPath("/w/.astroshot/friction-logs/x/runs/1/log.jsonl")).toEqual({ kind: "friction", astroshotDir: "/w/.astroshot" });
    expect(classifyPath("/w/.astroshot")).toEqual({ kind: "tree", astroshotDir: "/w/.astroshot" });
    expect(classifyPath("/w/.astroshot/f")).toMatchObject({ kind: "feature" });
    expect(classifyPath("/w/src/index.ts")).toBeNull();
    expect(classifyPath("/w/.astroshot/f/notes.txt")).toBeNull();
    expect(classifyPath("/w/.astroshot/f/deep/0001-a.png")).toBeNull();
  });

  it("keys events per subject", () => {
    expect(eventKey({ kind: "friction", astroshotDir: "/w/.astroshot" })).toBe("friction:/w/.astroshot");
  });
});
