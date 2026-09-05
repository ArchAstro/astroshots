import { describe, expect, it } from "vitest";

import { hasPrintableText, spliceGraphics } from "./graphics-stdout.js";

describe("graphics stdout splicing", () => {
  it("detects frames with visible text and ignores pure control writes", () => {
    expect(hasPrintableText("\x1b[2K\x1b[1A\x1b[G")).toBe(false);
    expect(hasPrintableText("\x1b[?2026h")).toBe(false);
    expect(hasPrintableText("\x1b[2K hello \x1b[0m")).toBe(true);
    expect(hasPrintableText("\x1b_Ga=p,i=1\x1b\\")).toBe(false);
  });

  it("keeps placements inside the synchronized update block", () => {
    expect(spliceGraphics("frame\x1b[?2026l", "<g>")).toBe("frame<g>\x1b[?2026l");
    expect(spliceGraphics("frame", "<g>")).toBe("frame<g>");
    expect(spliceGraphics("frame", "")).toBe("frame");
  });
});
