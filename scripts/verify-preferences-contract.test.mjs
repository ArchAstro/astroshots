import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const guard = "scripts/verify-preferences-contract.mjs";

/**
 * The guard derives its repo root from its own location, so a fixture repo is
 * built by copying the real files it reads plus the script itself. That keeps
 * the negative proofs honest: they exercise the same code path CI runs, on a
 * tree we are free to corrupt.
 */
const FIXTURE_FILES = [
  guard,
  "docs/PREFERENCES.md",
  "README.md",
  "macos/README.md",
  "macos/project.yml",
  "macos/Astroshots/App/Preferences.swift",
  "macos/Astroshots.xcodeproj/project.pbxproj",
];

function withFixtureRepo(run) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshots-prefs-guard-"));
  try {
    for (const relativePath of FIXTURE_FILES) {
      const destination = path.join(dir, relativePath);
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      fs.copyFileSync(path.join(repoRoot, relativePath), destination);
    }
    const git = (...args) =>
      execFileSync("git", args, { cwd: dir, encoding: "utf8", stdio: "pipe" });
    git("init", "--quiet");
    git("add", "--all");
    return run(dir, git);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

function runGuard(dir) {
  return spawnSync(process.execPath, [path.join(dir, guard)], {
    cwd: dir,
    encoding: "utf8",
  });
}

test("passes on the current tree", () => {
  const result = spawnSync(process.execPath, [path.join(repoRoot, guard)], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /ai\.archastro\.Astroshots/);
});

test("passes on the fixture copy of the current tree", () => {
  withFixtureRepo((dir) => {
    const result = runGuard(dir);
    assert.equal(result.status, 0, result.stdout + result.stderr);
  });
});

test("fails when a wrong-prefix domain string is introduced", () => {
  withFixtureRepo((dir, git) => {
    // The plausible-but-wrong guess, assembled so this test file itself does
    // not contain the literal the guard rejects.
    const wrongDomain = ["com", "archastro", "Astroshots"].join(".");
    fs.writeFileSync(
      path.join(dir, "scripts/read-watch-roots.sh"),
      `#!/usr/bin/env bash\ndefaults read ${wrongDomain} watchRoots\n`,
    );
    git("add", "--all");

    const result = runGuard(dir);
    assert.equal(result.status, 1, "guard must reject a wrong-prefix domain");
    assert.match(result.stderr, /scripts\/read-watch-roots\.sh:2/);
    assert.match(result.stderr, /wrong preferences\/bundle domain/);
    assert.ok(
      result.stderr.includes(wrongDomain),
      "guard must name the offending domain",
    );
  });
});

test("fails on a wrong-prefix domain in any tracked file type", () => {
  for (const target of ["docs/PREFERENCES.md", "macos/Astroshots/App/Preferences.swift"]) {
    withFixtureRepo((dir, git) => {
      const wrongDomain = ["com", "archastro", "Astroshots"].join(".");
      fs.appendFileSync(
        path.join(dir, target),
        `\n// read ${wrongDomain} for watch roots\n`,
      );
      git("add", "--all");

      const result = runGuard(dir);
      assert.equal(result.status, 1, `guard must reject a wrong domain in ${target}`);
      assert.match(result.stderr, new RegExp(target.replace(/[/.]/g, "\\$&")));
    });
  }
});

test("allows a deliberately marked wrong-domain example", () => {
  withFixtureRepo((dir, git) => {
    const wrongDomain = ["com", "archastro", "Astroshots"].join(".");
    fs.appendFileSync(
      path.join(dir, "docs/PREFERENCES.md"),
      `\nCounter-example: \`${wrongDomain}\` does not exist. <!-- domain-guard: allow -->\n`,
    );
    git("add", "--all");

    const result = runGuard(dir);
    assert.equal(result.status, 0, result.stdout + result.stderr);
    assert.match(result.stdout, /allowed wrong-domain example/);
  });
});

test("keeps the bundle identifier tied to project.yml instead of a literal", () => {
  withFixtureRepo((dir, git) => {
    const projectYml = path.join(dir, "macos/project.yml");
    const text = fs.readFileSync(projectYml, "utf8");
    // Rename the app without regenerating the Xcode project: the guard must
    // notice the derived identifier is no longer the one shipped downstream.
    fs.writeFileSync(
      projectYml,
      text.replace(
        /PRODUCT_BUNDLE_IDENTIFIER: ai\.archastro\.Astroshots/,
        "PRODUCT_BUNDLE_IDENTIFIER: ai.archastro.Renamed",
      ),
    );
    git("add", "--all");

    const result = runGuard(dir);
    assert.equal(result.status, 1, "guard must detect a project.yml/pbxproj mismatch");
    assert.match(result.stderr, /ai\.archastro\.Renamed/);
    assert.match(result.stderr, /xcodegen|must state the canonical domain/);
  });
});

test("fails when the documented watch-root precedence stops matching the app", () => {
  withFixtureRepo((dir, git) => {
    const swift = path.join(dir, "macos/Astroshots/App/Preferences.swift");
    const text = fs.readFileSync(swift, "utf8");
    // Drop the legacy singular fallback: upgraders would silently look
    // unconfigured, and the documented contract would be a lie.
    const stripped = text.replace(
      /\n *if let legacy = defaults\.string\(forKey: Key\.watchRoot\)[\s\S]*?\n *\}\n/,
      "\n",
    );
    assert.notEqual(stripped, text, "fixture edit must change the getter");
    fs.writeFileSync(swift, stripped);
    git("add", "--all");

    const result = runGuard(dir);
    assert.equal(result.status, 1, "guard must detect a dropped legacy read");
    assert.match(result.stderr, /watchRootPaths must read both/);
  });
});

test("fails when the contract doc loses its discoverability links", () => {
  withFixtureRepo((dir, git) => {
    const readme = path.join(dir, "README.md");
    fs.writeFileSync(
      readme,
      fs.readFileSync(readme, "utf8").replaceAll("docs/PREFERENCES.md", "docs/nowhere.md"),
    );
    git("add", "--all");

    const result = runGuard(dir);
    assert.equal(result.status, 1, "guard must require the README link");
    assert.match(result.stderr, /README\.md must link the preferences contract/);
  });
});

test("fails when a new preference key is left undocumented", () => {
  withFixtureRepo((dir, git) => {
    const swift = path.join(dir, "macos/Astroshots/App/Preferences.swift");
    fs.writeFileSync(
      swift,
      fs
        .readFileSync(swift, "utf8")
        .replace(
          '        static let watchRoots = "watchRoots"',
          '        static let watchRoots = "watchRoots"\n        static let undocumentedKnob = "undocumentedKnob"',
        ),
    );
    git("add", "--all");

    const result = runGuard(dir);
    assert.equal(result.status, 1, "guard must require new keys to be documented");
    assert.match(result.stderr, /does not mention the preference key "undocumentedKnob"/);
  });
});
