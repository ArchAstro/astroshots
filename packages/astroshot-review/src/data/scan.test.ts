import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { HashCache } from "./hash-cache.js";
import { findAstroshotDirs, scanTree } from "./scan.js";
import { markSeen } from "./review-store.js";

let root: string;

function write(file: string, content: string | Buffer) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), "scan-"));
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe("discovery", () => {
  it("finds .astroshot trees, skips heavy and hidden directories, and does not descend into trees", async () => {
    write(path.join(root, "app/.astroshot/feature/0001-a.png"), "x");
    write(path.join(root, "app/.astroshot/nested/.astroshot/deeper/0001-b.png"), "x");
    write(path.join(root, "app/node_modules/pkg/.astroshot/f/0001-c.png"), "x");
    write(path.join(root, ".hidden/.astroshot/f/0001-d.png"), "x");
    write(path.join(root, "deep/a/b/c/.astroshot/f/0001-e.png"), "x");
    const found = await findAstroshotDirs([root]);
    expect(found).toEqual([path.join(root, "app/.astroshot"), path.join(root, "deep/a/b/c/.astroshot")]);
    expect(await findAstroshotDirs([root], { maxDepth: 2 })).toEqual([path.join(root, "app/.astroshot")]);
  });
});

describe("scanTree", () => {
  it("builds shots with manifest metadata, movie pairing, and review scoping", async () => {
    const feature = path.join(root, "demo-app/.astroshot/checkout");
    write(path.join(feature, "0001-welcome.png"), "welcome");
    write(path.join(feature, "0002-journey.png"), "poster");
    write(path.join(feature, "0002-journey.webm"), "video");
    write(path.join(feature, "0003-orphan.png"), "orphan");
    write(
      path.join(feature, "manifest.json"),
      JSON.stringify({
        version: 1,
        run_id: "run-1",
        status: "passed",
        shots: [
          { id: "0001", file: "0001-welcome.png", title: "Welcome", description: "Landing", captured_at: "2026-09-05T10:00:00Z", url: "/welcome" },
          { id: "0002", file: "0002-journey.png", kind: "movie", duration_ms: 4200, source: "browser", chapters: [{ slug: "a", t_ms: 100 }] },
        ],
      }),
    );
    await markSeen({ directory: feature, fileName: "0001-welcome.png", runId: "run-1", targetPath: path.join(feature, "0001-welcome.png") });
    write(path.join(root, "demo-app/.astroshot/friction-logs/scenario/prompt.md"), "# prompt");
    write(path.join(root, "demo-app/.astroshot/friction-logs/scenario/runs/20260811T153000Z/log.jsonl"), JSON.stringify({ step: 1, id: "s1", title: "Step", screenshots: ["0001-s1.png"] }));
    write(path.join(root, "demo-app/.astroshot/friction-logs/scenario/runs/20260811T153000Z/0001-s1.png"), "x");

    const tree = await scanTree(path.join(root, "demo-app/.astroshot"), new HashCache());
    expect(tree.worktree).toBe("demo-app");
    const byFile = Object.fromEntries(tree.shots.map((shot) => [shot.fileName, shot]));
    expect(Object.keys(byFile).sort()).toEqual(["0001-welcome.png", "0002-journey.png", "0003-orphan.png"]);

    const welcome = byFile["0001-welcome.png"]!;
    expect(welcome.title).toBe("Welcome");
    expect(welcome.url).toBe("/welcome");
    expect(welcome.status).toBe("pass");
    expect(welcome.capturedAt).toBe(Date.parse("2026-09-05T10:00:00Z"));
    expect(welcome.review?.state).toBe("seen");

    const journey = byFile["0002-journey.png"]!;
    expect(journey.isMovie).toBe(true);
    expect(journey.videoFileName).toBe("0002-journey.webm");
    expect(journey.videoPath).toBe(path.join(feature, "0002-journey.webm"));
    expect(journey.durationMs).toBe(4200);
    expect(journey.chapters).toEqual([{ slug: "a", title: undefined, tMs: 100 }]);
    expect(journey.title).toBe("Journey");
    expect(journey.review?.state).toBe("pending");

    const orphan = byFile["0003-orphan.png"]!;
    expect(orphan.title).toBe("Orphan");
    expect(orphan.isMovie).toBe(false);

    expect(tree.frictionLogs.length).toBe(1);
    expect(tree.frictionLogs[0]!.runs[0]!.steps[0]!.screenshots.length).toBe(1);
    expect(tree.shots.some((shot) => shot.path.includes("friction-logs"))).toBe(false);
  });

  it("un-sees a shot whose bytes changed and keeps its comments", async () => {
    const feature = path.join(root, "app/.astroshot/f");
    const image = path.join(feature, "0001-a.png");
    write(image, "v1");
    await markSeen({ directory: feature, fileName: "0001-a.png", runId: null, targetPath: image }, { comment: "note" });
    write(image, "v2");
    const tree = await scanTree(path.join(root, "app/.astroshot"), new HashCache());
    const shot = tree.shots[0]!;
    expect(shot.review?.state).toBe("pending");
    expect(shot.review?.isStale).toBe(true);
    expect(shot.review?.comments.map((comment) => comment.body)).toEqual(["note"]);
  });
});
