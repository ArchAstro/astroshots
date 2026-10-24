import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { contentRevision, type ContentRevisionEntry } from "./identity.js";
import { importFrictionRecord, importShotRecord, writeFrictionAttempt } from "./portable-records.js";

const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScL6WQAAAABJRU5ErkJggg==",
  "base64",
);

const temps: string[] = [];
afterEach(() => {
  for (const dir of temps.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function tempDir(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "portable-unit-"));
  temps.push(dir);
  return dir;
}

function step(screenshotPath: string, description: string) {
  return {
    id: "land",
    title: "Land",
    description,
    transcript: "I land.",
    screenshotPath,
    screenshotName: "0001-land.png",
  };
}

describe("portable record import", () => {
  it("surfaces legacy slug/path ambiguity and does not invent media", async () => {
    const root = tempDir();
    const feature = path.join(root, "legacy");
    fs.mkdirSync(feature, { recursive: true });
    fs.writeFileSync(
      path.join(feature, "manifest.json"),
      JSON.stringify({ version: 1, feature: "legacy", run_id: "r1", shots: [{ file: "gone.png", slug: "gone" }] }),
    );
    const record = await importShotRecord(feature);
    expect(record.identity).toBe("ambiguous-legacy");
    expect(record.scenarioId).toBeNull();
    expect(record.diagnostics.map((item) => item.kind)).toEqual(
      expect.arrayContaining(["ambiguous-identity", "missing-asset"]),
    );
    expect(record.runs[0]!.entries[0]!.assets[0]!.missing).toBe(true);
  });

  it("keeps poster-only review validity distinct from content revision", async () => {
    const root = tempDir();
    const feature = path.join(root, "movie");
    fs.mkdirSync(feature, { recursive: true });
    fs.writeFileSync(path.join(feature, "0001-flow.png"), png);
    fs.writeFileSync(path.join(feature, "0001-flow.webm"), "video-bytes");
    fs.writeFileSync(
      path.join(feature, "manifest.json"),
      JSON.stringify({
        version: 1,
        schema_version: 1,
        scenario_id: "scenario-1",
        run_id: "run-1",
        shots: [
          {
            file: "0001-flow.png",
            slug: "flow",
            title: "Flow",
            kind: "movie",
            video: "0001-flow.webm",
            chapters: [{ slug: "mid", t_ms: 100 }],
            entry_id: "entry-1",
            asset_id: "poster-1",
            video_asset_id: "video-1",
          },
        ],
      }),
    );
    fs.writeFileSync(
      path.join(feature, "review.json"),
      JSON.stringify({
        version: 1,
        run_id: "run-1",
        reviews: {
          "0001-flow.png": {
            decision: "seen",
            image_sha256: "not-the-poster-hash",
            content_revision: "not-the-revision",
            comments: [{ id: "1", body: "note", created_at: "t" }],
          },
        },
      }),
    );
    const record = await importShotRecord(feature);
    expect(record.identity).toBe("identified");
    expect(record.runs[0]!.entries[0]!.assets.map((asset) => asset.role)).toEqual(["poster", "video"]);
    expect(record.runs[0]!.entries[0]!.chapters[0]!.slug).toBe("mid");
    expect(record.runs[0]!.review?.legacyHashMatches).toBe(false);
    expect(record.runs[0]!.review?.contentRevisionMatches).toBe(false);
    expect(record.runs[0]!.review?.comments[0]!.body).toBe("note");
    expect(record.runs[0]!.contentRevision).not.toBe("not-the-revision");
  });

  it("copies keep scenario identity and freeze the prompt per attempt", async () => {
    const root = tempDir();
    const scenario = path.join(root, "checkout");
    fs.mkdirSync(scenario, { recursive: true });
    fs.writeFileSync(path.join(scenario, "prompt.md"), "first prompt\n");
    const pngPath = path.join(root, "pixel.png");
    fs.writeFileSync(pngPath, png);
    await writeFrictionAttempt(scenario, "run-a", [step(pngPath, "Opened /")], { status: "complete" });
    const copy = path.join(root, "copy");
    fs.cpSync(scenario, copy, { recursive: true });
    fs.writeFileSync(path.join(copy, "prompt.md"), "edited prompt\n");
    await writeFrictionAttempt(copy, "run-b", [step(pngPath, "Opened / again")], { status: "failed" });
    const original = await importFrictionRecord(scenario);
    const copied = await importFrictionRecord(copy);
    expect(original.scenarioId).toBe(copied.scenarioId);
    expect(original.runs.map((run) => run.runId)).toEqual(["run-a"]);
    expect(copied.runs.map((run) => run.runId)).toEqual(["run-a", "run-b"]);
    expect(copied.runs.map((run) => run.status)).toEqual(["complete", "failed"]);
    expect(fs.readFileSync(path.join(copy, "runs", "run-a", "prompt.md"), "utf8")).toBe("first prompt\n");
    expect(fs.readFileSync(path.join(copy, "runs", "run-b", "prompt.md"), "utf8")).toBe("edited prompt\n");
  });

  it("refuses to overwrite an attempt and leaves a failed first attempt retryable", async () => {
    const root = tempDir();
    const scenario = path.join(root, "checkout");
    fs.mkdirSync(scenario, { recursive: true });
    const pngPath = path.join(root, "pixel.png");
    fs.writeFileSync(pngPath, png);

    // No prompt.md yet: the attempt fails and must leave nothing under runs/.
    await expect(
      writeFrictionAttempt(scenario, "run-a", [step(pngPath, "x")], { status: "complete" }),
    ).rejects.toThrow();
    expect(fs.readdirSync(path.join(scenario, "runs"))).toEqual([]);

    fs.writeFileSync(path.join(scenario, "prompt.md"), "p\n");
    await writeFrictionAttempt(scenario, "run-a", [step(pngPath, "x")], { status: "complete" });
    const logBefore = fs.readFileSync(path.join(scenario, "runs", "run-a", "log.jsonl"));
    await expect(
      writeFrictionAttempt(scenario, "run-a", [step(pngPath, "y")], { status: "failed" }),
    ).rejects.toThrow(/already exists/);
    expect(fs.readFileSync(path.join(scenario, "runs", "run-a", "log.jsonl")).equals(logBefore)).toBe(true);
    expect(fs.readdirSync(path.join(scenario, "runs"))).toEqual(["run-a"]);
  });

  it("rejects non-basename run ids, screenshot names and a missing status", async () => {
    const root = tempDir();
    const scenario = path.join(root, "checkout");
    fs.mkdirSync(scenario, { recursive: true });
    fs.writeFileSync(path.join(scenario, "prompt.md"), "p\n");
    const pngPath = path.join(root, "pixel.png");
    fs.writeFileSync(pngPath, png);
    const ok = { status: "complete" };
    await expect(writeFrictionAttempt(scenario, "../escape", [step(pngPath, "x")], ok)).rejects.toThrow(/runId/);
    await expect(writeFrictionAttempt(scenario, "a/b", [step(pngPath, "x")], ok)).rejects.toThrow(/runId/);
    await expect(
      writeFrictionAttempt(scenario, "r", [{ ...step(pngPath, "x"), screenshotName: "../x.png" }], ok),
    ).rejects.toThrow(/screenshotName/);
    await expect(writeFrictionAttempt(scenario, "r", [step(pngPath, "x")], { status: " " })).rejects.toThrow(/status/);
    expect(fs.existsSync(path.join(scenario, "meta.json"))).toBe(false);
    expect(fs.existsSync(path.join(root, "escape"))).toBe(false);
  });

  it("records the caller's status and a provenance per run", async () => {
    const root = tempDir();
    const scenario = path.join(root, "checkout");
    fs.mkdirSync(scenario, { recursive: true });
    fs.writeFileSync(path.join(scenario, "prompt.md"), "p\n");
    const pngPath = path.join(root, "pixel.png");
    fs.writeFileSync(pngPath, png);
    const provenance = (commit: string, dirty: boolean) => ({
      sourcePath: scenario,
      worktree: root,
      repo: null,
      commit,
      branch: "main",
      dirty,
      taskId: null,
      pr: null,
    });
    await writeFrictionAttempt(scenario, "run-a", [step(pngPath, "a")], {
      status: "running",
      provenance: provenance("aaa", false),
    });
    await writeFrictionAttempt(scenario, "run-b", [step(pngPath, "b")], {
      status: "failed",
      provenance: provenance("bbb", true),
    });
    const record = await importFrictionRecord(scenario);
    expect(record.runs.map((run) => [run.runId, run.status, run.provenance?.commit, run.provenance?.dirty])).toEqual([
      ["run-a", "running", "aaa", false],
      ["run-b", "failed", "bbb", true],
    ]);
    // Scenario provenance keeps the first attempt; it is not rewritten per run.
    expect(record.provenance.commit).toBe("aaa");
  });

  it("imports archived shot attempts as distinct runs", async () => {
    const root = tempDir();
    const feature = path.join(root, "feat");
    fs.mkdirSync(feature, { recursive: true });
    fs.writeFileSync(path.join(feature, "0001-a.png"), png);
    fs.writeFileSync(path.join(feature, "0002-b.png"), png);
    fs.writeFileSync(
      path.join(feature, "manifest.json"),
      JSON.stringify({
        version: 1,
        scenario_id: "s1",
        run_id: "run-2",
        status: "pass",
        shots: [{ file: "0002-b.png", slug: "b" }],
        previous_runs: [
          { run_id: "run-1", status: "fail", provenance: { commit: "abc", dirty: true }, shots: [{ file: "0001-a.png", slug: "a" }] },
          { run_id: "run-2", status: "fail", shots: [] },
        ],
      }),
    );
    const record = await importShotRecord(feature);
    expect(record.runs.map((run) => [run.runId, run.status, run.entries.length])).toEqual([
      ["run-1", "fail", 1],
      ["run-2", "fail", 0],
      ["run-2", "pass", 1],
    ]);
    expect(record.runs[0]!.provenance?.commit).toBe("abc");
    expect(record.diagnostics.map((item) => item.kind)).toContain("duplicate-run-id");
  });
});

describe("content revision", () => {
  const base: ContentRevisionEntry = {
    title: "A",
    description: "d",
    transcript: "t",
    url: "",
    good: [],
    improve: [],
    chapters: [],
    assetDigests: ["1"],
  };

  it("is deterministic and sensitive to every reviewed field and asset digest", () => {
    const rev = (patch: Partial<ContentRevisionEntry>, promptSha256: string | null = "abc") =>
      contentRevision({ promptSha256, entries: [{ ...base, ...patch }] });
    expect(rev({})).toBe(rev({}));
    for (const patch of [
      { title: "B" },
      { description: "e" },
      { transcript: "u" },
      { url: "https://x" },
      { good: ["g"] },
      { improve: ["i"] },
      { chapters: [{ slug: "c", title: "", tMs: 1, note: "" }] },
      { assetDigests: ["2"] },
    ]) {
      expect(rev(patch)).not.toBe(rev({}));
    }
    expect(rev({}, "other")).not.toBe(rev({}));
  });

  it("length-prefixes fields so adjacent values cannot be shifted", () => {
    const rev = (entry: Partial<ContentRevisionEntry>) => contentRevision({ entries: [{ ...base, ...entry }] });
    expect(rev({ title: "ab", description: "c" })).not.toBe(rev({ title: "a", description: "bc" }));
    expect(rev({ good: ["ab", "c"] })).not.toBe(rev({ good: ["a", "bc"] }));
    expect(rev({ good: ["a"], improve: [] })).not.toBe(rev({ good: [], improve: ["a"] }));
    expect(rev({ assetDigests: ["a\nb"] })).not.toBe(rev({ assetDigests: ["a", "b"] }));
  });
});

describe("writeFrictionAttempt guards", () => {
  it("refuses a corrupt scenario meta and reserved screenshot names", async () => {
    const root = tempDir();
    const scenario = path.join(root, "s");
    fs.mkdirSync(scenario, { recursive: true });
    fs.writeFileSync(path.join(scenario, "prompt.md"), "p\n");
    const pngPath = path.join(root, "pixel.png");
    fs.writeFileSync(pngPath, png);
    for (const name of ["log.jsonl", "meta.json", "notes.txt"]) {
      await expect(
        writeFrictionAttempt(scenario, "r", [{ ...step(pngPath, "x"), screenshotName: name }], { status: "complete" }),
      ).rejects.toThrow(/screenshotName/);
    }
    fs.writeFileSync(path.join(scenario, "meta.json"), "{");
    await expect(writeFrictionAttempt(scenario, "r", [step(pngPath, "x")], { status: "complete" })).rejects.toThrow(/not a JSON object/);
    expect(fs.readFileSync(path.join(scenario, "meta.json"), "utf8")).toBe("{");
  });
});
