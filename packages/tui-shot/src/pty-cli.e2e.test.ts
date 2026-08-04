import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import sharp from "sharp";
import { describe, expect, it } from "vitest";

describe("arbitrary PTY capture boundary", () => {
  it("drives a full-screen terminal process and captures the selected state", async () => {
    // Arrange a disposable output while keeping the fixture beside its real
    // child executable, so relative command paths exercise fixture semantics.
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-shot-e2e-"));
    const outPath = path.join(tempDir, "selected.png");
    try {
      // Cross the published bin into a real pseudoterminal. The child enters
      // the alternate screen, receives Down and Enter, and redraws using ANSI.
      const stdout = execFileSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "pty",
          path.resolve("fixtures/interactive-pty.yaml"),
          "-o",
          outPath,
        ],
        { cwd: path.resolve("."), env: process.env, encoding: "utf8" },
      );

      // The fixture's final wait and expectText prove the externally visible
      // selected state; PNG pixels prove xterm and Chromium captured it.
      expect(stdout).toContain(`wrote ${outPath}`);
      const image = sharp(outPath);
      const [metadata, stats] = await Promise.all([
        image.metadata(),
        image.stats(),
      ]);
      expect(metadata.format).toBe("png");
      expect(metadata.width).toBeGreaterThan(400);
      expect(metadata.height).toBeGreaterThan(150);
      expect(fs.statSync(outPath).size).toBeGreaterThan(4_000);
      expect(stats.channels.some((channel) => channel.max > channel.min)).toBe(
        true,
      );
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it("waits for a clean terminal exit before capturing its final frame", async () => {
    // Run without the test wrapper override: Unix exercises node-pty's native
    // exit event, while Windows exercises the production ConPTY status bridge.
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-exit-e2e-"));
    const outPath = path.join(tempDir, "complete.png");
    try {
      const stdout = execFileSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "pty",
          path.resolve("fixtures/completing-pty.yaml"),
          "-o",
          outPath,
        ],
        { cwd: path.resolve("."), env: process.env, encoding: "utf8" },
      );

      // A successful CLI result and real PNG prove waitForExit crossed the
      // process boundary and retained the program's final visible frame.
      expect(stdout).toContain(`wrote ${outPath}`);
      const metadata = await sharp(outPath).metadata();
      expect(metadata.format).toBe("png");
      expect(metadata.width).toBeGreaterThan(300);
      expect(metadata.height).toBeGreaterThan(100);
      expect(fs.statSync(outPath).size).toBeGreaterThan(2_000);
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it("rejects a crashed terminal process even when its expected text rendered", () => {
    // Launch a real PTY child that draws a plausible final frame and exits 7.
    // This guards documentation capture from certifying a crash as success.
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-crash-e2e-"));
    const outPath = path.join(tempDir, "crash.png");
    try {
      const result = spawnSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "pty",
          path.resolve("fixtures/crashing-pty.yaml"),
          "-o",
          outPath,
        ],
        {
          cwd: path.resolve("."),
          env: {
            ...process.env,
            // Exercise the ConPTY exit-status bridge on every development OS.
            ASTROSHOT_TEST_FORCE_PTY_EXIT_WRAPPER: "1",
            // Status file is written immediately; delay the OSC marker so
            // waitForExit must trust the file bridge (Windows production path).
            ASTROSHOT_TEST_DELAY_PTY_EXIT_MARKER_MS: "1500",
          },
          encoding: "utf8",
          // Cold Windows runners need headroom for node → wrapper → child.
          timeout: 30_000,
          maxBuffer: 2 * 1024 * 1024,
        },
      );

      // The visible assertion passes, but the process boundary is authoritative.
      expect(
        result.status,
        `expected CLI failure on crash, got status=${result.status}\n` +
          `stdout:\n${result.stdout}\nstderr:\n${result.stderr}\n` +
          `signal=${result.signal} error=${result.error?.message ?? ""}`,
      ).toBe(1);
      expect(result.stderr).toContain("exited with code 7 before capture");
      expect(fs.existsSync(outPath)).toBe(false);
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it("preserves a terminal interrupt until the wrapped program reports its exit", () => {
    // Force the Windows status bridge on every OS, then send a real Ctrl-C
    // through the PTY to both wrapper and target process.
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-signal-e2e-"));
    const outPath = path.join(tempDir, "interrupted.png");
    try {
      const result = spawnSync(
        process.execPath,
        [
          path.resolve("bin/tui-shot.mjs"),
          "pty",
          path.resolve("fixtures/interruptible-pty.yaml"),
          "-o",
          outPath,
        ],
        {
          cwd: path.resolve("."),
          env: {
            ...process.env,
            ASTROSHOT_TEST_FORCE_PTY_EXIT_WRAPPER: "1",
          },
          encoding: "utf8",
        },
      );

      // The target's chosen status survives the shared process-group signal.
      expect(result.status).toBe(1);
      expect(result.stderr).toContain("exited with code 42 before capture");
      expect(result.stderr).not.toContain(
        "status wrapper exited without reporting",
      );
      expect(fs.existsSync(outPath)).toBe(false);
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it.runIf(process.platform === "win32")(
    "rejects Windows batch scripts instead of introducing a hidden shell",
    () => {
      const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pty-batch-e2e-"));
      const fixturePath = path.join(tempDir, "batch.yaml");
      const commandPath = path.join(tempDir, "astroshot-batch-probe.cmd");
      const outPath = path.join(tempDir, "batch.png");
      try {
        fs.writeFileSync(commandPath, "@echo off\r\necho unsafe shell boundary\r\n");
        fs.writeFileSync(
          fixturePath,
          [
            "version: 1",
            "command: astroshot-batch-probe",
            "cols: 40",
            "rows: 6",
          ].join("\n"),
        );

        const result = spawnSync(
          process.execPath,
          [
            path.resolve("bin/tui-shot.mjs"),
            "pty",
            fixturePath,
            "-o",
            outPath,
          ],
          {
            cwd: path.resolve("."),
            env: {
              ...process.env,
              PATH: `${tempDir}${path.delimiter}${process.env.PATH ?? ""}`,
            },
            encoding: "utf8",
          },
        );

        expect(result.status).toBe(1);
        expect(result.stderr).toContain(
          "Windows batch script, which requires a shell",
        );
        expect(fs.existsSync(outPath)).toBe(false);
      } finally {
        fs.rmSync(tempDir, { recursive: true, force: true });
      }
    },
  );
});
