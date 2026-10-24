/**
 * Producer → filesystem → reader identity proof.
 *
 * Spawns the real `astroshot-capture` helper, then reads with the production
 * `dist` module from a second process. Does not prove cloud import, the macOS
 * app, or Chromium/graphics.
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { writeFrictionAttempt } from "./data/portable-records.js";
import type { PortableRecord } from "./data/model.js";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(packageRoot, "../..");
const captureHelper = path.join(repoRoot, "skills/astroshots-review/scripts/astroshot-capture");
const binHelper = path.join(repoRoot, "bin/astroshot-capture");
const distReader = path.join(packageRoot, "dist/index.js");
const pngBytes = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScL6WQAAAABJRU5ErkJggg==",
  "base64",
);

function git(cwd: string, args: string[]) {
  const result = spawnSync("git", ["-C", cwd, "-c", "commit.gpgsign=false", ...args], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr}`);
  }
}

function initRepo(root: string) {
  fs.mkdirSync(root, { recursive: true });
  git(root, ["init"]);
  git(root, ["config", "user.email", "portable@example.com"]);
  git(root, ["config", "user.name", "Portable Records"]);
  fs.writeFileSync(path.join(root, "README"), "seed\n");
  git(root, ["add", "."]);
  git(root, ["commit", "-m", "seed"]);
}

function runCapture(root: string, feature: string, slug: string, source: string, extra: string[] = []) {
  return spawnSync(
    captureHelper,
    ["--root", root, "--feature", feature, "--slug", slug, "--source", source, "--title", slug, ...extra],
    { encoding: "utf8" },
  );
}

function capture(root: string, feature: string, slug: string, source: string, extra: string[] = []) {
  const result = runCapture(root, feature, slug, source, extra);
  if (result.status !== 0) {
    throw new Error(`astroshot-capture failed: ${result.stderr || result.stdout}`);
  }
}

function readWithProductionProcess(directories: string[]): PortableRecord[] {
  if (!fs.existsSync(distReader)) {
    throw new Error(`production reader missing: ${distReader}; run the package build first`);
  }
  const script = `
    import { importPortableRecord } from ${JSON.stringify(distReader)};
    const dirs = ${JSON.stringify(directories)};
    const records = [];
    for (const dir of dirs) records.push(await importPortableRecord(dir));
    process.stdout.write(JSON.stringify(records));
  `;
  const result = spawnSync(process.execPath, ["--input-type=module", "-e", script], {
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(`production reader failed: ${result.stderr || result.stdout}`);
  }
  return JSON.parse(result.stdout) as PortableRecord[];
}

describe("portable records across worktrees", () => {
  it("preserves scenario identity and distinct attempts across copied worktrees", async () => {
    expect(fs.readFileSync(captureHelper, "utf8")).toBe(fs.readFileSync(binHelper, "utf8"));

    const temp = fs.mkdtempSync(path.join(os.tmpdir(), "portable-records-"));
    try {
      const sourcePng = path.join(temp, "pixel.png");
      fs.writeFileSync(sourcePng, pngBytes);

      const legacyRoot = path.join(temp, "legacy-tree");
      initRepo(legacyRoot);
      const legacyFeature = path.join(legacyRoot, ".astroshot", "legacy-feature");
      fs.mkdirSync(legacyFeature, { recursive: true });
      fs.writeFileSync(path.join(legacyFeature, "0001-old.png"), pngBytes);
      fs.writeFileSync(
        path.join(legacyFeature, "manifest.json"),
        JSON.stringify({
          version: 1,
          feature: "legacy-feature",
          run_id: "legacy-run",
          status: "pass",
          shots: [{ id: "0001", file: "0001-old.png", slug: "old", title: "Old" }],
        }),
      );
      const legacyFriction = path.join(legacyRoot, ".astroshot", "friction-logs", "checkout");
      fs.mkdirSync(path.join(legacyFriction, "runs", "20260101T000000Z"), { recursive: true });
      fs.writeFileSync(path.join(legacyFriction, "prompt.md"), "# Checkout\nlegacy prompt\n");
      fs.writeFileSync(
        path.join(legacyFriction, "runs", "20260101T000000Z", "log.jsonl"),
        [
          "not json",
          JSON.stringify({
            step: 1,
            id: "land",
            title: "Land",
            description: "Opened /",
            transcript: "I land.",
            screenshots: ["missing.png"],
          }),
        ].join("\n"),
      );

      capture(legacyRoot, "captured-legacy", "first", sourcePng);

      const identifiedRoot = path.join(temp, "identified-tree");
      initRepo(identifiedRoot);
      fs.writeFileSync(path.join(identifiedRoot, "dirty.txt"), "dirty\n");
      const identifiedFriction = path.join(identifiedRoot, ".astroshot", "friction-logs", "checkout");
      fs.mkdirSync(identifiedFriction, { recursive: true });
      fs.writeFileSync(path.join(identifiedFriction, "prompt.md"), "# Checkout\nfirst attempt prompt\n");
      const firstAttempt = await writeFrictionAttempt(identifiedFriction, "20260915T120000Z", [
        {
          id: "land",
          title: "Land",
          description: "Opened /",
          transcript: "I land on home and take the CTA.",
          screenshotPath: sourcePng,
          screenshotName: "0001-land.png",
        },
      ], { status: "running" });

      // Phase: the helper records attempt one of a still feature.
      capture(identifiedRoot, "install-wizard", "configure", sourcePng, ["--run-id", "wizard-run-1", "--status", "fail"]);

      const copyRoot = path.join(temp, "copied-tree");
      fs.cpSync(identifiedRoot, copyRoot, { recursive: true });
      const copiedFriction = path.join(copyRoot, ".astroshot", "friction-logs", "checkout");
      fs.writeFileSync(path.join(copiedFriction, "prompt.md"), "# Checkout\nedited after copy\n");
      const secondAttempt = await writeFrictionAttempt(copiedFriction, "20260915T130000Z", [
        {
          id: "land",
          title: "Land",
          description: "Opened / again",
          transcript: "I land again in the copied worktree.",
          screenshotPath: sourcePng,
          screenshotName: "0001-land.png",
        },
      ], { status: "failed" });

      // Phase: the copied worktree records a second, distinct attempt of the same feature.
      capture(copyRoot, "install-wizard", "configure-again", sourcePng, ["--run-id", "wizard-run-2", "--status", "pass"]);

      // Phase: attempts are never overwritten, and an archived run id is never reused.
      await expect(
        writeFrictionAttempt(copiedFriction, "20260915T130000Z", [], { status: "complete" }),
      ).rejects.toThrow(/already exists/);
      const reuse = runCapture(copyRoot, "install-wizard", "late", sourcePng, ["--run-id", "wizard-run-1"]);
      expect(reuse.status).not.toBe(0);
      expect(reuse.stderr).toContain("already recorded");

      const [legacyShot, legacyLog, identifiedShot, identifiedLog, copiedShot, copiedLog] =
        readWithProductionProcess([
          path.join(legacyRoot, ".astroshot", "legacy-feature"),
          legacyFriction,
          path.join(identifiedRoot, ".astroshot", "install-wizard"),
          identifiedFriction,
          path.join(copyRoot, ".astroshot", "install-wizard"),
          copiedFriction,
        ]);

      expect(legacyShot!.identity).toBe("ambiguous-legacy");
      expect(legacyShot!.scenarioId).toBeNull();
      expect(legacyShot!.diagnostics.some((item) => item.kind === "ambiguous-identity")).toBe(true);

      expect(legacyLog!.identity).toBe("ambiguous-legacy");
      expect(legacyLog!.diagnostics.map((item) => item.kind)).toEqual(
        expect.arrayContaining(["ambiguous-identity", "malformed-line", "missing-asset", "unfrozen-prompt"]),
      );
      expect(legacyLog!.runs[0]!.entries[0]!.assets[0]!.missing).toBe(true);

      expect(identifiedShot!.identity).toBe("identified");
      expect(identifiedShot!.scenarioId).toBe(copiedShot!.scenarioId);
      expect(identifiedShot!.runs).toHaveLength(1);
      expect(identifiedShot!.runs[0]!.runId).toBe("wizard-run-1");
      expect(identifiedShot!.runs[0]!.entries).toHaveLength(1);
      expect(identifiedShot!.provenance.dirty).toBe(true);
      expect(identifiedShot!.provenance.commit).toBeTruthy();
      expect(identifiedShot!.runs[0]!.entries[0]!.assets[0]!.sha256).toBeTruthy();
      // The copy archives attempt one (same bytes, same revision) and adds its own.
      expect(copiedShot!.runs.map((run) => [run.runId, run.status])).toEqual([
        ["wizard-run-1", "fail"],
        ["wizard-run-2", "pass"],
      ]);
      expect(copiedShot!.runs[0]!.contentRevision).toBe(identifiedShot!.runs[0]!.contentRevision);
      expect(copiedShot!.runs[0]!.entries[0]!.assets[0]!.fileName).toBe("0001-configure.png");
      expect(copiedShot!.runs[1]!.entries[0]!.assets[0]!.fileName).toBe("0002-configure-again.png");
      expect(copiedShot!.runs[0]!.provenance?.dirty).toBe(true);
      expect(copiedShot!.diagnostics.map((item) => item.kind)).not.toContain("duplicate-run-id");

      expect(identifiedLog!.scenarioId).toBe(copiedLog!.scenarioId);
      expect(identifiedLog!.identity).toBe("identified");
      expect(copiedLog!.runs.map((run) => run.runId).sort()).toEqual(
        ["20260915T120000Z", "20260915T130000Z"].sort(),
      );
      // Execution status is what the caller reported; each run carries its own provenance.
      expect(identifiedLog!.runs[0]!.status).toBe("running");
      expect(copiedLog!.runs.map((run) => run.status)).toEqual(["running", "failed"]);
      expect(copiedLog!.runs.every((run) => run.provenance?.commit)).toBe(true);
      expect(copiedLog!.runs[1]!.provenance?.worktree).toContain("copied-tree");
      expect(identifiedLog!.runs).toHaveLength(1);
      expect(identifiedLog!.runs[0]!.runId).not.toBe(
        copiedLog!.runs.find((run) => run.runId === "20260915T130000Z")!.runId,
      );

      const firstPrompt = fs.readFileSync(path.join(firstAttempt.runDir, "prompt.md"));
      const liveOriginal = fs.readFileSync(path.join(identifiedFriction, "prompt.md"));
      const liveCopied = fs.readFileSync(path.join(copiedFriction, "prompt.md"));
      const secondPrompt = fs.readFileSync(path.join(secondAttempt.runDir, "prompt.md"));
      expect(firstPrompt.equals(liveOriginal)).toBe(true);
      expect(secondPrompt.equals(liveCopied)).toBe(true);
      expect(firstPrompt.equals(secondPrompt)).toBe(false);

      const capturedBytes = fs.readFileSync(
        path.join(identifiedRoot, ".astroshot", "install-wizard", "0001-configure.png"),
      );
      expect(capturedBytes.equals(pngBytes)).toBe(true);
      expect(identifiedLog!.runs[0]!.promptSnapshot?.frozen).toBe(true);
      expect(copiedLog!.runs.find((run) => run.runId === "20260915T130000Z")!.promptSnapshot?.frozen).toBe(true);
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
  }, 60_000);
});
