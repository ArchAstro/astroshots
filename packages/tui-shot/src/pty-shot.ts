import fs from "node:fs";
import path from "node:path";

import type { IPty } from "node-pty";
import YAML from "yaml";

import {
  captureTerminalHtml,
  queueTerminalShot,
  validPositive,
} from "./shot.js";
import {
  createHeadlessTerminal,
  terminalPlainText,
  terminalToHtml,
  writeTerminal,
} from "./terminal-html.js";
import type {
  PtyAction,
  PtyKey,
  PtyShotFixture,
  PtyShotRequest,
} from "./types.js";

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

function fixtureError(fixturePath: string, detail: string): Error {
  return new Error(`Invalid PTY fixture ${fixturePath}: ${detail}`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function validateAction(
  value: unknown,
  fixturePath: string,
  index: number,
): PtyAction {
  if (!isRecord(value)) {
    throw fixtureError(fixturePath, `actions[${index}] must be an object`);
  }
  const operators = ["waitFor", "key", "text", "pauseMs"].filter(
    (name) => value[name] !== undefined,
  );
  if (operators.length !== 1) {
    throw fixtureError(
      fixturePath,
      `actions[${index}] must set exactly one of waitFor, key, text, or pauseMs`,
    );
  }
  if (operators[0] === "waitFor") {
    if (typeof value.waitFor !== "string" || !value.waitFor) {
      throw fixtureError(fixturePath, `actions[${index}].waitFor must be text`);
    }
    if (
      value.timeoutMs !== undefined &&
      (typeof value.timeoutMs !== "number" ||
        !Number.isFinite(value.timeoutMs) ||
        value.timeoutMs <= 0)
    ) {
      throw fixtureError(
        fixturePath,
        `actions[${index}].timeoutMs must be a positive number`,
      );
    }
    return { waitFor: value.waitFor, timeoutMs: value.timeoutMs as number };
  }
  if (operators[0] === "key") {
    if (
      typeof value.key !== "string" ||
      !(value.key.toLowerCase() in KEYSTROKES)
    ) {
      throw fixtureError(
        fixturePath,
        `actions[${index}].key must be one of ${Object.keys(KEYSTROKES).join(", ")}`,
      );
    }
    return { key: value.key.toLowerCase() as PtyKey };
  }
  if (operators[0] === "text") {
    if (typeof value.text !== "string") {
      throw fixtureError(fixturePath, `actions[${index}].text must be a string`);
    }
    return { text: value.text };
  }
  if (
    typeof value.pauseMs !== "number" ||
    !Number.isFinite(value.pauseMs) ||
    value.pauseMs < 0
  ) {
    throw fixtureError(
      fixturePath,
      `actions[${index}].pauseMs must be a non-negative number`,
    );
  }
  return { pauseMs: value.pauseMs };
}

function stringArray(
  value: unknown,
  fixturePath: string,
  field: string,
): string[] | undefined {
  if (value === undefined) return undefined;
  if (
    !Array.isArray(value) ||
    value.some((entry) => typeof entry !== "string")
  ) {
    throw fixtureError(fixturePath, `${field} must be an array of strings`);
  }
  return value;
}

function loadPtyFixture(fixturePath: string): PtyShotFixture {
  const absolute = path.resolve(fixturePath);
  if (!fs.existsSync(absolute)) {
    throw new Error(`Fixture not found: ${absolute}`);
  }
  const raw = fs.readFileSync(absolute, "utf8");
  let value: unknown;
  try {
    value = path.extname(absolute).toLowerCase() === ".json"
      ? JSON.parse(raw)
      : YAML.parse(raw);
  } catch (error) {
    throw fixtureError(
      absolute,
      error instanceof Error ? error.message : String(error),
    );
  }
  if (!isRecord(value)) {
    throw fixtureError(absolute, "the document must be an object");
  }
  if (value.version !== 1) {
    throw fixtureError(absolute, "version must be 1");
  }
  if (typeof value.command !== "string" || !value.command.trim()) {
    throw fixtureError(absolute, "command must be a non-empty string");
  }
  const args = stringArray(value.args, absolute, "args");
  const expectText = stringArray(value.expectText, absolute, "expectText");
  if (value.cwd !== undefined && typeof value.cwd !== "string") {
    throw fixtureError(absolute, "cwd must be a string");
  }
  if (
    value.allowNonZeroExit !== undefined &&
    typeof value.allowNonZeroExit !== "boolean"
  ) {
    throw fixtureError(absolute, "allowNonZeroExit must be a boolean");
  }
  for (const field of ["background", "foreground", "fontFamily"] as const) {
    if (value[field] !== undefined && typeof value[field] !== "string") {
      throw fixtureError(absolute, `${field} must be a string`);
    }
  }
  let env: Record<string, string> | undefined;
  if (value.env !== undefined) {
    if (
      !isRecord(value.env) ||
      Object.values(value.env).some((entry) => typeof entry !== "string")
    ) {
      throw fixtureError(absolute, "env values must be strings");
    }
    env = value.env as Record<string, string>;
  }
  if (value.actions !== undefined && !Array.isArray(value.actions)) {
    throw fixtureError(absolute, "actions must be an array");
  }
  const actions = (value.actions as unknown[] | undefined)?.map(
    (action, index) => validateAction(action, absolute, index),
  );
  return {
    ...(value as unknown as PtyShotFixture),
    command: value.command,
    args,
    cwd: value.cwd as string | undefined,
    env,
    actions,
    expectText,
  };
}

function milliseconds(
  value: number | undefined,
  fallback: number,
  name: string,
  maximum: number,
): number {
  const resolved = value ?? fallback;
  if (
    !Number.isFinite(resolved) ||
    resolved < 0 ||
    resolved > maximum
  ) {
    throw new Error(`${name} must be between 0 and ${maximum}`);
  }
  return resolved;
}

function delay(durationMs: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, durationMs));
}

function resolvePtyCommand(
  command: string,
  cwd: string,
  env: NodeJS.ProcessEnv,
): string {
  if (path.isAbsolute(command)) return command;
  if (command.includes("/") || command.includes("\\")) {
    return path.resolve(cwd, command);
  }
  if (process.platform !== "win32") return command;

  const pathValue =
    Object.entries(env).find(([name]) => name.toLowerCase() === "path")?.[1] ??
    "";
  const commandExtension = path.extname(command);
  const extensions = commandExtension
    ? [""]
    : (
        Object.entries(env).find(
          ([name]) => name.toLowerCase() === "pathext",
        )?.[1] ?? ".COM;.EXE"
      )
        .split(";")
        .filter(Boolean);
  for (const directory of pathValue.split(path.delimiter)) {
    const unquotedDirectory = directory.replace(/^"(.*)"$/, "$1");
    if (!unquotedDirectory) continue;
    for (const extension of extensions) {
      const candidate = path.join(unquotedDirectory, command + extension);
      try {
        if (fs.statSync(candidate).isFile()) return candidate;
      } catch {
        // Continue through PATH candidates.
      }
    }
  }
  throw new Error(`PTY command was not found on PATH: ${command}`);
}

async function takeIsolatedPtyShot(request: PtyShotRequest): Promise<string> {
  const absoluteFixturePath = path.resolve(request.fixturePath);
  const fixture = loadPtyFixture(absoluteFixturePath);
  const cols = validPositive(request.cols ?? fixture.cols ?? 100, "cols", {
    integer: true,
    maximum: 1_000,
  });
  const rows = validPositive(request.rows ?? fixture.rows ?? 30, "rows", {
    integer: true,
    maximum: 1_000,
  });
  const scale = validPositive(request.scale ?? fixture.scale ?? 2, "scale", {
    maximum: 4,
  });
  const timeoutMs = milliseconds(
    fixture.timeoutMs,
    15_000,
    "timeoutMs",
    120_000,
  );
  const settleMs = milliseconds(fixture.settleMs, 100, "settleMs", 10_000);
  const background = fixture.background ?? "#090a12";
  const foreground = fixture.foreground ?? "#e8e8f2";
  const fontFamily =
    fixture.fontFamily ??
    '"SFMono-Regular", "Cascadia Code", "Roboto Mono", Menlo, Consolas, monospace';
  const fontSize = validPositive(fixture.fontSize ?? 15, "fontSize", {
    maximum: 200,
  });
  const lineHeight = validPositive(fixture.lineHeight ?? 1.32, "lineHeight", {
    maximum: 10,
  });
  const padding = milliseconds(fixture.padding, 22, "padding", 1_000);
  const borderRadius = milliseconds(
    fixture.borderRadius,
    12,
    "borderRadius",
    1_000,
  );
  const fixtureDirectory = path.dirname(absoluteFixturePath);
  const cwd = path.resolve(fixtureDirectory, fixture.cwd ?? ".");
  if (!fs.existsSync(cwd) || !fs.statSync(cwd).isDirectory()) {
    throw new Error(`PTY fixture cwd is not a directory: ${cwd}`);
  }

  const terminal = createHeadlessTerminal(cols, rows);
  const childEnvironment: NodeJS.ProcessEnv = {
    ...process.env,
    TERM: "xterm-256color",
    COLORTERM: "truecolor",
    ...fixture.env,
  };
  const command = resolvePtyCommand(fixture.command, cwd, childEnvironment);
  let writes = Promise.resolve();
  let child: IPty | null = null;
  let exited: { exitCode: number; signal?: number } | null = null;
  let resolveExit: (() => void) | null = null;
  const exitPromise = new Promise<void>((resolve) => {
    resolveExit = resolve;
  });
  const deadline = Date.now() + timeoutMs;
  const screen = () => terminalPlainText(terminal, rows);
  const remaining = () => Math.max(0, deadline - Date.now());
  const exitState = () => exited;

  async function waitForText(text: string, actionTimeout?: number): Promise<void> {
    const actionDeadline = Math.min(
      deadline,
      Date.now() +
        milliseconds(actionTimeout, timeoutMs, "action timeoutMs", 120_000),
    );
    while (Date.now() <= actionDeadline) {
      await writes;
      if (screen().includes(text)) return;
      if (exited) break;
      await delay(Math.min(20, Math.max(1, actionDeadline - Date.now())));
    }
    const exitDetail = exited
      ? ` The program exited with code ${exited.exitCode}.`
      : "";
    throw new Error(
      `Timed out waiting for ${JSON.stringify(text)}.${exitDetail} Visible frame:\n${screen().slice(0, 1_200)}`,
    );
  }

  try {
    let spawnPty: typeof import("node-pty").spawn;
    try {
      ({ spawn: spawnPty } = await import("node-pty"));
    } catch (error) {
      throw new Error(
        "PTY capture is unavailable because the optional node-pty native addon could not load on this platform. Reinstall @archastro/astroshot in a supported Node.js environment.",
        { cause: error },
      );
    }
    child = spawnPty(command, fixture.args ?? [], {
      name: "xterm-256color",
      cols,
      rows,
      cwd,
      env: childEnvironment,
    });
    child.onData((data) => {
      writes = writes.then(() => writeTerminal(terminal, data));
    });
    child.onExit((event) => {
      exited = event;
      resolveExit?.();
    });

    for (const action of fixture.actions ?? []) {
      if (remaining() === 0) {
        throw new Error(`PTY fixture exceeded timeoutMs ${timeoutMs}`);
      }
      if ("waitFor" in action) {
        await waitForText(action.waitFor, action.timeoutMs);
      } else if ("key" in action) {
        child.write(KEYSTROKES[action.key]);
      } else if ("text" in action) {
        child.write(action.text);
      } else {
        const pauseMs = Math.min(action.pauseMs, remaining());
        await delay(pauseMs);
        if (pauseMs < action.pauseMs) {
          throw new Error(`PTY fixture exceeded timeoutMs ${timeoutMs}`);
        }
      }
    }
    if (settleMs > remaining()) {
      throw new Error(`PTY fixture exceeded timeoutMs ${timeoutMs}`);
    }
    await delay(settleMs);
    await writes;
    const plainFrame = screen();
    const completed = exitState();
    if (completed && completed.exitCode !== 0 && !fixture.allowNonZeroExit) {
      throw new Error(
        `PTY program exited with code ${completed.exitCode} before capture. ` +
          "Set allowNonZeroExit: true only when documenting an intentional failure state. " +
          `Visible frame:\n${plainFrame.slice(0, 1_200)}`,
      );
    }
    for (const expected of fixture.expectText ?? []) {
      if (!plainFrame.includes(expected)) {
        throw new Error(
          `Fixture did not render expected text ${JSON.stringify(expected)}. ` +
            `Visible frame:\n${plainFrame.slice(0, 1_200)}`,
        );
      }
    }
    const terminalRows = terminalToHtml(terminal, {
      cols,
      rows,
      foreground,
      background,
    });
    return await captureTerminalHtml({
      terminalRows,
      outPath: request.outPath,
      headed: request.headed,
      cols,
      rows,
      scale,
      background,
      foreground,
      fontFamily,
      fontSize,
      lineHeight,
      padding,
      borderRadius,
    });
  } finally {
    if (child && !exited) {
      try {
        child.kill();
      } catch {
        // The process may have exited between the state check and kill.
      }
      await Promise.race([exitPromise, delay(500)]);
      if (!exited) {
        try {
          child.kill("SIGKILL");
        } catch {
          // ConPTY and already-exited Unix children can reject a second kill.
        }
        await Promise.race([exitPromise, delay(500)]);
      }
    }
    await writes;
    terminal.dispose();
  }
}

export function takePtyShot(request: PtyShotRequest): Promise<string> {
  return queueTerminalShot(() => takeIsolatedPtyShot(request));
}
