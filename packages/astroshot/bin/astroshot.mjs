#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { writeFixtureTemplate } from "./templates.mjs";
import { demoHelp, runDemo } from "./demo.mjs";
import { doctorHelp, runDoctor } from "./doctor.mjs";

function help() {
  console.log(`astroshot — one CLI for React, Ink, PTY stills, and movies

Usage:
  astroshot demo [--feature <name>] [--root <dir>] [--dry-run] [--clean]
  astroshot doctor [--root <dir>] [--json]
  astroshot init react [fixture.tsx] [--force]
  astroshot init ink [fixture.tsx] [--force]
  astroshot init pty [fixture.yaml] [--force]
  astroshot react <fixture.tsx> -o <out.png> [options]
  astroshot react shot <fixture.tsx> -o <out.png> [options]
  astroshot react batch <manifest.yaml|json> [options]
  astroshot ink <fixture.tsx> -o <out.png> [options]
  astroshot ink batch <manifest.yaml|json> [options]
  astroshot pty <fixture.yaml|json> -o <out.png> [options]
  astroshot movie <command> [options]
  astroshot install-browser [--with-deps]

Start here:
  demo               Write a complete .astroshot/ example set (no prerequisites)
  doctor             Check Node, watched folders, app, Chromium, permissions

Commands:
  react              Capture an isolated React component (still PNG)
  ink                Capture an Ink component fixture (still PNG)
  pty                Capture any executable in a pseudoterminal (still PNG)
  movie              Record a journey movie into .astroshot/ (poster + video)
  init               Generate a React, Ink, or PTY fixture template
  install-browser    Install the shared Chromium runtime

Movie sources (see "astroshot movie which-source"):
  browser            Web / agent-browser / Playwright viewport
  pty                Truecolor TUI/CLI (never screenshot Terminal.app)
  desktop.window     Native macOS app window (uses OS screencapture)
  frames             Push your own PNG/JPEG sequence

Compatibility: "astroshot tui" remains an alias for "astroshot ink".
Run "astroshot <mode> --help" for mode options.`);
}

function modeHelp(mode) {
  if (mode === "react") {
    console.log(`astroshot react — deterministic React component screenshots

Usage:
  astroshot init react [fixture.tsx] [--force]
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

  if (mode === "ink") {
    console.log(`astroshot ink — deterministic Ink component screenshots

Usage:
  astroshot init ink [fixture.tsx] [--force]
  astroshot ink <fixture.tsx> -o <out.png> [options]
  astroshot ink batch <manifest.yaml|json> [options]

Options:
  -o, --out <path>       Output PNG path
  --cols <count>         Override terminal columns
  --rows <count>         Override terminal rows
  --scale <factor>       Override PNG device scale factor
  --out-dir <path>       Override batch output directory
  --headed               Show Chromium for debugging
  -h, --help             Show this help`);
    return;
  }

  console.log(`astroshot pty — screenshots of arbitrary terminal programs

Usage:
  astroshot init pty [fixture.yaml] [--force]
  astroshot pty <fixture.yaml|json> -o <out.png> [options]

Options:
  -o, --out <path>       Output PNG path
  --cols <count>         Override terminal columns
  --rows <count>         Override terminal rows
  --scale <factor>       Override PNG device scale factor
  --headed               Show Chromium for debugging
  -h, --help             Show this help`);
}

function engineBin(mode) {
  if (mode === "movie") {
    const entry = fileURLToPath(import.meta.resolve("@archastro/movie-harness"));
    // package exports "." → dist/index.js → package root is two levels up from dist
    const packageRoot = path.dirname(path.dirname(entry));
    return path.join(packageRoot, "bin", "astroshot-movie.mjs");
  }
  const packageName =
    mode === "react" ? "@archastro/react-shot" : "@archastro/tui-shot";
  const executable = mode === "react" ? "react-shot.mjs" : "tui-shot.mjs";
  const entry = fileURLToPath(import.meta.resolve(packageName));
  const packageRoot = path.dirname(path.dirname(entry));
  return path.join(packageRoot, "bin", executable);
}

function runEngine(mode, arguments_) {
  const engineMode = mode === "react" ? "react" : "tui";
  const engineArguments =
    mode === "pty" ? ["pty", ...arguments_] : arguments_;
  const normalized =
    mode !== "pty" &&
    engineArguments[0] &&
    /\.[cm]?tsx?$/.test(engineArguments[0])
      ? ["shot", ...engineArguments]
      : engineArguments;
  const result = spawnSync(
    process.execPath,
    [engineBin(engineMode), ...normalized],
    { stdio: "inherit" },
  );
  if (result.error) {
    console.error(`astroshot could not start ${mode} capture: ${result.error.message}`);
    process.exit(1);
  }
  if (result.signal) process.kill(process.pid, result.signal);
  process.exit(result.status ?? 1);
}

function initHelp() {
  console.log(`astroshot init — generate a fixture template

Usage:
  astroshot init react [fixture.tsx] [--force]
  astroshot init ink [fixture.tsx] [--force]
  astroshot init pty [fixture.yaml] [--force]

Defaults:
  react.shot.tsx
  ink.shot.tsx
  pty.shot.yaml

Existing files are never replaced unless --force is passed.`);
}

function runInit(arguments_) {
  if (
    arguments_.length === 0 ||
    arguments_[0] === "help" ||
    arguments_[0] === "-h" ||
    arguments_[0] === "--help"
  ) {
    initHelp();
    process.exit(arguments_.length === 0 ? 1 : 0);
  }

  const mode = arguments_[0];
  const rest = arguments_.slice(1);
  const force = rest.includes("--force") || rest.includes("-f");
  const unknownFlags = rest.filter(
    (value) => value.startsWith("-") && value !== "--force" && value !== "-f",
  );
  if (unknownFlags.length) {
    throw new Error(`Unknown init flag: ${unknownFlags[0]}`);
  }
  const positionals = rest.filter((value) => !value.startsWith("-"));
  if (positionals.length > 1) {
    throw new Error("init accepts at most one output path");
  }
  const result = writeFixtureTemplate({
    mode,
    outputPath: positionals[0],
    force,
  });
  console.log(`Created ${result.label} fixture: ${result.absolutePath}`);
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

if (command === "demo") {
  try {
    process.exit(runDemo(arguments_));
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    console.error("");
    console.error(demoHelp());
    process.exit(1);
  }
}

if (command === "doctor") {
  try {
    process.exit(runDoctor(arguments_));
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    console.error("");
    console.error(doctorHelp());
    process.exit(1);
  }
}

if (command === "install-browser") {
  runEngine("react", ["install-browser", ...arguments_]);
}

if (command === "init") {
  try {
    runInit(arguments_);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
  process.exit(0);
}

if (command === "react" || command === "ink" || command === "tui" || command === "pty") {
  const canonicalMode = command === "tui" ? "ink" : command;
  if (
    arguments_.length === 0 ||
    arguments_[0] === "help" ||
    arguments_[0] === "-h" ||
    arguments_[0] === "--help"
  ) {
    modeHelp(canonicalMode);
    process.exit(arguments_.length === 0 ? 1 : 0);
  }
  runEngine(canonicalMode, arguments_);
}

if (command === "movie") {
  // Always forward to movie-harness (including --help / which-source).
  const result = spawnSync(
    process.execPath,
    [engineBin("movie"), ...arguments_],
    { stdio: "inherit" },
  );
  if (result.error) {
    console.error(
      `astroshot could not start movie harness: ${result.error.message}`,
    );
    process.exit(1);
  }
  if (result.signal) process.kill(process.pid, result.signal);
  process.exit(result.status ?? 1);
}

console.error(`Unknown command: ${command}`);
help();
process.exit(1);
