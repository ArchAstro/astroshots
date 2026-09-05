import { describe, expect, it } from "vitest";

import { chapterTimeLabel, chaptersOf, durationLabel, matchManifestShot, parseFeatureStatus } from "./manifest.js";

describe("manifest matching", () => {
  const manifest = {
    shots: [
      { id: "0001", file: "0001-a.png", slug: "a" },
      { id: "0002", poster: "0002-b.png", slug: "b", kind: "movie", video: "0002-b.webm" },
      { id: "0003", slug: "configure" },
    ],
  };
  it("matches by file, poster, id, then slug", () => {
    expect(matchManifestShot(manifest, "0001-a.png")?.slug).toBe("a");
    expect(matchManifestShot(manifest, "0002-b.png")?.slug).toBe("b");
    expect(matchManifestShot(manifest, "0003-anything.png")?.slug).toBe("configure");
    expect(matchManifestShot(manifest, "configure.png")?.slug).toBe("configure");
    expect(matchManifestShot(manifest, "0009-zzz.png")).toBeNull();
    expect(matchManifestShot(null, "x.png")).toBeNull();
  });

  it("maps execution status aliases", () => {
    expect(parseFeatureStatus("in-progress")).toBe("running");
    expect(parseFeatureStatus("ok")).toBe("pass");
    expect(parseFeatureStatus("error")).toBe("fail");
    expect(parseFeatureStatus("pending")).toBe("idle");
    expect(parseFeatureStatus("weird")).toBeNull();
  });

  it("formats durations and chapter times", () => {
    expect(durationLabel(2800)).toBe("2.8s");
    expect(durationLabel(3000)).toBe("3s");
    expect(durationLabel(75_000)).toBe("1:15");
    expect(durationLabel(0)).toBeNull();
    expect(chapterTimeLabel(900)).toBe("0.9s");
    expect(chapterTimeLabel(61_000)).toBe("1:01");
    expect(chapterTimeLabel(undefined)).toBe("—");
  });

  it("accepts camelCase chapter times", () => {
    expect(chaptersOf({ chapters: [{ slug: "a", tMs: 5 }, { slug: "b", t_ms: 7 }] })).toEqual([
      { slug: "a", title: undefined, tMs: 5 },
      { slug: "b", title: undefined, tMs: 7 },
    ]);
  });
});
