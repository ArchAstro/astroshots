import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { sinkMovie } from "./sink.js";
import type { SinkMovieRequest } from "./types.js";

const temps: string[] = [];
afterEach(() => {
  for (const dir of temps.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function setup() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "sink-"));
  temps.push(root);
  const poster = path.join(root, "poster.png");
  const video = path.join(root, "video.webm");
  fs.writeFileSync(poster, "png");
  fs.writeFileSync(video, "webm");
  const sink = (runId: string, extra: Partial<SinkMovieRequest> = {}) =>
    sinkMovie({
      root,
      feature: "flow",
      slug: "tour",
      runId,
      source: "frames",
      posterPath: poster,
      videoPath: video,
      durationMs: 1000,
      chapters: [],
      ...extra,
    });
  const dir = path.join(root, ".astroshot", "flow");
  const manifest = () => JSON.parse(fs.readFileSync(path.join(dir, "manifest.json"), "utf8"));
  return { sink, manifest, dir };
}

describe("sinkMovie run rollover", () => {
  it("continues the same run id even after it was finalized, never duplicating it", () => {
    const { sink, manifest } = setup();
    sink("run-1", { status: "pass" });
    sink("run-1");
    const doc = manifest();
    expect(doc.run_id).toBe("run-1");
    expect(doc.shots).toHaveLength(2);
    expect(doc.previous_runs).toBeUndefined();
  });

  it("archives the previous attempt on a new run id and keeps the scenario id", () => {
    const { sink, manifest } = setup();
    sink("run-1", { status: "fail" });
    const first = manifest();
    sink("run-2");
    const doc = manifest();
    expect(doc.scenario_id).toBe(first.scenario_id);
    expect(doc.run_id).toBe("run-2");
    expect(doc.shots).toHaveLength(1);
    expect(doc.previous_runs).toEqual([
      { run_id: "run-1", status: "fail", provenance: first.provenance, shots: first.shots },
    ]);
  });

  it("refuses to reuse an archived run id before touching any file", () => {
    const { sink, manifest, dir } = setup();
    sink("run-1");
    sink("run-2");
    const before = fs.readdirSync(dir);
    expect(() => sink("run-1")).toThrow(/already recorded/);
    expect(fs.readdirSync(dir)).toEqual(before);
    expect(manifest().run_id).toBe("run-2");
    expect(manifest().shots).toHaveLength(1);
  });
});

describe("sinkMovie legacy rollover", () => {
  it("archives shots of a manifest without run_id under a synthesized id", () => {
    const { sink, manifest, dir } = setup();
    fs.mkdirSync(dir, { recursive: true });
    const legacyShots = [{ id: "0001", file: "0001-old.png", slug: "old" }];
    fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify({ version: 1, feature: "flow", shots: legacyShots }));
    sink("run-1");
    const doc = manifest();
    expect(doc.run_id).toBe("run-1");
    expect(doc.previous_runs).toEqual([{ run_id: "legacy-1", status: undefined, shots: legacyShots }].map((r) => JSON.parse(JSON.stringify(r))));
  });
});
