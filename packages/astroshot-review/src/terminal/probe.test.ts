import { describe, expect, it } from "vitest";

import { parseCellSizeEnv, parseProbeResponse, probeTerminal, supportsTrueColor } from "./probe.js";

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

function fakeTty(isTTY: boolean) {
  return { isTTY } as unknown as NodeJS.WriteStream & NodeJS.ReadStream;
}

describe("graphics mode selection", () => {
  it("uses half-blocks inside mosh without probing for kitty", async () => {
    const caps = await probeTerminal(
      { stdin: fakeTty(true), stdout: fakeTty(true) },
      { env: { COLORTERM: "truecolor", MOSH_SERVER_NETWORK_TMOUT: "600" }, probeFileMedium: false },
    );
    expect(caps.graphics).toBe("halfblocks");
    expect(caps.insideMosh).toBe(true);
    expect(caps.intercepted).toContain("mosh");
  });

  it("uses half-blocks inside herdr", async () => {
    const caps = await probeTerminal(
      { stdin: fakeTty(true), stdout: fakeTty(true) },
      { env: { COLORTERM: "truecolor", HERDR_PANE_ID: "w1:p3" } },
    );
    expect(caps.graphics).toBe("halfblocks");
    expect(caps.insideHerdr).toBe(true);
  });

  it("honors the forced half-block mode and the none mode", async () => {
    const forced = await probeTerminal(
      { stdin: fakeTty(true), stdout: fakeTty(true) },
      { env: { COLORTERM: "truecolor", ASTROSHOT_REVIEW_GRAPHICS: "halfblocks" } },
    );
    expect(forced.graphics).toBe("halfblocks");
    const off = await probeTerminal(
      { stdin: fakeTty(true), stdout: fakeTty(true) },
      { env: { COLORTERM: "truecolor", ASTROSHOT_REVIEW_GRAPHICS: "none" } },
    );
    expect(off.graphics).toBe("none");
  });

  it("stays off without a truecolor terminal", async () => {
    const caps = await probeTerminal(
      { stdin: fakeTty(true), stdout: fakeTty(true) },
      { env: { TERM: "vt100", MOSH_SERVER_NETWORK_TMOUT: "600" } },
    );
    expect(caps.graphics).toBe("none");
  });

  it("detects truecolor from COLORTERM and 256-color TERM", () => {
    expect(supportsTrueColor({ COLORTERM: "truecolor" } as NodeJS.ProcessEnv)).toBe(true);
    expect(supportsTrueColor({ TERM: "xterm-256color" } as NodeJS.ProcessEnv)).toBe(true);
    expect(supportsTrueColor({ TERM: "vt100" } as NodeJS.ProcessEnv)).toBe(false);
  });
});
