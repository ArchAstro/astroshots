import { describe, expect, it } from "vitest";

import { parseCellSizeEnv, parseProbeResponse } from "./probe.js";

describe("terminal probe parsing", () => {
  it("recognizes a kitty OK plus cell and window reports", () => {
    const report = parseProbeResponse("\x1b_Gi=31;OK\x1b\\\x1b[6;20;9t\x1b[4;800;1260t\x1b[?62;22c");
    expect(report.kittyOk).toBe(true);
    expect(report.fileOk).toBe(false);
    expect(report.cellWidth).toBe(9);
    expect(report.cellHeight).toBe(20);
    expect(report.windowWidth).toBe(1260);
    expect(report.sawDeviceAttributes).toBe(true);
  });

  it("treats an error reply as unsupported", () => {
    const report = parseProbeResponse("\x1b_Gi=31;ENOENT:bad\x1b\\\x1b[?1;2c");
    expect(report.kittyOk).toBe(false);
    expect(report.sawDeviceAttributes).toBe(true);
  });

  it("notices the file-medium probe separately", () => {
    const report = parseProbeResponse("\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\\x1b[?62c");
    expect(report.fileOk).toBe(true);
  });

  it("parses the cell size override", () => {
    expect(parseCellSizeEnv("10x20")).toEqual({ width: 10, height: 20 });
    expect(parseCellSizeEnv("8×16")).toEqual({ width: 8, height: 16 });
    expect(parseCellSizeEnv("nope")).toBeNull();
    expect(parseCellSizeEnv(undefined)).toBeNull();
  });
});
