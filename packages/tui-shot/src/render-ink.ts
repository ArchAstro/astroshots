import { EventEmitter } from "node:events";
import type { render as inkRender } from "ink";

import type { TuiShotFixture } from "./types.js";

export interface InkRuntime {
  render: typeof inkRender;
  chalk: { level: number };
}

const CI_KEYS = [
  "CI",
  "CONTINUOUS_INTEGRATION",
  "GITHUB_ACTIONS",
  "BUILD_NUMBER",
  "RUN_ID",
  "GITLAB_CI",
  "CIRCLECI",
  "TRAVIS",
  "BUILDKITE",
];

function fakeStdin(): NodeJS.ReadStream {
  const stream = new EventEmitter() as unknown as Record<string, unknown>;
  stream.isTTY = true;
  stream.setRawMode = () => stream;
  stream.setEncoding = () => stream;
  stream.resume = () => stream;
  stream.pause = () => stream;
  stream.ref = () => stream;
  stream.unref = () => stream;
  stream.read = () => null;
  return stream as unknown as NodeJS.ReadStream;
}

function printableText(chunk: string): string {
  return chunk
    .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\x1b[@-_]/g, "");
}

/** Render one complete ANSI frame from a deterministic Ink fixture. */
export function renderInkFrame(
  fixture: TuiShotFixture,
  cols: number,
  rows: number,
  runtime: InkRuntime,
): string {
  const chunks: string[] = [];
  const stdout = {
    columns: cols,
    rows,
    isTTY: true,
    write: (chunk: string | Uint8Array) => {
      chunks.push(String(chunk));
      return true;
    },
    on() {},
    off() {},
    once() {},
    removeListener() {},
    end() {},
  } as unknown as NodeJS.WriteStream;
  const saved = new Map<string, string | undefined>();
  const chalkLevel = runtime.chalk.level;
  for (const key of CI_KEYS) {
    saved.set(key, process.env[key]);
    delete process.env[key];
  }

  let frame = "";
  try {
    runtime.chalk.level = 3;
    const instance = runtime.render(fixture.component, {
      debug: true,
      exitOnCtrlC: false,
      patchConsole: false,
      stdin: fakeStdin(),
      stdout,
    });
    for (let index = chunks.length - 1; index >= 0; index--) {
      const chunk = chunks[index]!;
      if (printableText(chunk).trim().length > 0) {
        frame = chunk;
        break;
      }
    }
    instance.unmount();
    instance.cleanup();
  } finally {
    runtime.chalk.level = chalkLevel;
    for (const [key, value] of saved) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }

  if (!frame) {
    throw new Error(
      "Ink produced no printable frame. Check that the fixture renders visible content.",
    );
  }
  return frame;
}
