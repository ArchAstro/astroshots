import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { describe, expect, it } from "vitest";

import { takePtyShot } from "./pty-shot.js";
import { takeTuiShot } from "./shot.js";

describe("takeTuiShot", () => {
  it("rejects a fixture when its intended visible state did not render", async () => {
    await expect(
      takeTuiShot({
        fixturePath: path.resolve("fixtures/missing-expected-text.tsx"),
        outPath: path.join(os.tmpdir(), `tui-shot-missing-${process.pid}.png`),
      }),
    ).rejects.toThrow(/did not render expected text "Intended screen"/);
  });

  it("restores an existing global React bridge after a failed capture", async () => {
    const previousReact = Object.getOwnPropertyDescriptor(globalThis, "React");
    const sentinel = { source: "test-owner" };
    Object.defineProperty(globalThis, "React", {
      configurable: true,
      value: sentinel,
      writable: false,
    });

    try {
      await expect(
        takeTuiShot({
          fixturePath: path.resolve("fixtures/missing-expected-text.tsx"),
          outPath: path.join(os.tmpdir(), `tui-shot-react-${process.pid}.png`),
        }),
      ).rejects.toThrow(/did not render expected text/);
      expect((globalThis as unknown as Record<string, unknown>).React).toBe(
        sentinel,
      );
      expect(
        Object.getOwnPropertyDescriptor(globalThis, "React")?.writable,
      ).toBe(false);
    } finally {
      if (previousReact) Object.defineProperty(globalThis, "React", previousReact);
      else Reflect.deleteProperty(globalThis, "React");
    }
  });

  it("rejects malformed PTY appearance fields before launching a command", async () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-invalid-"));
    const fixturePath = path.join(tempDir, "invalid.yaml");
    fs.writeFileSync(
      fixturePath,
      "version: 1\ncommand: never-launched\nbackground:\n  unsafe: value\n",
    );
    try {
      await expect(
        takePtyShot({
          fixturePath,
          outPath: path.join(tempDir, "invalid.png"),
        }),
      ).rejects.toThrow(/background must be a string/);
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it("rejects a malformed waitForExit action before launching a command", async () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-invalid-exit-"));
    const fixturePath = path.join(tempDir, "invalid.yaml");
    fs.writeFileSync(
      fixturePath,
      [
        "version: 1",
        "command: never-launched",
        "actions:",
        "  - waitForExit: false",
        "",
      ].join("\n"),
    );
    try {
      await expect(
        takePtyShot({
          fixturePath,
          outPath: path.join(tempDir, "invalid.png"),
        }),
      ).rejects.toThrow(/waitForExit must be true/);
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });
});
