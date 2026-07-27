#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error("Usage: npm run version:packages -- <version>");
  process.exit(1);
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packagePaths = [
  "packages/react-shot/package.json",
  "packages/tui-shot/package.json",
  "packages/astroshot/package.json",
];

for (const packagePath of packagePaths) {
  const absolutePath = path.join(root, packagePath);
  const packageJson = JSON.parse(fs.readFileSync(absolutePath, "utf8"));
  packageJson.version = version;

  if (packageJson.name === "@archastro/astroshot") {
    packageJson.dependencies["@archastro/react-shot"] = version;
    packageJson.dependencies["@archastro/tui-shot"] = version;
  }

  fs.writeFileSync(absolutePath, `${JSON.stringify(packageJson, null, 2)}\n`);
}

execFileSync("npm", ["install", "--package-lock-only"], {
  cwd: root,
  stdio: "inherit",
});

console.log(
  `Set all screenshot packages and unified renderer dependencies to ${version}.`,
);
