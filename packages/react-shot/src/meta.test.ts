import { describe, expect, it } from "vitest";
import { isDialogSelector, resolveShotMeta } from "./meta.js";

describe("resolveShotMeta", () => {
  it("isolates dialog targets and preserves transparent corners by default", () => {
    const meta = resolveShotMeta({}, { selector: "[role=dialog]" });
    expect(meta.selector).toBe("[role=dialog]");
    expect(meta.stripOverlay).toBe(true);
    expect(meta.omitBackground).toBe(true);
  });

  it("captures the fixture root without overlay removal by default", () => {
    const meta = resolveShotMeta({}, {});
    expect(meta.selector).toBe("[data-react-shot-root]");
    expect(meta.stripOverlay).toBe(false);
  });

  it("uses browser fixture metadata when Node cannot import TSX", () => {
    const meta = resolveShotMeta(
      {},
      {
        width: 960,
        height: 1000,
        selector: "[role=dialog]",
        stripOverlay: true,
      },
    );
    expect(meta).toMatchObject({
      width: 960,
      height: 1000,
      selector: "[role=dialog]",
      stripOverlay: true,
    });
  });

  it("honors explicit dialog overlay behavior", () => {
    const meta = resolveShotMeta(
      {},
      { selector: "[role=dialog]", stripOverlay: false },
    );
    expect(meta.stripOverlay).toBe(false);
  });
});

describe("isDialogSelector", () => {
  it("recognizes common dialog selectors", () => {
    expect(isDialogSelector("[role=dialog]")).toBe(true);
    expect(isDialogSelector('[role="dialog"]')).toBe(true);
    expect(isDialogSelector("[role='dialog']")).toBe(true);
    expect(isDialogSelector("#root")).toBe(false);
  });
});
