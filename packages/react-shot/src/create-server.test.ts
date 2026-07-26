import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  resolveInstalledModule,
  resolveInstalledPackage,
} from "./create-server.js";

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

      expect(resolveInstalledModule(packageRoot, "@tailwindcss/vite")).toBe(
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

      expect(resolveInstalledPackage(packageRoot, "react")).toBe(
        fs.realpathSync(reactRoot),
      );
    } finally {
      fs.rmSync(workspace, { recursive: true, force: true });
    }
  });
});
