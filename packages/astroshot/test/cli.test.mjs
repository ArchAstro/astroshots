import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const packageRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const executable = path.join(packageRoot, "bin", "astroshot.mjs");

function runIn(cwd, ...arguments_) {
  return spawnSync(process.execPath, [executable, ...arguments_], {
    cwd,
    encoding: "utf8",
  });
}

function run(...arguments_) {
  return runIn(packageRoot, ...arguments_);
}

test("documents React, Ink, and arbitrary PTY modes from one executable", () => {
  const result = run("--help");
  assert.equal(result.status, 0);
  assert.match(result.stdout, /astroshot react/);
  assert.match(result.stdout, /astroshot ink/);
  assert.match(result.stdout, /astroshot pty/);
  assert.match(result.stdout, /astroshot init/);
  assert.match(result.stdout, /alias for "astroshot ink"/);
  assert.match(result.stdout, /install-browser/);
});

test("reports the unified package version", () => {
  const result = run("--version");
  assert.equal(result.status, 0);
  assert.match(result.stdout, /^0\.1\.0\s*$/);
});

test("documents each mode and keeps tui as an Ink compatibility alias", () => {
  const react = run("react", "--help");
  const ink = run("ink", "--help");
  const tui = run("tui", "--help");
  const pty = run("pty", "--help");
  assert.equal(react.status, 0, react.stderr);
  assert.equal(ink.status, 0, ink.stderr);
  assert.equal(tui.status, 0, tui.stderr);
  assert.equal(pty.status, 0, pty.stderr);
  assert.match(react.stdout, /astroshot react/);
  assert.match(ink.stdout, /astroshot ink/);
  assert.match(tui.stdout, /astroshot ink/);
  assert.match(pty.stdout, /arbitrary terminal programs/);
});

test("generates valid fixture templates for every capture mode", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-init-"));
  try {
    const react = runIn(tempDir, "init", "react");
    const ink = runIn(tempDir, "init", "ink", "screens/ready.tsx");
    const pty = runIn(tempDir, "init", "pty");
    const ptyJson = runIn(tempDir, "init", "pty", "screens/ready.json");
    assert.equal(react.status, 0, react.stderr);
    assert.equal(ink.status, 0, ink.stderr);
    assert.equal(pty.status, 0, pty.stderr);
    assert.equal(ptyJson.status, 0, ptyJson.stderr);

    assert.match(
      fs.readFileSync(path.join(tempDir, "react.shot.tsx"), "utf8"),
      /ReactShotFixture/,
    );
    assert.match(
      fs.readFileSync(path.join(tempDir, "screens/ready.tsx"), "utf8"),
      /InkShotFixture/,
    );
    assert.match(
      fs.readFileSync(path.join(tempDir, "pty.shot.yaml"), "utf8"),
      /command: \.\/target\/debug\/my-tui/,
    );
    assert.equal(
      JSON.parse(
        fs.readFileSync(path.join(tempDir, "screens/ready.json"), "utf8"),
      ).command,
      "./target/debug/my-tui",
    );

    const collision = runIn(tempDir, "init", "react");
    assert.equal(collision.status, 1);
    assert.match(collision.stderr, /Refusing to overwrite/);
    const forced = runIn(tempDir, "init", "react", "--force");
    assert.equal(forced.status, 0, forced.stderr);

    const sensitivePath = path.join(tempDir, "sensitive.txt");
    const symlinkPath = path.join(tempDir, "linked.tsx");
    fs.writeFileSync(sensitivePath, "do not replace");
    fs.symlinkSync(sensitivePath, symlinkPath);
    const symlink = runIn(tempDir, "init", "react", "linked.tsx", "--force");
    assert.equal(symlink.status, 1);
    assert.match(symlink.stderr, /Refusing to replace symbolic link/);
    assert.equal(fs.readFileSync(sensitivePath, "utf8"), "do not replace");
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

test("rejects an unknown screenshot mode", () => {
  const result = run("browser");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Unknown command: browser/);
});
