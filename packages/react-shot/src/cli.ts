#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import YAML from "yaml";
import { resolveBatchOutputPaths } from "./batch-paths.js";
import { closeSharedBrowser, takeShot } from "./shot.js";
import type { BatchManifest } from "./types.js";

type Flags = Record<string, string | boolean>;

function packageVersion(): string {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const packagePath = path.resolve(here, "../package.json");
  const parsed = JSON.parse(fs.readFileSync(packagePath, "utf8")) as {
    version?: string;
  };
  return parsed.version ?? "unknown";
}

function printHelp(): void {
  console.log(`react-shot - deterministic React component screenshots

Usage:
  react-shot shot <fixture.tsx> -o <out.png> [options]
  react-shot batch <manifest.yaml|json> [options]
  react-shot <fixture.tsx> -o <out.png> [options]
  react-shot install-browser [--with-deps]

Options:
  -o, --out <path>       Output PNG path
  --root <dir>           Package root for imports and aliases
  --config <path>        Config path; otherwise discovered from the fixture
  --width <px>           Override viewport width
  --height <px>          Override viewport height
  --headed               Show Chromium for debugging
  --with-deps            Install Chromium OS dependencies too
  -v, --version          Print the installed version
  -h, --help             Show this help

Run "react-shot install-browser" once before the first capture.
`);
}

function requiredValue(args: string[], flag: string): string {
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

  while (args.length > 0) {
    const argument = args.shift()!;
    if (argument === "-h" || argument === "--help") flags.help = true;
    else if (argument === "-v" || argument === "--version") flags.version = true;
    else if (argument === "--headed") flags.headed = true;
    else if (argument === "--with-deps") flags.withDeps = true;
    else if (argument === "-o" || argument === "--out") {
      flags.out = requiredValue(args, argument);
    } else if (argument === "--root") {
      flags.root = requiredValue(args, argument);
    } else if (argument === "--config") {
      flags.config = requiredValue(args, argument);
    } else if (argument === "--width") {
      flags.width = requiredValue(args, argument);
    } else if (argument === "--height") {
      flags.height = requiredValue(args, argument);
    } else if (argument.startsWith("-")) {
      throw new Error(`Unknown option: ${argument}`);
    } else {
      positionals.push(argument);
    }
  }
  return { flags, positionals };
}

function dimension(flags: Flags, name: "width" | "height"): number | undefined {
  const raw = flags[name];
  if (raw === undefined) return undefined;
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 1 || value > 10_000) {
    throw new Error(`--${name} must be an integer between 1 and 10000`);
  }
  return value;
}

async function captureFixture(fixture: string, flags: Flags): Promise<void> {
  const output = flags.out;
  if (typeof output !== "string") {
    throw new Error("A screenshot requires -o <out.png>");
  }
  const outPath = await takeShot({
    fixturePath: fixture,
    outPath: output,
    root: typeof flags.root === "string" ? flags.root : undefined,
    configPath:
      typeof flags.config === "string" ? flags.config : undefined,
    headed: Boolean(flags.headed),
    width: dimension(flags, "width"),
    height: dimension(flags, "height"),
  });
  console.log(`wrote ${outPath}`);
}

function manifestRelative(
  baseDirectory: string,
  value: string | undefined,
): string | undefined {
  return value ? path.resolve(baseDirectory, value) : undefined;
}

async function captureBatch(manifestPath: string, flags: Flags): Promise<void> {
  const absoluteManifest = path.resolve(manifestPath);
  const raw = fs.readFileSync(absoluteManifest, "utf8");
  const manifest = (
    absoluteManifest.endsWith(".json") ? JSON.parse(raw) : YAML.parse(raw)
  ) as BatchManifest;
  if (!Array.isArray(manifest?.shots) || manifest.shots.length === 0) {
    throw new Error(`No shots listed in ${absoluteManifest}`);
  }

  const baseDirectory = path.dirname(absoluteManifest);
  for (const entry of manifest.shots) {
    if (!entry.fixture || !entry.out) {
      throw new Error("Each batch entry requires fixture and out");
    }
  }
  const outPaths = resolveBatchOutputPaths(manifest.shots, baseDirectory);
  const cliWidth = dimension(flags, "width");
  const cliHeight = dimension(flags, "height");
  let completed = 0;

  for (const [index, entry] of manifest.shots.entries()) {
    const fixturePath = path.resolve(baseDirectory, entry.fixture);
    const outPath = outPaths[index]!;
    const root =
      typeof flags.root === "string"
        ? path.resolve(flags.root)
        : manifestRelative(baseDirectory, entry.root ?? manifest.root);
    const configPath =
      typeof flags.config === "string"
        ? path.resolve(flags.config)
        : manifestRelative(baseDirectory, entry.config ?? manifest.config);

    process.stdout.write(
      `shot ${path.relative(process.cwd(), fixturePath)} ... `,
    );
    await takeShot({
      fixturePath,
      outPath,
      root,
      configPath,
      headed: Boolean(flags.headed),
      width: cliWidth ?? entry.width,
      height: cliHeight ?? entry.height,
    });
    completed += 1;
    console.log(`wrote ${path.relative(process.cwd(), outPath)}`);
  }
  console.log(`done: ${completed}/${manifest.shots.length} shots`);
}

function installBrowser(flags: Flags): void {
  const require = createRequire(import.meta.url);
  const packagePath = require.resolve("playwright/package.json");
  const cliPath = path.join(path.dirname(packagePath), "cli.js");
  const arguments_ = ["install"];
  if (flags.withDeps) arguments_.push("--with-deps");
  arguments_.push("chromium");

  const result = spawnSync(process.execPath, [cliPath, ...arguments_], {
    stdio: "inherit",
  });
  if (result.error) {
    throw new Error(`Could not start Playwright installer: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Playwright browser installation failed with exit code ${result.status ?? "unknown"}`,
    );
  }
}

async function main(): Promise<void> {
  const argv = process.argv.slice(2);
  if (argv.length === 0) {
    printHelp();
    process.exitCode = 1;
    return;
  }
  const { flags, positionals } = parseArgs(argv);
  if (flags.version) {
    console.log(packageVersion());
    return;
  }
  if (flags.help || positionals[0] === "help") {
    printHelp();
    return;
  }

  try {
    if (positionals[0] === "install-browser") {
      installBrowser(flags);
    } else if (positionals[0] === "shot") {
      if (!positionals[1]) throw new Error("shot requires a fixture path");
      await captureFixture(positionals[1], flags);
    } else if (positionals[0] === "batch") {
      if (!positionals[1]) throw new Error("batch requires a manifest path");
      await captureBatch(positionals[1], flags);
    } else if (/\.[cm]?tsx?$/.test(positionals[0] ?? "")) {
      await captureFixture(positionals[0], flags);
    } else {
      throw new Error(`Unknown command: ${positionals[0]}`);
    }
  } finally {
    await closeSharedBrowser();
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
