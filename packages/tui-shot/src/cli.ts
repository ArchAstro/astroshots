#!/usr/bin/env node
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";

import YAML from "yaml";

import { resolveBatchOutputPaths } from "./batch-paths.js";
import { takePtyShot } from "./pty-shot.js";
import { closeSharedBrowser, takeTuiShot } from "./shot.js";
import type { BatchManifest } from "./types.js";

type Flags = Record<string, string | boolean>;
const localRequire = createRequire(import.meta.url);

function help(): void {
  console.log(`tui-shot — deterministic PNG screenshots of terminal interfaces

Usage:
  tui-shot install-browser [--with-deps]
  tui-shot shot <fixture.tsx> -o <out.png> [options]
  tui-shot pty <fixture.yaml|json> -o <out.png> [options]
  tui-shot batch <manifest.yaml|json> [options]

Options:
  -o, --out <path>       Output PNG path
  --cols <count>         Override terminal columns
  --rows <count>         Override terminal rows
  --scale <factor>       Override PNG device scale factor
  --out-dir <path>       Override batch output directory
  --headed               Show Chromium while capturing
  --with-deps            Install Chromium system dependencies (Linux)
  -h, --help             Show this help
`);
}

function takeValue(args: string[], flag: string): string {
  const value = args.shift();
  if (!value || value.startsWith("-")) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function parseArgs(argv: string[]): {
  flags: Flags;
  positionals: string[];
} {
  const args = [...argv];
  const flags: Flags = {};
  const positionals: string[] = [];
  while (args.length) {
    const value = args.shift()!;
    if (value === "-h" || value === "--help") flags.help = true;
    else if (value === "--headed") flags.headed = true;
    else if (value === "--with-deps") flags.withDeps = true;
    else if (value === "-o" || value === "--out")
      flags.out = takeValue(args, value);
    else if (value === "--cols") flags.cols = takeValue(args, value);
    else if (value === "--rows") flags.rows = takeValue(args, value);
    else if (value === "--scale") flags.scale = takeValue(args, value);
    else if (value === "--out-dir") flags.outDir = takeValue(args, value);
    else if (value.startsWith("-")) throw new Error(`Unknown flag: ${value}`);
    else positionals.push(value);
  }
  return { flags, positionals };
}

function numberFlag(
  flags: Flags,
  key: string,
  options: { integer?: boolean; maximum: number },
): number | undefined {
  const raw = flags[key];
  if (raw === undefined) return undefined;
  const value = Number(raw);
  if (
    !Number.isFinite(value) ||
    value <= 0 ||
    value > options.maximum ||
    (options.integer && !Number.isInteger(value))
  ) {
    const kind = options.integer ? "positive integer" : "positive number";
    throw new Error(
      `--${key} must be a ${kind} no greater than ${options.maximum}`,
    );
  }
  return value;
}

function captureOverrides(flags: Flags) {
  return {
    cols: numberFlag(flags, "cols", { integer: true, maximum: 1_000 }),
    rows: numberFlag(flags, "rows", { integer: true, maximum: 1_000 }),
    scale: numberFlag(flags, "scale", { maximum: 4 }),
  };
}

function assertPngPath(outPath: string): void {
  if (path.extname(outPath).toLowerCase() !== ".png") {
    throw new Error(`Output must use a .png extension: ${outPath}`);
  }
}

function installBrowser(withDeps: boolean): void {
  const playwrightRoot = path.dirname(
    localRequire.resolve("playwright/package.json"),
  );
  const result = spawnSync(
    process.execPath,
    [
      path.join(playwrightRoot, "cli.js"),
      "install",
      ...(withDeps ? ["--with-deps"] : []),
      "chromium",
    ],
    { stdio: "inherit" },
  );
  if (result.error) {
    throw new Error(`Could not start Playwright installer: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`Playwright browser installation exited with status ${result.status}`);
  }
}

async function shot(fixturePath: string, flags: Flags): Promise<void> {
  const outPath = flags.out;
  if (typeof outPath !== "string") {
    throw new Error("shot requires -o <out.png>");
  }
  assertPngPath(outPath);
  const written = await takeTuiShot({
    fixturePath,
    outPath,
    headed: Boolean(flags.headed),
    ...captureOverrides(flags),
  });
  console.log(`wrote ${written}`);
}

async function pty(fixturePath: string, flags: Flags): Promise<void> {
  const outPath = flags.out;
  if (typeof outPath !== "string") {
    throw new Error("pty requires -o <out.png>");
  }
  assertPngPath(outPath);
  const written = await takePtyShot({
    fixturePath,
    outPath,
    headed: Boolean(flags.headed),
    ...captureOverrides(flags),
  });
  console.log(`wrote ${written}`);
}

function parseManifest(absolute: string): BatchManifest {
  const raw = fs.readFileSync(absolute, "utf8");
  const value: unknown = absolute.endsWith(".json")
    ? JSON.parse(raw)
    : YAML.parse(raw);
  if (!value || typeof value !== "object" || !Array.isArray((value as BatchManifest).shots)) {
    throw new Error(`No shots listed in ${absolute}`);
  }
  const manifest = value as BatchManifest;
  if (
    manifest.shots.length === 0 ||
    manifest.shots.some(
      (entry) =>
        !entry ||
        typeof entry.fixture !== "string" ||
        !entry.fixture ||
        typeof entry.out !== "string" ||
        !entry.out,
    )
  ) {
    throw new Error(`Every shot in ${absolute} needs fixture and out paths`);
  }
  return manifest;
}

async function batch(manifestPath: string, flags: Flags): Promise<void> {
  const absolute = path.resolve(manifestPath);
  const manifest = parseManifest(absolute);
  const base = path.dirname(absolute);
  const outDir =
    typeof flags.outDir === "string" ? path.resolve(flags.outDir) : null;
  const outPaths = resolveBatchOutputPaths(manifest.shots, base, outDir);
  for (const outPath of outPaths) assertPngPath(outPath);
  const overrides = captureOverrides(flags);
  let completed = 0;
  for (const [index, entry] of manifest.shots.entries()) {
    const fixturePath = path.resolve(base, entry.fixture);
    const outPath = outPaths[index]!;
    process.stdout.write(
      `shot ${path.relative(process.cwd(), fixturePath)} … `,
    );
    await takeTuiShot({
      fixturePath,
      outPath,
      headed: Boolean(flags.headed),
      ...overrides,
    });
    console.log(`→ ${path.relative(process.cwd(), outPath)}`);
    completed++;
  }
  console.log(`done: ${completed}/${manifest.shots.length} shots`);
}

async function main(): Promise<void> {
  const argv = process.argv.slice(2);
  const command = argv[0];
  if (!command || command === "help" || command === "-h" || command === "--help") {
    help();
    if (!command) process.exitCode = 1;
    return;
  }
  const { flags, positionals } = parseArgs(argv.slice(1));
  if (flags.help) {
    help();
    return;
  }
  if (command === "install-browser") {
    if (positionals.length) {
      throw new Error("install-browser does not accept arguments");
    }
    installBrowser(Boolean(flags.withDeps));
  } else if (command === "shot") {
    if (!positionals[0]) throw new Error("shot requires a fixture path");
    await shot(positionals[0], flags);
  } else if (command === "pty") {
    if (!positionals[0]) throw new Error("pty requires a fixture path");
    await pty(positionals[0], flags);
  } else if (command === "batch") {
    if (!positionals[0]) throw new Error("batch requires a manifest path");
    await batch(positionals[0], flags);
  } else {
    throw new Error(`Unknown command: ${command}`);
  }
}

main()
  .catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  })
  .finally(closeSharedBrowser);
