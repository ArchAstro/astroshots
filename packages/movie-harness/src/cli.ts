import fs from "node:fs";
import path from "node:path";

import { encodeSolidPng } from "./png.js";
import { resolveRoot } from "./paths.js";
import { finalizeManifest } from "./sink.js";
import {
  formatSourceCatalog,
  recommendSource,
  SOURCE_DECISION_TABLE,
  sourceHintForError,
} from "./source-help.js";
import { recordBrowserMovie } from "./sources/browser.js";
import {
  assertDesktopToolchain,
  checkScreenRecordingAccess,
  desktopMatchFromFlags,
  listDesktopWindows,
  openScreenRecordingSettings,
  recordDesktopWindowMovie,
} from "./sources/desktop-macos.js";
import {
  loadFrameSession,
  markFrameSession,
  pushFrameToSession,
  startFrameSession,
  stopFrameSession,
} from "./sources/frames-store.js";
import {
  recordPtyMovie,
  recordTruecolorDemoMovie,
} from "./sources/pty.js";
import type { MovieFormat, Size } from "./types.js";

function usage(): string {
  return `astroshot movie — universal movie harness → .astroshot/ poster + video

${SOURCE_DECISION_TABLE}

Commands
--------
  which-source [intent…]     Recommend a --source for an agent/human intent
  list-windows               List macOS windows (id, bundle, title) for desktop.window
  check-screen-access        Detect Screen Recording TCC (optional --request prompt)
  open-screen-settings       Open System Settings → Screen Recording
  start / push-frame / mark / stop
                             Multi-process frames session
  run                        One-shot recorder (browser | pty | desktop.window | frames | …)
  finalize                   Set manifest status pass|fail|idle

run --source …
--------------
  browser         --url URL | --script path.mjs [--headed] [--settle-ms N]
  pty             --fixture path.yaml|json   # truecolor TUI — preferred for terminals
  pty-demo        [--color 7c5cff]           # truecolor smoke, no program
  desktop.window  --bundle-id ID | --window-id N | --title-regex RE | --owner NAME | --pid N
                  [--duration-ms 3000] [--fps 10] [--cursor] [--allow-blank]
  frames          [--demo-frames N]          # or use start/push-frame/stop

Common options
--------------
  --feature NAME   kebab-case feature dir under .astroshot/ (required for capture)
  --slug SLUG      kebab-case movie slug (required for capture)
  --root DIR       worktree root (default: git root or cwd)
  --run-id ID      stable run identity (share across movies in one journey)
  --title TEXT     human title
  --description T  what the movie proves
  --size WxH       encode size (desktop defaults to window pixels)
  --fps N          default 15 (desktop default 10)
  --format webm|mp4
  --status running|pass|fail|idle
  --session ID     frames session id (default: latest)
  -h, --help

Agent notes
-----------
  • Prefer the decision table over improvising OS screen recording for web/TUI.
  • Always land poster+video under .astroshot/ so Astroshots streams posters.
  • desktop.window uses macOS screencapture (already on the system) + Swift window list
    shipped in this package — no extra binary download. Needs Screen Recording TCC.
  • For full catalog: astroshot movie help-sources

Examples
--------
  astroshot movie which-source "record a ratatui TUI with truecolor"
  astroshot movie run --source browser --feature web --slug home --url https://example.com
  astroshot movie run --source pty --feature tui --slug flow --fixture ./flow.pty.yaml
  astroshot movie run --source desktop.window --feature app --slug onboard \\
    --bundle-id com.example.App --duration-ms 4000
  astroshot movie list-windows
`;
}

type Args = Record<string, string | boolean>;

function parseArgs(argv: string[]): { command: string; args: Args; rest: string[] } {
  if (argv.length === 0 || argv[0] === "-h" || argv[0] === "--help") {
    return { command: "help", args: {}, rest: [] };
  }
  const command = argv[0]!;
  const args: Args = {};
  const rest: string[] = [];
  for (let i = 1; i < argv.length; i++) {
    const token = argv[i]!;
    if (token === "-h" || token === "--help") {
      args.help = true;
      continue;
    }
    if (!token.startsWith("--")) {
      rest.push(token);
      continue;
    }
    const key = token.slice(2);
    const next = argv[i + 1];
    if (next === undefined || next.startsWith("--")) {
      args[key] = true;
    } else {
      args[key] = next;
      i += 1;
    }
  }
  return { command, args, rest };
}

function req(args: Args, key: string): string {
  const value = args[key];
  if (typeof value !== "string" || !value) {
    throw new Error(`--${key} is required`);
  }
  return value;
}

function opt(args: Args, key: string): string | undefined {
  const value = args[key];
  return typeof value === "string" ? value : undefined;
}

function parseSize(value: string | undefined): Size | undefined {
  if (!value) return undefined;
  const match = /^(\d+)x(\d+)$/.exec(value);
  if (!match) throw new Error(`--size must be WxH, got ${value}`);
  return { width: Number(match[1]), height: Number(match[2]) };
}

function parseFormat(value: string | undefined): MovieFormat | undefined {
  if (!value) return undefined;
  if (value !== "webm" && value !== "mp4") {
    throw new Error(`--format must be webm|mp4`);
  }
  return value;
}

function parseFps(value: string | undefined): number | undefined {
  if (!value) return undefined;
  const n = Number(value);
  if (!(n > 0 && n <= 60)) throw new Error(`--fps must be in (0, 60]`);
  return n;
}

export async function runCli(argv: string[]): Promise<number> {
  try {
    const { command, args, rest } = parseArgs(argv);
    if (command === "help" || args.help) {
      process.stdout.write(`${usage()}\n`);
      return 0;
    }

    switch (command) {
      case "which-source":
        return cmdWhichSource(rest, args);
      case "help-sources":
        process.stdout.write(`${formatSourceCatalog()}\n`);
        return 0;
      case "list-windows":
        return cmdListWindows();
      case "check-screen-access":
        return cmdCheckScreenAccess(args);
      case "open-screen-settings":
        return cmdOpenScreenSettings();
      case "start":
        return cmdStart(args);
      case "push-frame":
        return cmdPushFrame(args);
      case "mark":
        return cmdMark(args);
      case "stop":
        return await cmdStop(args);
      case "run":
        return await cmdRun(args);
      case "finalize":
        return cmdFinalize(args);
      default:
        throw new Error(`unknown command: ${command}\n\n${usage()}`);
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`error: ${message}\n`);
    if (!/which-source|decision table|list-windows/i.test(message)) {
      process.stderr.write(`hint: ${sourceHintForError()}\n`);
    }
    return 1;
  }
}

function cmdWhichSource(rest: string[], args: Args): number {
  const intent =
    rest.join(" ") ||
    opt(args, "intent") ||
    opt(args, "for") ||
    "";
  if (!intent) {
    process.stdout.write(
      `${SOURCE_DECISION_TABLE}\n\n` +
        `Pass free text, e.g.:\n` +
        `  astroshot movie which-source "native SwiftUI onboarding window"\n` +
        `  astroshot movie which-source "ratatui truecolor dashboard"\n`,
    );
    return 0;
  }
  const advice = recommendSource(intent);
  process.stdout.write(
    `${JSON.stringify({ intent, recommended: advice.source, reason: advice.reason }, null, 2)}\n`,
  );
  return 0;
}

function cmdListWindows(): number {
  assertDesktopToolchain();
  const windows = listDesktopWindows();
  process.stdout.write(`${JSON.stringify(windows, null, 2)}\n`);
  process.stderr.write(
    `# ${windows.length} windows — match with --window-id / --bundle-id / --title-regex / --owner / --pid\n`,
  );
  return 0;
}

function cmdCheckScreenAccess(args: Args): number {
  assertDesktopToolchain();
  const request = Boolean(args.request);
  const report = checkScreenRecordingAccess({ request });
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  if (!report.granted) {
    process.stderr.write(
      `\nScreen Recording: DENIED (enable "${report.enableApp}")\n` +
        `  → ${report.settingsHint}\n` +
        `  → or: astroshot movie open-screen-settings\n` +
        `  → then quit & reopen ${report.enableApp}, re-run check-screen-access\n`,
    );
    if (args["open-settings"] || args.open) {
      openScreenRecordingSettings();
    }
    return 2;
  }
  process.stderr.write(
    `# Screen Recording preflight: OK — still enable "${report.enableApp}" in Settings if captures fail\n`,
  );
  return 0;
}

function cmdOpenScreenSettings(): number {
  assertDesktopToolchain();
  const ok = openScreenRecordingSettings();
  if (!ok) {
    process.stderr.write(
      "error: could not open System Settings; open Privacy & Security → Screen Recording manually\n",
    );
    return 1;
  }
  process.stderr.write(
    "Opened System Settings (Screen Recording). Enable the app that launched this command, quit it fully, reopen, then:\n" +
      "  astroshot movie check-screen-access\n",
  );
  return 0;
}

function cmdStart(args: Args): number {
  const state = startFrameSession({
    feature: req(args, "feature"),
    slug: req(args, "slug"),
    root: opt(args, "root"),
    runId: opt(args, "run-id"),
    title: opt(args, "title"),
    description: opt(args, "description"),
    size: parseSize(opt(args, "size")),
    fps: parseFps(opt(args, "fps")),
    format: parseFormat(opt(args, "format")),
  });
  process.stdout.write(
    `${JSON.stringify(
      {
        id: state.id,
        feature: state.feature,
        slug: state.slug,
        runId: state.runId,
        frameDir: state.frameDir,
      },
      null,
      2,
    )}\n`,
  );
  return 0;
}

function cmdPushFrame(args: Args): number {
  const state = loadFrameSession(
    opt(args, "root") ?? resolveRoot(),
    req(args, "feature"),
    opt(args, "session"),
  );
  const file = req(args, "file");
  const next = pushFrameToSession(state, path.resolve(file));
  process.stdout.write(
    `${JSON.stringify({ id: next.id, frameCount: next.frameCount }, null, 2)}\n`,
  );
  return 0;
}

function cmdMark(args: Args): number {
  const state = loadFrameSession(
    opt(args, "root") ?? resolveRoot(),
    req(args, "feature"),
    opt(args, "session"),
  );
  const next = markFrameSession(state, req(args, "slug"), opt(args, "note"));
  process.stdout.write(
    `${JSON.stringify({ id: next.id, chapters: next.chapters }, null, 2)}\n`,
  );
  return 0;
}

async function cmdStop(args: Args): Promise<number> {
  const state = loadFrameSession(
    opt(args, "root") ?? resolveRoot(),
    req(args, "feature"),
    opt(args, "session"),
  );
  const status = (opt(args, "status") ?? "running") as
    | "running"
    | "pass"
    | "fail"
    | "idle";
  const artifact = await stopFrameSession(state, { status });
  process.stdout.write(`${JSON.stringify(artifact, null, 2)}\n`);
  return 0;
}

async function cmdRun(args: Args): Promise<number> {
  const source = req(args, "source");
  const feature = req(args, "feature");
  const slug = req(args, "slug");
  const common = {
    feature,
    slug,
    root: opt(args, "root"),
    runId: opt(args, "run-id"),
    title: opt(args, "title"),
    description: opt(args, "description"),
    size: parseSize(opt(args, "size")),
    fps: parseFps(opt(args, "fps")),
    format: parseFormat(opt(args, "format")),
    status: (opt(args, "status") ?? "running") as
      | "running"
      | "pass"
      | "fail"
      | "idle",
  };

  if (source === "frames") {
    const count = Number(opt(args, "demo-frames") ?? "8");
    const size = common.size ?? { width: 320, height: 180 };
    let state = startFrameSession({ ...common, size });
    const tmpDir = path.join(state.frameDir, "..", "demo-src");
    fs.mkdirSync(tmpDir, { recursive: true });
    const colors: Array<[number, number, number]> = [
      [124, 92, 255],
      [90, 200, 250],
      [80, 220, 160],
      [240, 180, 110],
      [240, 114, 122],
      [232, 232, 242],
      [90, 90, 122],
      [9, 10, 18],
    ];
    for (let i = 0; i < count; i++) {
      const rgb = colors[i % colors.length]!;
      const png = encodeSolidPng(size.width, size.height, rgb);
      const file = path.join(tmpDir, `f-${i}.png`);
      fs.writeFileSync(file, png);
      state = pushFrameToSession(state, file);
      if (i === Math.floor(count / 2)) {
        state = markFrameSession(state, "midpoint");
      }
    }
    const artifact = await stopFrameSession(state, {
      status: common.status,
    });
    process.stdout.write(`${JSON.stringify(artifact, null, 2)}\n`);
    return 0;
  }

  if (source === "browser") {
    const artifact = await recordBrowserMovie({
      ...common,
      url: opt(args, "url"),
      scriptPath: opt(args, "script"),
      headed: Boolean(args.headed),
      settleMs: opt(args, "settle-ms")
        ? Number(opt(args, "settle-ms"))
        : undefined,
    });
    process.stdout.write(`${JSON.stringify(artifact, null, 2)}\n`);
    return 0;
  }

  if (source === "pty") {
    const artifact = await recordPtyMovie({
      ...common,
      fixturePath: req(args, "fixture"),
    });
    process.stdout.write(`${JSON.stringify(artifact, null, 2)}\n`);
    return 0;
  }

  if (source === "pty-demo") {
    const artifact = await recordTruecolorDemoMovie({
      ...common,
      color: opt(args, "color"),
    });
    process.stdout.write(`${JSON.stringify(artifact, null, 2)}\n`);
    return 0;
  }

  if (source === "desktop.window") {
    assertDesktopToolchain();
    const match = desktopMatchFromFlags({
      "window-id": opt(args, "window-id"),
      "bundle-id": opt(args, "bundle-id"),
      "title-regex": opt(args, "title-regex"),
      owner: opt(args, "owner"),
      pid: opt(args, "pid"),
      pick: opt(args, "pick"),
    });
    const durationMs = opt(args, "duration-ms")
      ? Number(opt(args, "duration-ms"))
      : 3_000;
    const artifact = await recordDesktopWindowMovie({
      ...common,
      match,
      durationMs,
      cursor: Boolean(args.cursor),
      allowBlank: Boolean(args["allow-blank"]),
      fps: common.fps ?? 10,
    });
    process.stdout.write(`${JSON.stringify(artifact, null, 2)}\n`);
    return 0;
  }

  if (source === "desktop.display" || source === "desktop.region") {
    throw new Error(
      `${source} is not implemented yet.\n` +
        `Use --source desktop.window for a single app, or --source frames.\n` +
        sourceHintForError("desktop.window"),
    );
  }

  throw new Error(
    `unknown --source ${JSON.stringify(source)}\n${SOURCE_DECISION_TABLE}`,
  );
}

function cmdFinalize(args: Args): number {
  const root = resolveRoot(opt(args, "root"));
  const feature = req(args, "feature");
  const runId = req(args, "run-id");
  const status = req(args, "status");
  if (!["pass", "fail", "idle", "running"].includes(status)) {
    throw new Error("--status must be pass|fail|idle|running");
  }
  finalizeManifest(
    root,
    feature,
    runId,
    status as "pass" | "fail" | "idle" | "running",
  );
  process.stdout.write(
    `${JSON.stringify({ feature, runId, status }, null, 2)}\n`,
  );
  return 0;
}
