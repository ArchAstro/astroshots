// Ports packages/react-shot/src/create-server.test.ts against the helper's
// own copies of `resolveInstalledModule` / `resolveInstalledPackage`.
// Run with `node --test rust/node-helper/helper.test.mjs`; `cargo test` runs
// it through rust/astroshot/tests/node_helper.rs.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { resolveInstalledModule, resolveInstalledPackage } from "./helper.mjs";

describe("resolveInstalledModule", () => {
  it("finds a package hoisted above the fixture package root", () => {
    const workspace = fs.mkdtempSync(
      path.join(os.tmpdir(), "react-shot-resolve-"),
    );
    const packageRoot = path.join(workspace, "packages", "web");
    const tailwindRoot = path.join(
      workspace,
      "node_modules",
      "@tailwindcss",
      "vite",
    );
    const entry = path.join(tailwindRoot, "index.mjs");

    try {
      fs.mkdirSync(packageRoot, { recursive: true });
      fs.mkdirSync(tailwindRoot, { recursive: true });
      fs.writeFileSync(
        path.join(tailwindRoot, "package.json"),
        JSON.stringify({
          name: "@tailwindcss/vite",
          type: "module",
          exports: "./index.mjs",
        }),
      );
      fs.writeFileSync(entry, "export default () => ({ name: 'fake' });\n");

      assert.equal(
        resolveInstalledModule(packageRoot, "@tailwindcss/vite"),
        fs.realpathSync(entry),
      );
    } finally {
      fs.rmSync(workspace, { recursive: true, force: true });
    }
  });

  it("finds a hoisted package root even when its export is nested", () => {
    const workspace = fs.mkdtempSync(
      path.join(os.tmpdir(), "react-shot-package-resolve-"),
    );
    const packageRoot = path.join(workspace, "packages", "web");
    const reactRoot = path.join(workspace, "node_modules", "react");
    const entry = path.join(reactRoot, "dist", "index.js");

    try {
      fs.mkdirSync(packageRoot, { recursive: true });
      fs.mkdirSync(path.dirname(entry), { recursive: true });
      fs.writeFileSync(
        path.join(reactRoot, "package.json"),
        JSON.stringify({
          name: "react",
          exports: "./dist/index.js",
        }),
      );
      fs.writeFileSync(entry, "module.exports = {};\n");

      assert.equal(
        resolveInstalledPackage(packageRoot, "react"),
        fs.realpathSync(reactRoot),
      );
    } finally {
      fs.rmSync(workspace, { recursive: true, force: true });
    }
  });
});
