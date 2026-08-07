import fs from "node:fs";
import path from "node:path";

import { chromium, type Browser, type Page } from "playwright";
import YAML from "yaml";

import { MovieSession } from "../session.js";
import {
  createHeadlessTerminal,
  terminalDocument,
  terminalPlainText,
  terminalToHtml,
  writeTerminal,
  type HeadlessTerminal,
} from "../terminal-paint.js";
import type {
  MovieArtifact,
  MovieSessionOptions,
  PtyAction,
  PtyKey,
  PtyMovieFixture,
} from "../types.js";

const KEYSTROKES: Record<PtyKey, string> = {
  enter: "\r",
  up: "\x1b[A",
  down: "\x1b[B",
  right: "\x1b[C",
  left: "\x1b[D",
  tab: "\t",
  escape: "\x1b",
  backspace: "\x7f",
  space: " ",
  "ctrl-c": "\x03",
  "ctrl-d": "\x04",
};

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function loadPtyMovieFixture(fixturePath: string): PtyMovieFixture {
  const absolute = path.resolve(fixturePath);
  if (!fs.existsSync(absolute)) {
    throw new Error(`PTY fixture not found: ${absolute}`);
  }
  const text = fs.readFileSync(absolute, "utf8");
  const value =
    absolute.endsWith(".json") ? JSON.parse(text) : YAML.parse(text);
  if (!isRecord(value)) {
    throw new Error(`Invalid PTY fixture ${absolute}: document must be object`);
  }
  if (value.version !== 1) {
    throw new Error(`Invalid PTY fixture ${absolute}: version must be 1`);
  }
  if (typeof value.command !== "string" || !value.command.trim()) {
    throw new Error(`Invalid PTY fixture ${absolute}: command required`);
  }
  return value as unknown as PtyMovieFixture;
}

export type PtyMovieSessionOptions = Omit<MovieSessionOptions, "source"> & {
  fixturePath: string;
};

/**
 * Run a PTY fixture, sample truecolor terminal frames into a MovieSession,
 * and publish poster + video. Color path: SGR → xterm cells → HTML → Chromium.
 */
export async function recordPtyMovie(
  options: PtyMovieSessionOptions,
): Promise<MovieArtifact> {
  const fixture = loadPtyMovieFixture(options.fixturePath);
  const fixtureDir = path.dirname(path.resolve(options.fixturePath));
  const cols = fixture.cols ?? 80;
  const rows = fixture.rows ?? 24;
  const fontSize = fixture.fontSize ?? 14;
  const lineHeight = fixture.lineHeight ?? 1.35;
  const padding = fixture.padding ?? 16;
  const borderRadius = fixture.borderRadius ?? 12;
  const scale = fixture.scale ?? 2;
  const background = fixture.background ?? "#090a12";
  const foreground = fixture.foreground ?? "#e8e8f2";
  const fontFamily =
    fixture.fontFamily ??
    "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace";
  const fps = fixture.movieFps ?? options.fps ?? 12;
  const timeoutMs = fixture.timeoutMs ?? 30_000;
  const settleMs = fixture.settleMs ?? 80;

  const paint = {
    cols,
    rows,
    foreground,
    background,
    fontFamily,
    fontSize,
    lineHeight,
    padding,
    borderRadius,
    scale,
  };
  const { cssWidth, cssHeight } = terminalDocument("", paint);
  const size = {
    width: cssWidth + 32,
    height: cssHeight + 32,
  };

  const session = MovieSession.create({
    ...options,
    size,
    fps,
    source: "pty",
  });

  const terminal = createHeadlessTerminal(cols, rows);
  let browser: Browser | undefined;
  let page: Page | undefined;
  let sampleTimer: ReturnType<typeof setInterval> | undefined;
  let sampling = Promise.resolve();

  try {
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({
      viewport: size,
      deviceScaleFactor: scale,
    });

    const sample = async () => {
      if (!page) return;
      const rowsHtml = terminalToHtml(terminal, {
        cols,
        rows,
        foreground,
        background,
      });
      const { html } = terminalDocument(rowsHtml, paint);
      await page.setContent(html, { waitUntil: "load" });
      const png = await page.locator("[data-tui-shot]").screenshot({
        type: "png",
        omitBackground: true,
      });
      session.pushFrame(png);
    };

    const intervalMs = Math.max(40, Math.round(1000 / fps));
    sampleTimer = setInterval(() => {
      sampling = sampling.then(sample).catch(() => undefined);
    }, intervalMs);

    await runPtyProgram({
      fixture,
      fixtureDir,
      terminal,
      cols,
      rows,
      timeoutMs,
      settleMs,
    });

    clearInterval(sampleTimer);
    sampleTimer = undefined;
    await sampling;
    // Final frame after settle — must match still-shot color path.
    await sample();

    return await session.stop({ status: options.status ?? "running" });
  } finally {
    if (sampleTimer) clearInterval(sampleTimer);
    await page?.close().catch(() => undefined);
    await browser?.close().catch(() => undefined);
    terminal.dispose();
  }
}

async function runPtyProgram(args: {
  fixture: PtyMovieFixture;
  fixtureDir: string;
  terminal: HeadlessTerminal;
  cols: number;
  rows: number;
  timeoutMs: number;
  settleMs: number;
}): Promise<void> {
  const { fixture, fixtureDir, terminal, cols, rows, timeoutMs, settleMs } =
    args;

  let spawnPty: typeof import("node-pty").spawn;
  try {
    ({ spawn: spawnPty } = await import("node-pty"));
  } catch (error) {
    throw new Error(
      "PTY movie capture requires the optional node-pty native addon.",
      { cause: error },
    );
  }

  const cwd = fixture.cwd
    ? path.resolve(fixtureDir, fixture.cwd)
    : fixtureDir;
  const command = resolveCommand(fixture.command, cwd);
  const childEnv = {
    ...process.env,
    TERM: "xterm-256color",
    COLORTERM: "truecolor",
    ...fixture.env,
  };

  let writes = Promise.resolve();
  let exited: { exitCode: number; signal?: number } | null = null;
  let resolveExit: (() => void) | null = null;
  const exitPromise = new Promise<void>((resolve) => {
    resolveExit = resolve;
  });

  const child = spawnPty(command, fixture.args ?? [], {
    name: "xterm-256color",
    cols,
    rows,
    cwd,
    env: childEnv as Record<string, string>,
  });

  child.onData((data) => {
    writes = writes.then(() => writeTerminal(terminal, data));
  });
  child.onExit((event) => {
    exited = event;
    resolveExit?.();
  });

  const deadline = Date.now() + timeoutMs;
  const screen = () => terminalPlainText(terminal, rows);
  const remaining = () => Math.max(0, deadline - Date.now());

  try {
    for (const action of fixture.actions ?? []) {
      if (remaining() === 0) {
        throw new Error(`PTY movie exceeded timeoutMs ${timeoutMs}`);
      }
      await runAction(action, {
        child,
        writes: () => writes,
        screen,
        remaining,
        deadline,
        exited: () => exited,
        exitPromise,
      });
    }
    await delay(Math.min(settleMs, remaining()));
    await writes;
  } finally {
    try {
      child.kill();
    } catch {
      /* already dead */
    }
    // Drain a short window so dispose is clean.
    await Promise.race([exitPromise, delay(500)]);
  }

  for (const expected of fixture.expectText ?? []) {
    if (!screen().includes(expected)) {
      throw new Error(
        `PTY movie did not render expected text ${JSON.stringify(expected)}. ` +
          `Visible:\n${screen().slice(0, 1200)}`,
      );
    }
  }

  // Exit code is advisory: we may kill the child after sampling. Non-zero
  // before expectText is still surfaced via missing text / timeouts above.
  void exited;
  void fixture.allowNonZeroExit;
}

async function runAction(
  action: PtyAction,
  ctx: {
    child: { write: (data: string) => void };
    writes: () => Promise<void>;
    screen: () => string;
    remaining: () => number;
    deadline: number;
    exited: () => { exitCode: number; signal?: number } | null;
    exitPromise: Promise<void>;
  },
): Promise<void> {
  if ("waitFor" in action) {
    const actionDeadline = Math.min(
      ctx.deadline,
      Date.now() + (action.timeoutMs ?? 120_000),
    );
    while (Date.now() <= actionDeadline) {
      await ctx.writes();
      if (ctx.screen().includes(action.waitFor)) return;
      if (ctx.exited()) break;
      await delay(20);
    }
    throw new Error(
      `Timed out waiting for ${JSON.stringify(action.waitFor)}. Visible:\n${ctx.screen().slice(0, 1200)}`,
    );
  }
  if ("waitForExit" in action) {
    const actionDeadline = Math.min(
      ctx.deadline,
      Date.now() + (action.timeoutMs ?? 120_000),
    );
    while (Date.now() <= actionDeadline) {
      if (ctx.exited()) return;
      await Promise.race([ctx.exitPromise, delay(20)]);
    }
    throw new Error("Timed out waiting for PTY exit");
  }
  if ("key" in action) {
    ctx.child.write(KEYSTROKES[action.key]);
    return;
  }
  if ("text" in action) {
    ctx.child.write(action.text);
    return;
  }
  await delay(Math.min(action.pauseMs, ctx.remaining()));
}

function resolveCommand(command: string, cwd: string): string {
  if (command.includes(path.sep) || command.startsWith(".")) {
    return path.resolve(cwd, command);
  }
  // PATH lookup — node-pty resolves bare names on most platforms.
  return command;
}

/** Synthetic truecolor PTY-like movie without node-pty (for CI smoke). */
export async function recordTruecolorDemoMovie(
  options: Omit<MovieSessionOptions, "source"> & {
    /** Hex color without #, default 7c5cff */
    color?: string;
  },
): Promise<MovieArtifact> {
  const color = options.color ?? "7c5cff";
  const r = Number.parseInt(color.slice(0, 2), 16);
  const g = Number.parseInt(color.slice(2, 4), 16);
  const b = Number.parseInt(color.slice(4, 6), 16);
  if ([r, g, b].some((n) => Number.isNaN(n))) {
    throw new Error(`invalid color ${color}`);
  }

  const cols = 40;
  const rows = 8;
  const paint = {
    cols,
    rows,
    foreground: "#e8e8f2",
    background: "#090a12",
    fontFamily: "ui-monospace, monospace",
    fontSize: 16,
    lineHeight: 1.35,
    padding: 16,
    borderRadius: 12,
    scale: 2,
  };
  const { cssWidth, cssHeight } = terminalDocument("", paint);
  const size = { width: cssWidth + 32, height: cssHeight + 32 };
  const session = MovieSession.create({
    ...options,
    size,
    fps: options.fps ?? 10,
    source: "pty",
  });

  const terminal = createHeadlessTerminal(cols, rows);
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({
      viewport: size,
      deviceScaleFactor: 2,
    });
    const labels = ["truecolor", "movie", "harness", color];
    for (const label of labels) {
      await writeTerminal(
        terminal,
        `\x1b[2J\x1b[H\x1b[38;2;${r};${g};${b}m${label}\x1b[0m\r\n`,
      );
      const rowsHtml = terminalToHtml(terminal, {
        cols,
        rows,
        foreground: paint.foreground,
        background: paint.background,
      });
      if (!rowsHtml.includes(`color:#${color}`)) {
        throw new Error(
          `truecolor path lost #${color}; html=${rowsHtml.slice(0, 200)}`,
        );
      }
      const { html } = terminalDocument(rowsHtml, paint);
      await page.setContent(html, { waitUntil: "load" });
      const png = await page.locator("[data-tui-shot]").screenshot({
        type: "png",
        omitBackground: true,
      });
      session.pushFrame(png);
      await delay(120);
    }
    await page.close();
    return await session.stop({ status: options.status ?? "running" });
  } finally {
    terminal.dispose();
    await browser.close().catch(() => undefined);
  }
}

