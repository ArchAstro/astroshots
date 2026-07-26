import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { loadConfig, resolvePackageRoot } from "./config.js";

describe("loadConfig", () => {
  it("resolves filesystem settings relative to a TypeScript config", async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "react-shot-config-"));
    const configPath = path.join(directory, "react-shot.config.ts");
    try {
      fs.writeFileSync(
        configPath,
        `export default {
        root: "./app",
        alias: { "@": "./app/src" },
        styles: ["./app/global.css"],
        postcssConfig: "./postcss.config.mjs"
      };`,
      );

      const config = await loadConfig(configPath);

      expect(config).toMatchObject({
        root: path.join(directory, "app"),
        alias: { "@": path.join(directory, "app/src") },
        styles: [path.join(directory, "app/global.css")],
        postcssConfig: path.join(directory, "postcss.config.mjs"),
      });
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
});

describe("resolvePackageRoot", () => {
  it("uses the nearest package root for a nested fixture", () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "react-shot-root-"));
    const fixture = path.join(directory, "fixtures", "nested", "card.tsx");
    try {
      fs.mkdirSync(path.dirname(fixture), { recursive: true });
      fs.writeFileSync(path.join(directory, "package.json"), "{}");
      fs.writeFileSync(fixture, "export default {}");

      expect(resolvePackageRoot(fixture)).toBe(directory);
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
});
