import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

import { MovieSession } from "../session.js";
import type { MovieArtifact, MovieSessionOptions, Size } from "../types.js";

export interface DesktopWindowInfo {
  id: number;
  pid: number;
  owner: string;
  title: string;
  bundleId: string | null;
  width: number;
  height: number;
  x: number;
  y: number;
  onScreen: boolean;
  /** CGWindow layer; 0 = normal, >0 = floating/popover chrome. */
  layer?: number;
}

export interface DesktopWindowMatch {
  windowId?: number;
  bundleId?: string;
  titleRegex?: string;
  owner?: string;
  pid?: number;
  /** When multiple match, pick largest (default) or first. */
  pick?: "largest" | "first";
  /**
   * Prefer on-screen windows when several match (default true).
   * Off-screen / empty host windows often produce black frames.
   */
  preferOnScreen?: boolean;
}

export type DesktopWindowMovieOptions = Omit<MovieSessionOptions, "source"> & {
  match: DesktopWindowMatch;
  /** How long to sample. Default 3000. */
  durationMs?: number;
  /** Include cursor in frames (screencapture -C). Default false. */
  cursor?: boolean;
  /**
   * Allow mostly-blank / black posters to encode (default false).
   * Off-screen windows and denied Screen Recording often produce black frames.
   */
  allowBlank?: boolean;
};

const SCREENCAPTURE = "/usr/sbin/screencapture";

/** Known deep-links to the Screen Recording privacy pane (varies by macOS). */
const SCREEN_RECORDING_SETTINGS_URLS = [
  "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
  "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture",
];

export interface ScreenAccessReport {
  granted: boolean;
  requested: boolean;
  hostApp: string;
  hostBundleId: string | null;
  settingsHint: string;
  /** Best-effort name of the app the user should enable (terminal/IDE). */
  enableApp: string;
}

function packageNativeSwift(): string {
  const here = path.dirname(fileURLToPath(import.meta.url));
  // dist/sources -> ../../native/macos
  return path.resolve(here, "../../native/macos/WindowTools.swift");
}

function assertMacOS(): void {
  if (process.platform !== "darwin") {
    throw new Error(
      "desktop.window is only implemented on macOS (CGWindowList + screencapture). " +
        "On other platforms use --source frames and push your own captures. " +
        "See: astroshot movie which-source",
    );
  }
}

function assertScreencapture(): void {
  if (!fs.existsSync(SCREENCAPTURE)) {
    throw new Error(`screencapture not found at ${SCREENCAPTURE}`);
  }
}

function runWindowTools(args: string[]): {
  status: number | null;
  stdout: string;
  stderr: string;
} {
  const script = packageNativeSwift();
  if (!fs.existsSync(script)) {
    throw new Error(
      `WindowTools.swift missing at ${script}. Reinstall @archastro/movie-harness.`,
    );
  }
  const result = spawnSync("swift", [script, ...args], {
    encoding: "utf8",
    maxBuffer: 8 * 1024 * 1024,
  });
  if (result.error) {
    throw new Error(
      `Could not run Swift window tools (${result.error.message}). ` +
        "Install Xcode Command Line Tools (`xcode-select --install`).",
    );
  }
  return {
    status: result.status,
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
  };
}

/**
 * Open System Settings to the Screen Recording privacy list.
 * Best-effort: URL schemes differ slightly across macOS versions.
 */
export function openScreenRecordingSettings(): boolean {
  assertMacOS();
  for (const url of SCREEN_RECORDING_SETTINGS_URLS) {
    const result = spawnSync("open", [url], { encoding: "utf8" });
    if (result.status === 0) return true;
  }
  // Fallback: open the Privacy & Security root.
  const fallback = spawnSync(
    "open",
    ["x-apple.systempreferences:com.apple.preference.security"],
    { encoding: "utf8" },
  );
  return fallback.status === 0;
}

/**
 * Name the app the human should toggle in Screen Recording settings.
 * Prefer $TERM_PROGRAM (Ghostty, iTerm, vscode…) over the Swift runner process.
 */
export function resolveEnableAppName(swiftHostApp?: string): string {
  const term = process.env.TERM_PROGRAM?.trim();
  if (term) {
    const map: Record<string, string> = {
      ghostty: "Ghostty",
      "iTerm.app": "iTerm",
      Apple_Terminal: "Terminal",
      vscode: "Code", // VS Code / Cursor often still set vscode
      WarpTerminal: "Warp",
      WezTerm: "WezTerm",
      Alacritty: "Alacritty",
    };
    return map[term] ?? term;
  }
  if (process.env.CURSOR_TRACE_ID || process.env.VSCODE_PID) {
    return process.env.CURSOR_TRACE_ID ? "Cursor" : "Code";
  }
  if (swiftHostApp && !/swift/i.test(swiftHostApp)) return swiftHostApp;
  return "your terminal or IDE (the app that launched this command)";
}

/**
 * Detect Screen Recording TCC (best-effort).
 *
 * Uses CoreGraphics preflight via Swift. Note: the Swift process identity may
 * differ from `screencapture`'s responsible app (your terminal). Capture
 * failure remains authoritative; this steers the human to Settings early.
 */
export function checkScreenRecordingAccess(options?: {
  /** Call CGRequestScreenCaptureAccess when not already granted. */
  request?: boolean;
}): ScreenAccessReport {
  assertMacOS();
  const args = ["screen-access"];
  if (options?.request) args.push("--request");
  const result = runWindowTools(args);
  const text = result.stdout.trim();
  if (!text) {
    throw new Error(
      `screen-access returned no JSON (status ${result.status}): ${result.stderr.slice(0, 400)}`,
    );
  }
  const parsed = JSON.parse(text) as Omit<ScreenAccessReport, "enableApp">;
  const enableApp = resolveEnableAppName(parsed.hostApp);
  return {
    granted: Boolean(parsed.granted),
    requested: Boolean(parsed.requested),
    hostApp: parsed.hostApp || "unknown",
    hostBundleId: parsed.hostBundleId ?? null,
    enableApp,
    settingsHint: `System Settings → Privacy & Security → Screen Recording → enable ${enableApp}, then quit & reopen it`,
  };
}

export function formatScreenRecordingDeniedHelp(
  report?: ScreenAccessReport,
): string {
  const app = report?.enableApp ?? resolveEnableAppName();
  const lines = [
    `Screen Recording permission is required for --source desktop.window.`,
    ``,
    `Fix:`,
    `  1. Open System Settings → Privacy & Security → Screen Recording`,
    `     (or: astroshot movie open-screen-settings)`,
    `  2. Enable "${app}"`,
    `  3. Quit and reopen ${app} completely (TCC applies on next launch)`,
    `  4. Re-run: astroshot movie check-screen-access`,
    ``,
    `Note: macOS may not always show an automatic prompt; the Settings toggle is the reliable path.`,
    `browser / pty sources do not need this permission.`,
  ];
  return lines.join("\n");
}

function throwScreenRecordingDenied(report?: ScreenAccessReport): never {
  // Best-effort: open Settings so the human does not have to hunt.
  try {
    openScreenRecordingSettings();
  } catch {
    /* ignore */
  }
  throw new Error(formatScreenRecordingDeniedHelp(report));
}

/**
 * Preflight Screen Recording; optionally request + open Settings on deny.
 * Call before desktop.window capture.
 */
export function ensureScreenRecordingAccess(options?: {
  request?: boolean;
  /** Open System Settings when denied (default true). */
  openSettings?: boolean;
}): ScreenAccessReport {
  const report = checkScreenRecordingAccess({
    request: options?.request ?? true,
  });
  if (report.granted) return report;
  if (options?.openSettings !== false) {
    try {
      openScreenRecordingSettings();
    } catch {
      /* ignore */
    }
  }
  throw new Error(formatScreenRecordingDeniedHelp(report));
}

/** List layer-0 windows as JSON via shipped Swift tool (interpreted by `swift`). */
export function listDesktopWindows(): DesktopWindowInfo[] {
  assertMacOS();
  const result = runWindowTools(["list"]);
  if (result.status !== 0) {
    throw new Error(
      `Window list failed (${result.status}): ${result.stderr.slice(0, 500)}`,
    );
  }
  const text = result.stdout.trim();
  if (!text) return [];
  const parsed = JSON.parse(text) as DesktopWindowInfo[];
  return parsed.map((row) => ({
    ...row,
    bundleId: row.bundleId ?? null,
  }));
}

export function matchDesktopWindow(
  windows: DesktopWindowInfo[],
  match: DesktopWindowMatch,
): DesktopWindowInfo {
  let candidates = windows;

  if (match.windowId !== undefined) {
    candidates = candidates.filter((w) => w.id === match.windowId);
  }
  if (match.bundleId) {
    const want = match.bundleId.toLowerCase();
    candidates = candidates.filter(
      (w) => (w.bundleId ?? "").toLowerCase() === want,
    );
  }
  if (match.owner) {
    const want = match.owner.toLowerCase();
    candidates = candidates.filter((w) =>
      w.owner.toLowerCase().includes(want),
    );
  }
  if (match.pid !== undefined) {
    candidates = candidates.filter((w) => w.pid === match.pid);
  }
  if (match.titleRegex) {
    const re = new RegExp(match.titleRegex, "i");
    candidates = candidates.filter((w) => re.test(w.title));
  }

  if (candidates.length === 0) {
    const sample = windows
      .slice(0, 8)
      .map(
        (w) =>
          `  id=${w.id} pid=${w.pid} layer=${w.layer ?? "?"} bundle=${w.bundleId ?? "?"} owner=${JSON.stringify(w.owner)} title=${JSON.stringify(w.title)} ${w.width}x${w.height} onScreen=${w.onScreen}`,
      )
      .join("\n");
    throw new Error(
      `No desktop window matched ${JSON.stringify(match)}.\n` +
        `Run: astroshot movie list-windows\n` +
        `Sample windows:\n${sample || "  (none)"}`,
    );
  }

  const preferOnScreen = match.preferOnScreen !== false;
  if (preferOnScreen && match.windowId === undefined) {
    const onScreen = candidates.filter((w) => w.onScreen);
    if (onScreen.length > 0) candidates = onScreen;
  }

  if (match.pick === "first") return candidates[0]!;
  // Default: largest area, then lower layer (normal windows over floaters).
  return [...candidates].sort((a, b) => {
    const area = b.width * b.height - a.width * a.height;
    if (area !== 0) return area;
    return (a.layer ?? 0) - (b.layer ?? 0);
  })[0]!;
}

/** Human-readable manifest description for a captured window. */
export function describeDesktopWindow(target: DesktopWindowInfo): string {
  const name = target.bundleId?.split(".").pop() || target.owner || "window";
  const title = target.title.trim();
  const size = `${target.width}×${target.height}`;
  const where = target.onScreen ? "on-screen" : "off-screen";
  const layer =
    target.layer !== undefined && target.layer > 0
      ? `, layer ${target.layer}`
      : "";
  if (title) {
    return `${name}: “${title}” (${size}, ${where}${layer})`;
  }
  return `${name} window (${size}, ${where}${layer})`;
}

/**
 * True when a PNG is almost entirely very dark (typical failed/off-screen capture).
 * Samples up to ~4k pixels across a simple grid; supports 8-bit RGB/RGBA.
 */
export function isNearlyBlankPng(
  filePath: string,
  options?: { maxMeanLuma?: number; minDarkFraction?: number },
): boolean {
  const maxMeanLuma = options?.maxMeanLuma ?? 12;
  const minDarkFraction = options?.minDarkFraction ?? 0.97;
  let data: Buffer;
  try {
    data = fs.readFileSync(filePath);
  } catch {
    return true;
  }
  if (data.length < 33 || data.toString("ascii", 1, 4) !== "PNG") return true;

  // Minimal PNG scan: find IDAT chunks, inflate, average luma on a grid.
  const width = data.readUInt32BE(16);
  const height = data.readUInt32BE(20);
  const bitDepth = data[24];
  const colorType = data[25];
  if (!width || !height || bitDepth !== 8) return false;
  // 2 = RGB, 6 = RGBA
  if (colorType !== 2 && colorType !== 6) return false;
  const channels = colorType === 6 ? 4 : 3;

  const idatParts: Buffer[] = [];
  let offset = 8;
  while (offset + 8 <= data.length) {
    const len = data.readUInt32BE(offset);
    const type = data.toString("ascii", offset + 4, offset + 8);
    const start = offset + 8;
    const end = start + len;
    if (end + 4 > data.length) break;
    if (type === "IDAT") idatParts.push(data.subarray(start, end));
    if (type === "IEND") break;
    offset = end + 4;
  }
  if (idatParts.length === 0) return true;

  let inflated: Buffer;
  try {
    inflated = zlib.inflateSync(Buffer.concat(idatParts));
  } catch {
    return false; // can't decode — don't claim blank
  }

  const stride = 1 + width * channels; // filter byte + row
  const expected = stride * height;
  if (inflated.length < expected) return false;

  // Only sample rows that use filter type 0 (None) for correct RGB bytes.
  // For filtered rows, still sample raw bytes as a coarse darkness heuristic.
  let samples = 0;
  let dark = 0;
  let lumaSum = 0;
  const stepY = Math.max(1, Math.floor(height / 32));
  const stepX = Math.max(1, Math.floor(width / 32));
  for (let y = 0; y < height; y += stepY) {
    const rowStart = y * stride;
    const filter = inflated[rowStart] ?? 0;
    for (let x = 0; x < width; x += stepX) {
      const i = rowStart + 1 + x * channels;
      const r = inflated[i] ?? 0;
      const g = inflated[i + 1] ?? 0;
      const b = inflated[i + 2] ?? 0;
      // filter≠0 means bytes aren't raw RGB; still treat very low triples as dark.
      const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
      lumaSum += luma;
      if (luma <= maxMeanLuma) dark += 1;
      samples += 1;
      void filter;
    }
  }
  if (samples === 0) return true;
  const mean = lumaSum / samples;
  const darkFraction = dark / samples;
  return mean <= maxMeanLuma && darkFraction >= minDarkFraction;
}

function captureWindowPng(
  windowId: number,
  outPath: string,
  cursor: boolean,
): void {
  const finalArgs = cursor
    ? ["-x", "-C", "-o", "-t", "png", "-l", String(windowId), outPath]
    : ["-x", "-o", "-t", "png", "-l", String(windowId), outPath];

  const result = spawnSync(SCREENCAPTURE, finalArgs, { encoding: "utf8" });
  if (result.status !== 0) {
    let access: ScreenAccessReport | undefined;
    try {
      access = checkScreenRecordingAccess({ request: false });
    } catch {
      /* ignore */
    }
    if (access && !access.granted) {
      throwScreenRecordingDenied(access);
    }
    throw new Error(
      `screencapture failed for window ${windowId} (${result.status}): ` +
        `${result.stderr || result.stdout || "unknown error"}.\n` +
        formatScreenRecordingDeniedHelp(access),
    );
  }
  if (!fs.existsSync(outPath) || fs.statSync(outPath).size < 32) {
    let access: ScreenAccessReport | undefined;
    try {
      access = checkScreenRecordingAccess({ request: false });
    } catch {
      /* ignore */
    }
    if (access && !access.granted) {
      throwScreenRecordingDenied(access);
    }
    throw new Error(
      `screencapture produced an empty image for window ${windowId}. ` +
        "The window may have closed, or Screen Recording is denied.\n" +
        formatScreenRecordingDeniedHelp(access),
    );
  }
}

function readPngSize(filePath: string): Size | null {
  const fd = fs.openSync(filePath, "r");
  try {
    const buf = Buffer.alloc(24);
    fs.readSync(fd, buf, 0, 24, 0);
    if (buf.toString("ascii", 1, 4) !== "PNG") return null;
    return {
      width: buf.readUInt32BE(16),
      height: buf.readUInt32BE(20),
    };
  } finally {
    fs.closeSync(fd);
  }
}

/**
 * Sample a macOS window at `fps` for `durationMs`, encode to movie + poster.
 * Uses OS `screencapture` (already on every Mac) — no separate download.
 */
export async function recordDesktopWindowMovie(
  options: DesktopWindowMovieOptions,
): Promise<MovieArtifact> {
  assertMacOS();
  assertScreencapture();

  const durationMs = options.durationMs ?? 3_000;
  const fps = options.fps ?? 10;
  if (!(durationMs > 0)) {
    throw new Error("--duration-ms must be positive");
  }

  // Detect TCC up front (may prompt once; opens Settings if still denied).
  ensureScreenRecordingAccess({ request: true, openSettings: true });

  const windows = listDesktopWindows();
  const target = matchDesktopWindow(windows, options.match);

  if (!target.onScreen && !options.allowBlank) {
    throw new Error(
      `Matched window id=${target.id} (${target.bundleId ?? target.owner}) is off-screen ` +
        `(${target.width}×${target.height} at ${target.x},${target.y}). ` +
        "Off-screen windows usually produce black movies. Bring the window on-screen " +
        "(open the tray/popover) or pass --allow-blank to record anyway.\n" +
        `Hint: ${describeDesktopWindow(target)}`,
    );
  }

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-desktop-"));
  const probe = path.join(tmp, "probe.png");
  try {
    captureWindowPng(target.id, probe, Boolean(options.cursor));
  } catch (error) {
    fs.rmSync(tmp, { recursive: true, force: true });
    throw error;
  }

  if (!options.allowBlank && isNearlyBlankPng(probe)) {
    fs.rmSync(tmp, { recursive: true, force: true });
    throw new Error(
      `Probe frame for window id=${target.id} is nearly blank/black. ` +
        "Screen Recording may be denied for this host app, the window may be empty, " +
        "or the popover may not be visible. Fix visibility/TCC, or pass --allow-blank.\n" +
        `Window: ${describeDesktopWindow(target)}\n` +
        "Run: astroshot movie check-screen-access",
    );
  }

  const probed = readPngSize(probe);
  const size: Size = options.size ??
    probed ?? {
      width: target.width,
      height: target.height,
    };

  const session = MovieSession.create({
    ...options,
    size,
    fps,
    source: "desktop.window",
    description: options.description ?? describeDesktopWindow(target),
  });

  // Seed with probe frame so we never end empty if duration is tiny.
  session.pushFrame(fs.readFileSync(probe));

  const intervalMs = Math.max(50, Math.round(1000 / fps));
  const deadline = Date.now() + durationMs;
  let index = 1;

  try {
    while (Date.now() < deadline) {
      const framePath = path.join(tmp, `f-${String(index).padStart(5, "0")}.png`);
      const started = Date.now();
      try {
        captureWindowPng(target.id, framePath, Boolean(options.cursor));
        session.pushFrameFile(framePath);
      } catch (error) {
        // Window may close mid-recording; stop cleanly if we have frames.
        if (session.listFrames().length > 0) break;
        throw error;
      }
      index += 1;
      const elapsed = Date.now() - started;
      const sleep = Math.max(0, intervalMs - elapsed);
      if (sleep > 0 && Date.now() + sleep < deadline) {
        await new Promise((r) => setTimeout(r, sleep));
      }
    }

    return await session.stop({ status: options.status ?? "running" });
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

/** Resolve match flags from CLI-style strings. */
export function desktopMatchFromFlags(flags: {
  "window-id"?: string;
  "bundle-id"?: string;
  "title-regex"?: string;
  owner?: string;
  pid?: string;
  pick?: string;
}): DesktopWindowMatch {
  const match: DesktopWindowMatch = {};
  if (flags["window-id"]) match.windowId = Number(flags["window-id"]);
  if (flags["bundle-id"]) match.bundleId = flags["bundle-id"];
  if (flags["title-regex"]) match.titleRegex = flags["title-regex"];
  if (flags.owner) match.owner = flags.owner;
  if (flags.pid) match.pid = Number(flags.pid);
  if (flags.pick === "first" || flags.pick === "largest") {
    match.pick = flags.pick;
  }
  if (
    match.windowId === undefined &&
    !match.bundleId &&
    !match.titleRegex &&
    !match.owner &&
    match.pid === undefined
  ) {
    throw new Error(
      "desktop.window requires one of --window-id, --bundle-id, --title-regex, --owner, or --pid. " +
        "Run: astroshot movie list-windows",
    );
  }
  if (match.windowId !== undefined && Number.isNaN(match.windowId)) {
    throw new Error("--window-id must be a number");
  }
  if (match.pid !== undefined && Number.isNaN(match.pid)) {
    throw new Error("--pid must be a number");
  }
  return match;
}

/** Ensure Swift is runnable (for clearer errors at CLI start). */
export function assertDesktopToolchain(): void {
  assertMacOS();
  try {
    execFileSync("swift", ["--version"], { encoding: "utf8", stdio: "pipe" });
  } catch {
    throw new Error(
      "desktop.window requires the Swift toolchain (`swift` on PATH). " +
        "Install Xcode Command Line Tools: xcode-select --install",
    );
  }
  assertScreencapture();
}
