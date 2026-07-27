#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

function help() {
  console.log(`astroshot — one CLI for React and terminal UI screenshots

Usage:
  astroshot react <fixture.tsx> -o <out.png> [options]
  astroshot react shot <fixture.tsx> -o <out.png> [options]
  astroshot react batch <manifest.yaml|json> [options]
  astroshot tui <fixture.tsx> -o <out.png> [options]
  astroshot tui shot <fixture.tsx> -o <out.png> [options]
  astroshot tui batch <manifest.yaml|json> [options]
  astroshot install-browser [--with-deps]

Commands:
  react              Capture an isolated React component
  tui                Capture an Ink terminal UI state
  install-browser    Install the shared Chromium runtime

Run "astroshot react --help" or "astroshot tui --help" for mode options.`);
}

function modeHelp(mode) {
  if (mode === "react") {
    console.log(`astroshot react — deterministic React component screenshots

Usage:
  astroshot react <fixture.tsx> -o <out.png> [options]
  astroshot react batch <manifest.yaml|json> [options]

Options:
  -o, --out <path>       Output PNG path
  --root <dir>           Package root for imports and aliases
  --config <path>        Fixture configuration path
  --width <px>           Override viewport width
  --height <px>          Override viewport height
  --headed               Show Chromium for debugging
  -h, --help             Show this help`);
    return;
  }

  console.log(`astroshot tui — deterministic Ink terminal UI screenshots

Usage:
  astroshot tui <fixture.tsx> -o <out.png> [options]
  astroshot tui batch <manifest.yaml|json> [options]

Options:
  -o, --out <path>       Output PNG path
  --cols <count>         Override terminal columns
  --rows <count>         Override terminal rows
  --scale <factor>       Override PNG device scale factor
  --out-dir <path>       Override batch output directory
  --headed               Show Chromium for debugging
  -h, --help             Show this help`);
}

function engineBin(mode) {
  const packageName =
    mode === "react" ? "@archastro/react-shot" : "@archastro/tui-shot";
  const executable = mode === "react" ? "react-shot.mjs" : "tui-shot.mjs";
  const entry = fileURLToPath(import.meta.resolve(packageName));
  const packageRoot = path.dirname(path.dirname(entry));
  return path.join(packageRoot, "bin", executable);
}

function runEngine(mode, arguments_) {
  const normalized =
    arguments_[0] && /\.[cm]?tsx?$/.test(arguments_[0])
      ? ["shot", ...arguments_]
      : arguments_;
  const result = spawnSync(
    process.execPath,
    [engineBin(mode), ...normalized],
    { stdio: "inherit" },
  );
  if (result.error) {
    console.error(`astroshot could not start ${mode} capture: ${result.error.message}`);
    process.exit(1);
  }
  if (result.signal) process.kill(process.pid, result.signal);
  process.exit(result.status ?? 1);
}

const [command, ...arguments_] = process.argv.slice(2);

if (command === "-v" || command === "--version") {
  const packageJSON = JSON.parse(
    fs.readFileSync(new URL("../package.json", import.meta.url), "utf8"),
  );
  console.log(packageJSON.version);
  process.exit(0);
}

if (!command || command === "help" || command === "-h" || command === "--help") {
  help();
  process.exit(command ? 0 : 1);
}

if (command === "install-browser") {
  runEngine("react", ["install-browser", ...arguments_]);
}

if (command === "react" || command === "tui") {
  if (
    arguments_.length === 0 ||
    arguments_[0] === "help" ||
    arguments_[0] === "-h" ||
    arguments_[0] === "--help"
  ) {
    modeHelp(command);
    process.exit(arguments_.length === 0 ? 1 : 0);
  }
  runEngine(command, arguments_);
}

console.error(`Unknown command: ${command}`);
help();
process.exit(1);
