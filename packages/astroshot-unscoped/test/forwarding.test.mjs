import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const wrapperRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repoRoot = path.resolve(wrapperRoot, "..", "..");
const wrapperBin = path.join(wrapperRoot, "bin", "astroshot.mjs");
const cliBin = path.join(
  repoRoot,
  "packages",
  "astroshot",
  "bin",
  "astroshot.mjs",
);

/** Separate pipes for stdout and stderr, so a stream that leaks into the wrong
 * channel fails the assertion instead of hiding inside merged output. */
function run(executable, arguments_, options = {}) {
  return spawnSync(process.execPath, [executable, ...arguments_], {
    cwd: options.cwd ?? wrapperRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function runWrapper(...arguments_) {
  return run(wrapperBin, arguments_);
}

function runCli(...arguments_) {
  return run(cliBin, arguments_);
}

/** Every invocation must be indistinguishable from calling the real CLI. */
function assertMatchesCli(...arguments_) {
  const wrapper = runWrapper(...arguments_);
  const direct = runCli(...arguments_);
  assert.equal(
    wrapper.status,
    direct.status,
    `exit code differs for [${arguments_.join(" ")}]`,
  );
  assert.equal(
    wrapper.stdout,
    direct.stdout,
    `stdout differs for [${arguments_.join(" ")}]`,
  );
  assert.equal(
    wrapper.stderr,
    direct.stderr,
    `stderr differs for [${arguments_.join(" ")}]`,
  );
  return wrapper;
}

test("forwards stdout and a zero exit code", () => {
  const result = assertMatchesCli("--version");
  assert.equal(result.status, 0);
  const cliManifest = JSON.parse(
    fs.readFileSync(
      path.join(repoRoot, "packages", "astroshot", "package.json"),
      "utf8",
    ),
  );
  assert.equal(result.stdout.trim(), cliManifest.version);
  assert.equal(result.stderr, "");
});

test("forwards help output and the non-zero bare-invocation exit code", () => {
  const bare = assertMatchesCli();
  assert.equal(bare.status, 1);
  assert.match(bare.stdout, /astroshot react/);

  const explicit = assertMatchesCli("--help");
  assert.equal(explicit.status, 0);
});

test("forwards stderr and a failing exit code without polluting stdout", () => {
  const result = assertMatchesCli("browser");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Unknown command: browser/);
  assert.doesNotMatch(result.stdout, /Unknown command/);
});

test("forwards multiple arguments, including quoted phrases", () => {
  const result = assertMatchesCli(
    "movie",
    "which-source",
    "ratatui truecolor dashboard",
  );
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /"recommended": "pty"/);
});

test("forwards flags verbatim rather than reinterpreting them", () => {
  const result = assertMatchesCli("init", "react", "--not-a-real-flag");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Unknown init flag: --not-a-real-flag/);
});

test("runs the CLI in the caller's working directory", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-wrapper-"));
  try {
    const result = spawnSync(
      process.execPath,
      [wrapperBin, "init", "pty", "screens/ready.json"],
      { cwd: tempDir, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
    assert.equal(result.status, 0, result.stderr);
    const fixture = JSON.parse(
      fs.readFileSync(path.join(tempDir, "screens", "ready.json"), "utf8"),
    );
    assert.equal(fixture.command, "./target/debug/my-tui");
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});

test("reports a missing bundled CLI instead of failing obscurely", () => {
  // A tarball packed without its bundleDependencies would look like this.
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-nobundle-"));
  try {
    fs.mkdirSync(path.join(tempDir, "bin"), { recursive: true });
    fs.copyFileSync(wrapperBin, path.join(tempDir, "bin", "astroshot.mjs"));
    const result = spawnSync(
      process.execPath,
      [path.join(tempDir, "bin", "astroshot.mjs"), "--version"],
      { cwd: tempDir, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
    assert.equal(result.status, 1);
    assert.match(result.stderr, /bundled @archastro\/astroshot CLI is missing/);
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
});
