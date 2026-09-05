import fs from "node:fs";
import path from "node:path";

import { render } from "ink";
import { createElement } from "react";

import { ReviewStore } from "./data/store.js";
import { ImageService } from "./images/service.js";
import { createGraphicsStdout } from "./terminal/graphics-stdout.js";
import { ImageLayer } from "./terminal/image-layer.js";
import { probeTerminal } from "./terminal/probe.js";
import { App } from "./ui/app.js";
import { ServicesContext } from "./ui/context.js";
import { detectFfmpeg } from "./video/ffmpeg.js";

export function reviewHelp(): string {
  return `astroshot review — the Astroshots tray in your terminal

Usage:
  astroshot review [<dir>...] [options]

Options:
  --root <dir>       Folder to watch for .astroshot/ trees (repeatable)
  --no-graphics      Skip terminal pictures (text only)
  --no-watch         Do not follow filesystem changes
  --no-index         Ignore the on-disk index (full scan every start)
  -h, --help         Show this help

Without roots, \`astroshot review\` uses the folders the Astroshots app
watches (macOS) and otherwise the current directory. Pictures need a
terminal with the Kitty graphics protocol (Ghostty, kitty, WezTerm);
movies additionally need ffmpeg on PATH.

Keys: ↑↓ move · ⏎ open · f full screen · s seen · c feedback · u history ·
      m movies · 1/2 tabs · , settings · ? help · q quit`;
}

export interface ParsedArgs {
  roots: string[];
  graphics: boolean;
  watch: boolean;
  index: boolean;
  help: boolean;
  version: boolean;
  rootsSource: "app" | "cli" | "cwd";
}

export function parseArgs(argv: string[]): ParsedArgs {
  const parsed: ParsedArgs = { roots: [], graphics: true, watch: true, index: true, help: false, version: false, rootsSource: "cli" };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]!;
    switch (argument) {
      case "-h":
      case "--help":
        parsed.help = true;
        break;
      case "-v":
      case "--version":
        parsed.version = true;
        break;
      case "--no-graphics":
        parsed.graphics = false;
        break;
      case "--no-watch":
        parsed.watch = false;
        break;
      case "--no-index":
        parsed.index = false;
        break;
      case "--root": {
        const value = argv[index + 1];
        if (!value) throw new Error("--root requires a directory");
        parsed.roots.push(value);
        index += 1;
        break;
      }
      case "--roots-source": {
        const value = argv[index + 1];
        if (value !== "app" && value !== "cli" && value !== "cwd") throw new Error("--roots-source must be app, cli, or cwd");
        parsed.rootsSource = value;
        index += 1;
        break;
      }
      default:
        if (argument.startsWith("-")) throw new Error(`Unknown option: ${argument}`);
        parsed.roots.push(argument);
    }
  }
  if (parsed.roots.length === 0) {
    parsed.roots = [process.cwd()];
    parsed.rootsSource = "cwd";
  }
  return parsed;
}

function readVersion(): string {
  try {
    const packageJson = JSON.parse(fs.readFileSync(new URL("../package.json", import.meta.url), "utf8")) as { version?: string };
    return packageJson.version ?? "0.0.0";
  } catch {
    return "0.0.0";
  }
}

export async function main(argv: string[]): Promise<number> {
  let args: ParsedArgs;
  try {
    args = parseArgs(argv);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    console.error("");
    console.error(reviewHelp());
    return 1;
  }
  if (args.help) {
    console.log(reviewHelp());
    return 0;
  }
  if (args.version) {
    console.log(readVersion());
    return 0;
  }
  if (!process.stdout.isTTY || !process.stdin.isTTY) {
    console.error("astroshot review needs an interactive terminal (stdin and stdout must be a TTY).");
    return 1;
  }
  const roots = args.roots.map((root) => path.resolve(root)).filter((root) => {
    try {
      return fs.statSync(root).isDirectory();
    } catch {
      console.error(`Ignoring missing folder: ${root}`);
      return false;
    }
  });

  const debugLog = process.env.ASTROSHOT_REVIEW_DEBUG
    ? (message: string) => fs.appendFileSync("astroshot-review.log", `${new Date().toISOString()} ${message}\n`)
    : null;
  const capabilities = await probeTerminal(
    { stdin: process.stdin, stdout: process.stdout },
    { env: args.graphics ? process.env : { ...process.env, ASTROSHOT_REVIEW_GRAPHICS: "none" } },
  );
  debugLog?.(`capabilities ${JSON.stringify(capabilities)}`);
  const ffmpeg = detectFfmpeg();
  const service = new ImageService();
  const layer = new ImageLayer({
    capabilities,
    service,
    write: (data) => {
      process.stdout.write(data);
    },
    onError: (src, error) => debugLog?.(`image ${src}: ${error.message}`),
    onDebug: debugLog ? (message) => debugLog(`layer ${message}`) : undefined,
  });
  const store = new ReviewStore({
    roots,
    watch: args.watch,
    useIndex: args.index,
    onLog: (message) => debugLog?.(`store ${message}`),
  });
  void store.start();

  const stdout = createGraphicsStdout(process.stdout, layer);
  const services = { store, layer, capabilities, ffmpeg, rootsSource: args.rootsSource, version: readVersion() };
  let cleared = false;
  const clearPictures = () => {
    if (cleared) return;
    cleared = true;
    const output = layer.clear();
    if (output) process.stdout.write(output);
  };
  const instance = render(createElement(ServicesContext.Provider, { value: services }, createElement(App, { onQuit: clearPictures })), {
    stdout,
    stdin: process.stdin,
    alternateScreen: true,
    exitOnCtrlC: true,
    patchConsole: true,
  });
  // `kill <pid>` or a closing terminal must not leave pictures or the
  // alternate screen behind; Ink only handles Ctrl+C itself.
  const onSignal = (signal: NodeJS.Signals) => {
    clearPictures();
    try {
      process.stdout.write("\x1b[?1049l\x1b[?25h");
      if (process.stdin.isTTY) process.stdin.setRawMode(false);
    } catch {
      // The terminal may already be gone.
    }
    instance.unmount();
    process.exit(signal === "SIGHUP" ? 129 : 143);
  };
  process.once("SIGTERM", onSignal);
  process.once("SIGHUP", onSignal);
  try {
    await instance.waitUntilExit();
  } finally {
    process.off("SIGTERM", onSignal);
    process.off("SIGHUP", onSignal);
    clearPictures();
    await store.dispose();
    await service.dispose();
  }
  return 0;
}
