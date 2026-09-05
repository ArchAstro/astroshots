import { describe, expect, it } from "vitest";

import type { Shot } from "../data/model.js";
import { contiguousGroups, filterShots, reviewSiblings, streamCounts } from "./selectors.js";
import { flattenStream, isNavigable, nextNavigable, scrollWindow } from "./stream.js";

function shot(overrides: Partial<Shot>): Shot {
  return {
    id: overrides.path ?? "/p",
    path: "/p",
    fileName: "0001-a.png",
    worktreePath: "/w",
    worktree: "w",
    worktreeShort: "w",
    feature: "f",
    featureDir: "/w/.astroshot/f",
    sequence: "0001",
    slug: "a",
    title: "A",
    description: "",
    url: null,
    runId: "r",
    status: null,
    capturedAt: 0,
    mtimeMs: 0,
    isMovie: false,
    videoFileName: null,
    videoPath: null,
    durationMs: null,
    source: null,
    chapters: [],
    review: null,
    ...overrides,
  };
}

const seen = { state: "seen" as const, decision: "seen", hashMatches: true, isStale: false, comments: [], reviewedAt: null };

describe("stream selectors", () => {
  const shots = [
    shot({ path: "/w1/1", worktreePath: "/w1", feature: "f", sequence: "0002", capturedAt: 4, isMovie: true }),
    shot({ path: "/w2/1", worktreePath: "/w2", capturedAt: 3 }),
    shot({ path: "/w1/2", worktreePath: "/w1", feature: "f", sequence: "0001", capturedAt: 2, review: seen }),
    shot({ path: "/w1/3", worktreePath: "/w1", feature: "g", capturedAt: 1 }),
  ];

  it("groups contiguously, not by dictionary", () => {
    const groups = contiguousGroups(shots);
    expect(groups.map((group) => [group.worktreePath, group.shots.length, group.id])).toEqual([
      ["/w1", 1, "/w1/1"],
      ["/w2", 1, "/w2/1"],
      ["/w1", 2, "/w1/3"],
    ]);
  });

  it("filters unseen/history and movies", () => {
    expect(filterShots(shots, "unseen", false).map((entry) => entry.path)).toEqual(["/w1/1", "/w2/1", "/w1/3"]);
    expect(filterShots(shots, "history", false).map((entry) => entry.path)).toEqual(["/w1/2"]);
    expect(filterShots(shots, "unseen", true).map((entry) => entry.path)).toEqual(["/w1/1"]);
    expect(streamCounts(shots, false)).toEqual({ pending: 3, seen: 1, movies: 1 });
  });

  it("orders review siblings oldest first within worktree, feature, and run", () => {
    const siblings = reviewSiblings(shots, shots[0]!);
    expect(siblings.map((entry) => entry.path)).toEqual(["/w1/2", "/w1/1"]);
  });

  it("flattens groups, skips expanded headers when navigating, and windows around the cursor", () => {
    const groups = contiguousGroups(shots);
    const collapsed = new Set<string>();
    const items = flattenStream(groups, collapsed);
    expect(items.map((item) => item.kind)).toEqual(["header", "shot", "header", "shot", "header", "shot", "shot"]);
    expect(isNavigable(items[0]!, collapsed)).toBe(false);
    expect(nextNavigable(items, collapsed, 0, 1)).toBe(1);
    expect(nextNavigable(items, collapsed, 2, 1)).toBe(3);
    expect(nextNavigable(items, collapsed, 2, -1)).toBe(1);
    const collapsedFirst = new Set([groups[0]!.id]);
    expect(isNavigable(flattenStream(groups, collapsedFirst)[0]!, collapsedFirst)).toBe(true);
    // 7 items: heights 1,4,1,4,1,4,4. A 10-line window ending at item 6 starts at item 4 (1+4+4).
    expect(scrollWindow(items, 6, 0, 10)).toBe(4);
    expect(scrollWindow(items, 1, 3, 10)).toBe(1);
    expect(scrollWindow([], 0, 0, 10)).toBe(0);
  });
});
