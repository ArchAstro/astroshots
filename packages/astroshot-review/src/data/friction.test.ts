import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { parseJsonl, runDisplayTitle, stepCountLabel } from "./friction.js";

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
