import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { contentRevision } from "./identity.js";
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
    await writeFrictionAttempt(scenario, "run-a", [
      {
        id: "land",
        title: "Land",
        description: "Opened /",
        transcript: "I land.",
        screenshotPath: pngPath,
        screenshotName: "0001-land.png",
      },
    ]);
    const copy = path.join(root, "copy");
    fs.cpSync(scenario, copy, { recursive: true });
    fs.writeFileSync(path.join(copy, "prompt.md"), "edited prompt\n");
    await writeFrictionAttempt(copy, "run-b", [
      {
        id: "land",
        title: "Land",
        description: "Opened / again",
        transcript: "I land again.",
        screenshotPath: pngPath,
        screenshotName: "0001-land.png",
      },
    ]);
    const original = await importFrictionRecord(scenario);
    const copied = await importFrictionRecord(copy);
    expect(original.scenarioId).toBe(copied.scenarioId);
    expect(original.runs.map((run) => run.runId)).toEqual(["run-a"]);
    expect(copied.runs.map((run) => run.runId).sort()).toEqual(["run-a", "run-b"]);
    expect(fs.readFileSync(path.join(copy, "runs", "run-a", "prompt.md"), "utf8")).toBe("first prompt\n");
    expect(fs.readFileSync(path.join(copy, "runs", "run-b", "prompt.md"), "utf8")).toBe("edited prompt\n");
  });

  it("hashes ordered text and asset digests deterministically", () => {
    const first = contentRevision({
      promptSha256: "abc",
      entries: [{ title: "A", description: "d", transcript: "t", assetDigests: ["1"] }],
    });
    const same = contentRevision({
      promptSha256: "abc",
      entries: [{ title: "A", description: "d", transcript: "t", assetDigests: ["1"] }],
    });
    const changed = contentRevision({
      promptSha256: "abc",
      entries: [{ title: "A", description: "d", transcript: "t", assetDigests: ["2"] }],
    });
    expect(first).toBe(same);
    expect(first).not.toBe(changed);
  });
});
