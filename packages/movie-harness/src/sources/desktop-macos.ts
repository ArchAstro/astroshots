import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
}

export interface DesktopWindowMatch {
  windowId?: number;
  bundleId?: string;
  titleRegex?: string;
  owner?: string;
  pid?: number;
  /** When multiple match, pick largest (default) or first. */
  pick?: "largest" | "first";
}

export type DesktopWindowMovieOptions = Omit<MovieSessionOptions, "source"> & {
  match: DesktopWindowMatch;
  /** How long to sample. Default 3000. */
  durationMs?: number;
  /** Include cursor in frames (screencapture -C). Default false. */
  cursor?: boolean;
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
          `  id=${w.id} pid=${w.pid} bundle=${w.bundleId ?? "?"} owner=${JSON.stringify(w.owner)} title=${JSON.stringify(w.title)} ${w.width}x${w.height}`,
      )
      .join("\n");
    throw new Error(
      `No desktop window matched ${JSON.stringify(match)}.\n` +
        `Run: astroshot movie list-windows\n` +
        `Sample windows:\n${sample || "  (none)"}`,
    );
  }

  if (match.pick === "first") return candidates[0]!;
  // Default largest (list is already size-sorted, but re-sort for safety).
  return [...candidates].sort(
    (a, b) => b.width * b.height - a.width * a.height,
  )[0]!;
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

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-desktop-"));
  const probe = path.join(tmp, "probe.png");
  try {
    captureWindowPng(target.id, probe, Boolean(options.cursor));
  } catch (error) {
    fs.rmSync(tmp, { recursive: true, force: true });
    throw error;
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
    description:
      options.description ??
      `desktop.window id=${target.id} ${target.bundleId ?? target.owner} ${JSON.stringify(target.title)}`,
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
