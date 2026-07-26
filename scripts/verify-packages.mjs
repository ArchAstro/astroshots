#!/usr/bin/env node
/**
 * Pack both public workspaces, install their tarballs into a clean consumer,
 * and exercise the exact executable names that npx exposes.
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "astroshots-pack-"));
const archiveDir = path.join(tempRoot, "archives");
const consumerDir = path.join(tempRoot, "consumer");

fs.mkdirSync(archiveDir, { recursive: true });
fs.mkdirSync(consumerDir, { recursive: true });

function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: options.cwd ?? repoRoot,
    encoding: options.encoding,
    env: { ...process.env, ...options.env },
    stdio: options.encoding ? ["ignore", "pipe", "inherit"] : "inherit",
  });
}

function pack(packageDirectory) {
  const before = new Set(fs.readdirSync(archiveDir));
  run("npm", [
    "pack",
    path.join(repoRoot, packageDirectory),
    "--pack-destination",
    archiveDir,
  ]);
  const created = fs
    .readdirSync(archiveDir)
    .filter((file) => file.endsWith(".tgz") && !before.has(file));
  if (created.length !== 1) {
    throw new Error(
      `expected one tarball for ${packageDirectory}, found ${created.length}`,
    );
  }
  return path.join(archiveDir, created[0]);
}

try {
  const reactTarball = pack("packages/react-shot");
  const tuiTarball = pack("packages/tui-shot");

  fs.writeFileSync(
    path.join(consumerDir, "package.json"),
    JSON.stringify(
      {
        name: "astroshots-clean-consumer",
        version: "1.0.0",
        private: true,
        type: "module",
      },
      null,
      2,
    ) + "\n",
  );

  run(
    "npm",
    [
      "install",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      reactTarball,
      tuiTarball,
      "react@19",
      "react-dom@19",
      "ink@7",
    ],
    { cwd: consumerDir },
  );

  run("npx", ["--no-install", "react-shot", "--help"], {
    cwd: consumerDir,
  });
  run("npx", ["--no-install", "tui-shot", "--help"], {
    cwd: consumerDir,
  });

  for (const packageName of ["react-shot", "tui-shot"]) {
    const packageRoot = path.join(
      consumerDir,
      "node_modules",
      "@archastro",
      packageName,
    );
    for (const requiredPath of [
      "package.json",
      `bin/${packageName}.mjs`,
      "dist/index.js",
      "dist/index.d.ts",
    ]) {
      if (!fs.existsSync(path.join(packageRoot, requiredPath))) {
        throw new Error(
          `@archastro/${packageName} tarball is missing ${requiredPath}`,
        );
      }
    }
  }

  if (process.env.ASTROSHOTS_VERIFY_PACKAGES_CAPTURE === "1") {
    fs.writeFileSync(
      path.join(consumerDir, "react-fixture.tsx"),
      `import React from "react";
export default {
  width: 420,
  height: 160,
  selector: "[data-pack-proof]",
  waitFor: "text=Packed React CLI",
  component: React.createElement("div", {
    "data-pack-proof": true,
    style: { padding: "24px", background: "#111827", color: "white" }
  }, "Packed React CLI")
};
`,
    );
    fs.writeFileSync(
      path.join(consumerDir, "tui-fixture.tsx"),
      `import React, { useState } from "react";
import { Box, Text } from "ink";

function PackProof() {
  const [label] = useState("Packed terminal CLI");
  return React.createElement(
    Box,
    { borderStyle: "round", paddingX: 1 },
    React.createElement(Text, { color: "cyan" }, label)
  );
}

export default {
  cols: 36,
  rows: 5,
  scale: 1,
  expectText: ["Packed terminal CLI"],
  component: React.createElement(PackProof)
};
`,
    );

    run(
      "npx",
      [
        "--no-install",
        "react-shot",
        "shot",
        "./react-fixture.tsx",
        "-o",
        "./react-proof.png",
      ],
      { cwd: consumerDir },
    );
    run(
      "npx",
      [
        "--no-install",
        "tui-shot",
        "shot",
        "./tui-fixture.tsx",
        "-o",
        "./tui-proof.png",
      ],
      { cwd: consumerDir },
    );

    for (const image of ["react-proof.png", "tui-proof.png"]) {
      const imagePath = path.join(consumerDir, image);
      if (!fs.existsSync(imagePath) || fs.statSync(imagePath).size === 0) {
        throw new Error(`clean consumer did not generate ${image}`);
      }
    }
  }

  console.log(
    "verify-packages: tarballs install cleanly and expose both npx commands",
  );
} finally {
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
