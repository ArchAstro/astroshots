import { describe, expect, it } from "vitest";

import { rgbToHalfBlockLines } from "./halfblocks.js";

describe("half-block art", () => {
  it("packs two vertical pixels into one cell with fg/bg truecolor", () => {
    // 1x2: top red, bottom blue.
    const rgb = Buffer.from([255, 0, 0, 0, 0, 255]);
    const lines = rgbToHalfBlockLines({ width: 1, height: 2, rgb });
    expect(lines).toHaveLength(1);
    expect(lines[0]).toBe("\x1b[38;2;255;0;0m\x1b[48;2;0;0;255m▀\x1b[0m");
  });

  it("emits one line per two rows and reuses the last row when odd", () => {
    const rgb = Buffer.alloc(3 * 3, 10); // 1x3
    const lines = rgbToHalfBlockLines({ width: 1, height: 3, rgb });
    expect(lines).toHaveLength(2);
  });
});
