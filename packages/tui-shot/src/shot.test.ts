import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { describe, expect, it } from "vitest";

import { takePtyShot } from "./pty-shot.js";

describe("takePtyShot", () => {
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
