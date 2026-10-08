import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { HashCache } from "./hash-cache.js";
import { loadUserStories, parseJsonl, runDisplayTitle, stepCountLabel } from "./friction.js";

let dir: string;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "friction-"));
  fs.writeFileSync(path.join(dir, "0001-a.png"), "x");
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

describe("friction JSONL", () => {
  it("honors aliases, skips comments and bad lines, drops missing screenshots", async () => {
    const text = [
      "# header",
      "",
      "not json",
      JSON.stringify({ title: "Auto", screenshots: ["missing.png", "nested/0001-a.png"] }),
      JSON.stringify({ step: 2, id: "two", narration: "spoken", screenshot: "0001-a.png", looks_good: ["ok"], improvements: [" fix ", ""] }),
    ].join("\n");
    const steps = await parseJsonl(text, dir);
    expect(steps.map((step) => step.step)).toEqual([1, 2]);
    const two = steps[1]!;
    expect(two.transcript).toBe("spoken");
    expect(two.screenshots).toEqual([path.join(dir, "0001-a.png")]);
    expect(two.good).toEqual(["ok"]);
    expect(two.improve).toEqual(["fix"]);
    const auto = steps[0]!;
    expect(auto.stepId).toBe("step-1");
    expect(auto.title).toBe("Auto");
    expect(auto.screenshots).toEqual([path.join(dir, "0001-a.png")]);
  });

  it("formats run titles and step counts", () => {
    expect(runDisplayTitle("20260811T153000Z")).toMatch(/^Aug 11 · \d{2}:\d{2}$/);
    expect(runDisplayTitle("20260811T153000Z-2")).toMatch(/^Aug 11 · /);
    expect(runDisplayTitle("short")).toBe("short");
    expect(runDisplayTitle("a-very-long-run-identifier")).toBe("run-identifier");
    expect(stepCountLabel(1)).toBe("1 step");
    expect(stepCountLabel(3)).toBe("3 steps");
  });
});

describe("user stories in both directories", () => {
  const context = { worktreePath: "/w/wt7", worktree: "wt7" };

  function writeStory(tree: string, slug: string, title: string) {
    const story = path.join(dir, tree, slug);
    const run = path.join(story, "runs", "20260811T153000Z");
    fs.mkdirSync(run, { recursive: true });
    fs.writeFileSync(path.join(run, "log.jsonl"), `${JSON.stringify({ step: 1, id: "a" })}\n`);
    fs.writeFileSync(path.join(story, "meta.json"), JSON.stringify({ title }));
  }

  it("lists a story that exists only under stories", async () => {
    writeStory("stories", "signup", "New");
    const logs = await loadUserStories(dir, context, new HashCache());
    expect(logs.map((log) => log.title)).toEqual(["New"]);
    expect(logs[0]!.directory).toBe(path.join(dir, "stories", "signup"));
  });

  it("lists a story that exists only under legacy friction-logs", async () => {
    writeStory("friction-logs", "signup", "Old");
    const logs = await loadUserStories(dir, context, new HashCache());
    expect(logs.map((log) => log.title)).toEqual(["Old"]);
    expect(logs[0]!.directory).toBe(path.join(dir, "friction-logs", "signup"));
  });

  it("takes a slug present in both trees from stories", async () => {
    writeStory("friction-logs", "signup", "Old");
    writeStory("friction-logs", "legacy-only", "Legacy only");
    writeStory("stories", "signup", "New");
    const logs = await loadUserStories(dir, context, new HashCache());
    expect(logs.map((log) => log.title).sort()).toEqual(["Legacy only", "New"]);
    expect(logs.find((log) => log.slug === "signup")!.directory).toBe(path.join(dir, "stories", "signup"));
  });

  it("does not let an empty stories directory hide the legacy story", async () => {
    fs.mkdirSync(path.join(dir, "stories", "signup"), { recursive: true });
    writeStory("friction-logs", "signup", "Old");
    const logs = await loadUserStories(dir, context, new HashCache());
    expect(logs.map((log) => log.title)).toEqual(["Old"]);
  });
});
