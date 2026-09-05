/**
 * Detect what the host terminal can draw before Ink takes over stdin/stdout.
 *
 * Sends the kitty graphics query, the cell-size and window-size reports, and a
 * primary device attributes request whose reply always arrives last, so a
 * terminal that ignores the graphics query still terminates the probe.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { encodeFileQuery, encodeQuery } from "./kitty.js";

export type GraphicsProtocol = "kitty" | "none";

export interface TerminalCapabilities {
  graphics: GraphicsProtocol;
  /** Whether the terminal accepted a file-path transmission (t=f). */
  fileMedium: boolean;
  /** Pixel size of one cell. Falls back to a 1:2 guess when unreported. */
  cellWidth: number;
  cellHeight: number;
  cellSource: "query" | "env" | "fallback";
  /** Why graphics are off, for the Settings pane. */
  reason?: string;
  insideTmux: boolean;
  insideSsh: boolean;
}

export const FALLBACK_CELL = { width: 10, height: 20 };

const QUERY_ID = 31;
const FILE_QUERY_ID = 32;

// Smallest valid PNG: 1×1 transparent pixel.
const PROBE_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  "base64",
);

export interface ProbeStreams {
  stdin: NodeJS.ReadStream;
  stdout: NodeJS.WriteStream;
}

export interface ProbeOptions {
  timeoutMs?: number;
  env?: NodeJS.ProcessEnv;
  probeFileMedium?: boolean;
}

export function parseCellSizeEnv(value: string | undefined): { width: number; height: number } | null {
  if (!value) return null;
  const match = /^(\d+)\s*[x×]\s*(\d+)$/i.exec(value.trim());
  if (!match) return null;
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (width <= 0 || height <= 0) return null;
  return { width, height };
}

export interface ProbeReport {
  kittyOk: boolean;
  fileOk: boolean;
  cellWidth?: number;
  cellHeight?: number;
  windowWidth?: number;
  windowHeight?: number;
  sawDeviceAttributes: boolean;
}

/** Interpret the raw bytes a terminal wrote back during the probe. */
export function parseProbeResponse(text: string): ProbeReport {
  const report: ProbeReport = {
    kittyOk: false,
    fileOk: false,
    sawDeviceAttributes: /\x1b\[\?[\d;]*c/.test(text),
  };
  const graphics = /\x1b_G([^\x1b]*)\x1b\\/g;
  for (const match of text.matchAll(graphics)) {
    const body = match[1] ?? "";
    if (body.includes(`i=${QUERY_ID}`) && /;OK/.test(body)) report.kittyOk = true;
    if (body.includes(`i=${FILE_QUERY_ID}`) && /;OK/.test(body)) report.fileOk = true;
  }
  const cell = /\x1b\[6;(\d+);(\d+)t/.exec(text);
  if (cell) {
    report.cellHeight = Number(cell[1]);
    report.cellWidth = Number(cell[2]);
  }
  const window = /\x1b\[4;(\d+);(\d+)t/.exec(text);
  if (window) {
    report.windowHeight = Number(window[1]);
    report.windowWidth = Number(window[2]);
  }
  return report;
}

function readResponse(
  stdin: NodeJS.ReadStream,
  timeoutMs: number,
): Promise<string> {
  return new Promise((resolve) => {
    let buffer = "";
    let settled = false;
    const wasRaw = stdin.isRaw;
    const finish = () => {
      if (settled) return;
      settled = true;
      stdin.off("data", onData);
      clearTimeout(timer);
      if (stdin.isTTY && !wasRaw) stdin.setRawMode(false);
      stdin.pause();
      resolve(buffer);
    };
    const onData = (chunk: Buffer | string) => {
      buffer += chunk.toString();
      // Primary DA reply terminates the probe.
      if (/\x1b\[\?[\d;]*c/.test(buffer)) finish();
    };
    const timer = setTimeout(finish, timeoutMs);
    if (stdin.isTTY && !wasRaw) stdin.setRawMode(true);
    stdin.on("data", onData);
    stdin.resume();
  });
}

export async function probeTerminal(
  streams: ProbeStreams,
  options: ProbeOptions = {},
): Promise<TerminalCapabilities> {
  const env = options.env ?? process.env;
  const timeoutMs = options.timeoutMs ?? 600;
  const insideTmux = Boolean(env.TMUX);
  const insideSsh = Boolean(env.SSH_CONNECTION || env.SSH_TTY || env.SSH_CLIENT);
  const envCell = parseCellSizeEnv(env.ASTROSHOT_REVIEW_CELL_PX);
  const forced = env.ASTROSHOT_REVIEW_GRAPHICS;

  const base: TerminalCapabilities = {
    graphics: "none",
    fileMedium: false,
    cellWidth: envCell?.width ?? FALLBACK_CELL.width,
    cellHeight: envCell?.height ?? FALLBACK_CELL.height,
    cellSource: envCell ? "env" : "fallback",
    insideTmux,
    insideSsh,
  };

  if (forced === "none") {
    return { ...base, reason: "ASTROSHOT_REVIEW_GRAPHICS=none" };
  }
  if (forced === "kitty") {
    return { ...base, graphics: "kitty", fileMedium: env.ASTROSHOT_REVIEW_FILE_MEDIUM === "1" };
  }
  if (!streams.stdout.isTTY || !streams.stdin.isTTY) {
    return { ...base, reason: "stdin/stdout is not a terminal" };
  }
  if (insideTmux) {
    return {
      ...base,
      reason: "tmux does not pass graphics through by default; run outside tmux",
    };
  }

  let probeFile: string | null = null;
  const wantFileProbe = options.probeFileMedium ?? !insideSsh;
  if (wantFileProbe) {
    try {
      const directory = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-review-probe-"));
      probeFile = path.join(directory, "probe.png");
      fs.writeFileSync(probeFile, PROBE_PNG);
    } catch {
      probeFile = null;
    }
  }

  const query =
    encodeQuery(QUERY_ID) +
    (probeFile ? encodeFileQuery(FILE_QUERY_ID, probeFile) : "") +
    "\x1b[16t" +
    "\x1b[14t" +
    "\x1b[c";

  const pending = readResponse(streams.stdin, timeoutMs);
  streams.stdout.write(query);
  const response = await pending;
  if (probeFile) {
    fs.rmSync(path.dirname(probeFile), { recursive: true, force: true });
  }
  const report = parseProbeResponse(response);

  let cellWidth = base.cellWidth;
  let cellHeight = base.cellHeight;
  let cellSource = base.cellSource;
  if (!envCell) {
    if (report.cellWidth && report.cellHeight) {
      cellWidth = report.cellWidth;
      cellHeight = report.cellHeight;
      cellSource = "query";
    } else if (report.windowWidth && report.windowHeight) {
      const columns = streams.stdout.columns || 80;
      const rows = streams.stdout.rows || 24;
      cellWidth = Math.max(1, Math.floor(report.windowWidth / columns));
      cellHeight = Math.max(1, Math.floor(report.windowHeight / rows));
      cellSource = "query";
    }
  }

  if (!report.kittyOk) {
    return {
      ...base,
      cellWidth,
      cellHeight,
      cellSource,
      reason: report.sawDeviceAttributes
        ? "terminal did not answer the kitty graphics query (try Ghostty, kitty, or WezTerm)"
        : "terminal did not answer the capability probe",
    };
  }
  return {
    graphics: "kitty",
    fileMedium: report.fileOk,
    cellWidth,
    cellHeight,
    cellSource,
    insideTmux,
    insideSsh,
  };
}
