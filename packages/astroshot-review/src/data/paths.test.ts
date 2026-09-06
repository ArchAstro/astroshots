import { describe, expect, it } from "vitest";

import { humanize, isInsideFrictionLogs, parseShotPath, sequenceAndSlug, worktreeShort } from "./paths.js";

describe("shot paths", () => {
  it("accepts <worktree>/.astroshot/<feature>/<image>", () => {
    const parsed = parseShotPath("/repos/app/.astroshot/checkout/0002-configure.png");
    expect(parsed).toEqual({
      worktreePath: "/repos/app",
      worktree: "app",
      feature: "checkout",
      featureDir: "/repos/app/.astroshot/checkout",
      fileName: "0002-configure.png",
    });
  });

  it("rejects friction logs, non-images, and malformed layouts", () => {
    expect(parseShotPath("/repos/app/.astroshot/friction-logs/x/runs/1/0001-a.png")).toBeNull();
    expect(parseShotPath("/repos/app/.astroshot/checkout/manifest.json")).toBeNull();
    expect(parseShotPath("/repos/app/.astroshot/0001-a.png")).toBeNull();
    expect(parseShotPath("/repos/app/shots/checkout/0001-a.png")).toBeNull();
    expect(isInsideFrictionLogs("/repos/app/.astroshot/friction-logs/x/prompt.md")).toBe(true);
  });

  it("splits sequence and slug only for numeric prefixes", () => {
    expect(sequenceAndSlug("0004-configure.png")).toEqual({ sequence: "0004", slug: "configure" });
    expect(sequenceAndSlug("hero-shot.png")).toEqual({ sequence: null, slug: "hero-shot" });
    expect(humanize("next-steps_two")).toBe("Next Steps Two");
  });

  it("shortens worktree names like the app", () => {
    expect(worktreeShort("firstlanding-wt12")).toBe("wt12");
    expect(worktreeShort("demo-app")).toBe("demo-app");
    expect(worktreeShort("very-long-worktree-name")).toBe("very-l");
  });
});
