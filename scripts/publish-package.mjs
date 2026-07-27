#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageName = process.argv[2];
const allowedPackages = new Map([
  ["@archastro/react-shot", "packages/react-shot/package.json"],
  ["@archastro/tui-shot", "packages/tui-shot/package.json"],
  ["@archastro/astroshot", "packages/astroshot/package.json"],
]);

if (!allowedPackages.has(packageName)) {
  console.error(
    `Usage: node scripts/publish-package.mjs ${[...allowedPackages.keys()].join("|")}`,
  );
  process.exit(1);
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(
  fs.readFileSync(path.join(root, allowedPackages.get(packageName)), "utf8"),
);
const registry = "https://registry.npmjs.org";

function npmView(field) {
  const result = spawnSync(
    "npm",
    [
      "view",
      `${packageName}@${manifest.version}`,
      field,
      "--json",
      "--registry",
      registry,
    ],
    { cwd: root, encoding: "utf8" },
  );

  if (result.status === 0) return JSON.parse(result.stdout);

  const error = `${result.stdout}\n${result.stderr}`;
  if (error.includes("E404")) return null;
  throw new Error(
    `npm view failed for ${packageName}@${manifest.version}\n${error}`,
  );
}

const publishedVersion = npmView("version");
if (publishedVersion === null) {
  execFileSync(
    "npm",
    ["publish", "--workspace", packageName, "--access", "public", "--registry", registry],
    { cwd: root, stdio: "inherit" },
  );
  process.exit(0);
}

if (publishedVersion !== manifest.version) {
  throw new Error(
    `Registry returned unexpected version ${publishedVersion} for ${packageName}@${manifest.version}`,
  );
}

const pack = JSON.parse(
  execFileSync(
    "npm",
    ["pack", "--workspace", packageName, "--dry-run", "--json"],
    { cwd: root, encoding: "utf8" },
  ),
);
const localIntegrity = pack[0]?.integrity;
const publishedIntegrity = npmView("dist.integrity");

if (!localIntegrity || publishedIntegrity !== localIntegrity) {
  throw new Error(
    `${packageName}@${manifest.version} already exists with different package contents`,
  );
}

console.log(
  `${packageName}@${manifest.version} is already published with matching contents; skipping.`,
);
