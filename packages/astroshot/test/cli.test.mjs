import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const packageRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const executable = path.join(packageRoot, "bin", "astroshot.mjs");

function run(...arguments_) {
  return spawnSync(process.execPath, [executable, ...arguments_], {
    cwd: packageRoot,
    encoding: "utf8",
  });
}

test("documents both screenshot modes from one executable", () => {
  const result = run("--help");
  assert.equal(result.status, 0);
  assert.match(result.stdout, /astroshot react/);
  assert.match(result.stdout, /astroshot tui/);
  assert.match(result.stdout, /install-browser/);
});

test("reports the unified package version", () => {
  const result = run("--version");
  assert.equal(result.status, 0);
  assert.match(result.stdout, /^0\.1\.0\s*$/);
});

test("delegates mode-specific help to both engines", () => {
  const react = run("react", "--help");
  const tui = run("tui", "--help");
  assert.equal(react.status, 0, react.stderr);
  assert.equal(tui.status, 0, tui.stderr);
  assert.match(react.stdout, /astroshot react/);
  assert.match(tui.stdout, /astroshot tui/);
});

test("rejects an unknown screenshot mode", () => {
  const result = run("browser");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Unknown command: browser/);
});
