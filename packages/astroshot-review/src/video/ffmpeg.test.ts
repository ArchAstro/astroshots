import { describe, expect, it } from "vitest";

import { splitPngStream } from "./ffmpeg.js";

const IEND = Buffer.from([0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82]);

describe("PNG stream splitting", () => {
  it("returns whole frames and keeps the remainder", () => {
    const frameA = Buffer.concat([Buffer.from("AAAA"), IEND]);
    const frameB = Buffer.concat([Buffer.from("BB"), IEND]);
    const partial = Buffer.from("CC");
    const { frames, rest } = splitPngStream(Buffer.concat([frameA, frameB, partial]));
    expect(frames.length).toBe(2);
    expect(frames[0]!.equals(frameA)).toBe(true);
    expect(frames[1]!.equals(frameB)).toBe(true);
    expect(rest.toString()).toBe("CC");
  });
});
