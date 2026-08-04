import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const script = path.join(root, "scripts/roll-changelog.mjs");

function withTempChangelog(body, run) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshots-changelog-"));
  const changelog = path.join(dir, "CHANGELOG.md");
  fs.writeFileSync(changelog, body);
  // Script always writes repo-root CHANGELOG; exercise via copy+cwd override
  // by running against a temp repo root that contains scripts/ + CHANGELOG.
  const scriptsDir = path.join(dir, "scripts");
  fs.mkdirSync(scriptsDir);
  fs.copyFileSync(script, path.join(scriptsDir, "roll-changelog.mjs"));
  try {
    return run(dir, changelog);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test("rolls Unreleased notes into a dated track section", () => {
  withTempChangelog(
    `# Changelog

## [Unreleased]

### Added

- Left/right paging

## [0.1.0] (npm) - 2026-07-01
`,
    (dir, changelog) => {
      execFileSync(
        process.execPath,
        [
          path.join(dir, "scripts/roll-changelog.mjs"),
          "0.1.15",
          "--track",
          "macos",
          "--date",
          "2026-08-04",
        ],
        { cwd: dir, encoding: "utf8" },
      );
      const text = fs.readFileSync(changelog, "utf8");
      assert.match(text, /## \[Unreleased\]\n\n## \[0\.1\.15\] \(macos\) - 2026-08-04/);
      assert.match(text, /Left\/right paging/);
      assert.match(text, /## \[0\.1\.0\] \(npm\) - 2026-07-01/);
    },
  );
});

test("rejects duplicate track version and missing Unreleased", () => {
  withTempChangelog(
    `# Changelog

## [Unreleased]

## [0.1.1] (npm) - 2026-08-01
`,
    (dir) => {
      assert.throws(() => {
        execFileSync(
          process.execPath,
          [
            path.join(dir, "scripts/roll-changelog.mjs"),
            "0.1.1",
            "--track",
            "npm",
            "--date",
            "2026-08-04",
          ],
          { cwd: dir, encoding: "utf8", stdio: "pipe" },
        );
      });
    },
  );

  withTempChangelog(
    `# Changelog

## [0.1.0] (npm) - 2026-07-01
`,
    (dir) => {
      assert.throws(() => {
        execFileSync(
          process.execPath,
          [
            path.join(dir, "scripts/roll-changelog.mjs"),
            "0.1.1",
            "--track",
            "npm",
          ],
          { cwd: dir, encoding: "utf8", stdio: "pipe" },
        );
      });
    },
  );
});
