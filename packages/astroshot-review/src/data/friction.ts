/**
 * Friction logs: `.astroshot/friction-logs/<slug>/{prompt.md,meta.json,runs/<run>/log.jsonl}`.
 * Mirrors the macOS loader, including field aliases and silent skipping of
 * malformed lines and missing screenshots.
 */
import fs from "node:fs";
import path from "node:path";

import type { HashCache } from "./hash-cache.js";
import type { FrictionLog, FrictionRun, FrictionStep, ReviewSnapshot } from "./model.js";
import { humanize, worktreeShort } from "./paths.js";
import {
  entryNeedsHash,
  readReviewDocument,
  scopedEntry,
  snapshotFromEntry,
} from "./review-store.js";

export const PROMPT_FILE = "prompt.md";
export const META_FILE = "meta.json";
export const RUNS_DIR = "runs";
export const LOG_FILE = "log.jsonl";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function stringList(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .filter((entry): entry is string => typeof entry === "string")
      .map((entry) => entry.trim())
      .filter(Boolean);
  }
  if (typeof value === "string" && value.trim()) return [value.trim()];
  return [];
}

function firstString(record: Record<string, unknown>, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string") return value;
  }
  return null;
}

async function exists(filePath: string): Promise<boolean> {
  try {
    await fs.promises.access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function isDirectoryEntry(parent: string, entry: fs.Dirent): Promise<boolean> {
  if (entry.isDirectory()) return true;
  if (!entry.isSymbolicLink()) return false;
  try {
    return (await fs.promises.stat(path.join(parent, entry.name))).isDirectory();
  } catch {
    return false;
  }
}

async function mtimeOf(filePath: string): Promise<number | null> {
  try {
    return (await fs.promises.stat(filePath)).mtimeMs;
  } catch {
    return null;
  }
}

/** Parse JSONL text into ordered steps; screenshots are resolved against `runDir`. */
export async function parseJsonl(text: string, runDir: string): Promise<FrictionStep[]> {
  const steps: FrictionStep[] = [];
  let index = 0;
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch {
      continue;
    }
    if (!isRecord(parsed)) continue;
    index += 1;
    const step = typeof parsed.step === "number" ? parsed.step : index;
    const stepId = firstString(parsed, "id") ?? `step-${step}`;
    const screenshots: string[] = [];
    for (const name of stringList(parsed.screenshots ?? parsed.screenshot)) {
      const byBasename = path.join(runDir, path.basename(name));
      const asWritten = path.resolve(runDir, name);
      if (await exists(byBasename)) screenshots.push(byBasename);
      else if (await exists(asWritten)) screenshots.push(asWritten);
    }
    steps.push({
      id: `${step}-${stepId}`,
      step,
      stepId,
      title: firstString(parsed, "title") ?? humanize(stepId),
      description: firstString(parsed, "description") ?? "",
      transcript: firstString(parsed, "transcript", "narration", "voiceover") ?? "",
      screenshots,
      good: stringList(parsed.good ?? parsed.looks_good),
      improve: stringList(parsed.improve ?? parsed.can_improve ?? parsed.improvements),
      url: firstString(parsed, "url"),
      capturedAt: firstString(parsed, "captured_at"),
    });
  }
  return steps.sort((a, b) => a.step - b.step);
}

async function runReview(runDir: string, runId: string, hashes: HashCache): Promise<ReviewSnapshot | null> {
  const logPath = path.join(runDir, LOG_FILE);
  let document;
  try {
    document = await readReviewDocument(runDir);
  } catch {
    return null;
  }
  if (!document) return null;
  const entry = scopedEntry(document, LOG_FILE, runId);
  let sha: string | null = null;
  if (entryNeedsHash(entry)) {
    try {
      sha = await hashes.hash(logPath);
    } catch {
      sha = null;
    }
  }
  return snapshotFromEntry(entry, sha);
}

export async function loadRun(runDir: string, runId: string, hashes: HashCache): Promise<FrictionRun | null> {
  const logPath = path.join(runDir, LOG_FILE);
  const hasLog = await exists(logPath);
  let steps: FrictionStep[] = [];
  if (hasLog) {
    steps = await parseJsonl(await fs.promises.readFile(logPath, "utf8"), runDir);
  } else {
    let entries: string[];
    try {
      entries = await fs.promises.readdir(runDir);
    } catch {
      return null;
    }
    if (!entries.some((entry) => /\.(png|jpe?g|webp|gif)$/i.test(entry))) return null;
  }
  const capturedAt = (await mtimeOf(runDir)) ?? (await mtimeOf(logPath)) ?? Date.now();
  return {
    runId,
    directory: runDir,
    logPath: hasLog ? logPath : null,
    capturedAt,
    status: null,
    steps,
    review: hasLog ? await runReview(runDir, runId, hashes) : null,
  };
}

export async function loadRuns(logDir: string, hashes: HashCache): Promise<FrictionRun[]> {
  const runsDir = path.join(logDir, RUNS_DIR);
  const runs: FrictionRun[] = [];
  let entries: fs.Dirent[] = [];
  try {
    entries = await fs.promises.readdir(runsDir, { withFileTypes: true });
  } catch {
    entries = [];
  }
  for (const entry of entries) {
    if (entry.name.startsWith(".") || !(await isDirectoryEntry(runsDir, entry))) continue;
    const run = await loadRun(path.join(runsDir, entry.name), entry.name, hashes);
    if (run) runs.push(run);
  }
  if (runs.length === 0 && (await exists(path.join(logDir, LOG_FILE)))) {
    const flat = await loadRun(logDir, "latest", hashes);
    if (flat) runs.push(flat);
  }
  return runs.sort((a, b) => b.capturedAt - a.capturedAt);
}

export async function loadFrictionLog(
  logDir: string,
  context: { worktreePath: string; worktree: string },
  hashes: HashCache,
): Promise<FrictionLog | null> {
  const slug = path.basename(logDir);
  const promptPath = path.join(logDir, PROMPT_FILE);
  const metaPath = path.join(logDir, META_FILE);
  const hasPrompt = await exists(promptPath);
  const runs = await loadRuns(logDir, hashes);
  if (!hasPrompt && runs.length === 0) return null;

  let meta: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(await fs.promises.readFile(metaPath, "utf8"));
    if (isRecord(parsed)) meta = parsed;
  } catch {
    meta = {};
  }
  const metaTitle = firstString(meta, "title")?.trim();
  const candidates = [
    await mtimeOf(logDir),
    hasPrompt ? await mtimeOf(promptPath) : null,
    await mtimeOf(metaPath),
    runs[0]?.capturedAt ?? null,
    typeof meta.updated_at === "string" ? Date.parse(meta.updated_at) : null,
  ].filter((value): value is number => typeof value === "number" && Number.isFinite(value));

  return {
    id: `${context.worktreePath}::${slug}`,
    slug,
    directory: logDir,
    worktreePath: context.worktreePath,
    worktree: context.worktree,
    worktreeShort: worktreeShort(context.worktree),
    title: metaTitle || humanize(slug),
    description: firstString(meta, "description") ?? "",
    status: firstString(meta, "status") ?? runs[0]?.status ?? null,
    updatedAt: candidates.length ? Math.max(...candidates) : Date.now(),
    promptPath: hasPrompt ? promptPath : null,
    runs,
  };
}

export async function loadFrictionLogs(
  frictionDir: string,
  context: { worktreePath: string; worktree: string },
  hashes: HashCache,
): Promise<FrictionLog[]> {
  let entries: fs.Dirent[];
  try {
    entries = await fs.promises.readdir(frictionDir, { withFileTypes: true });
  } catch {
    return [];
  }
  const logs: FrictionLog[] = [];
  for (const entry of entries) {
    if (entry.name.startsWith(".") || !(await isDirectoryEntry(frictionDir, entry))) continue;
    const log = await loadFrictionLog(path.join(frictionDir, entry.name), context, hashes);
    if (log) logs.push(log);
  }
  return logs.sort((a, b) => b.updatedAt - a.updatedAt);
}

/** `MMM d · HH:mm` in local time for `yyyyMMddTHHmmssZ(-N)` run ids. */
export function runDisplayTitle(runId: string): string {
  const match = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z(?:-\d+)?$/.exec(runId);
  if (match) {
    const date = new Date(
      Date.UTC(
        Number(match[1]),
        Number(match[2]) - 1,
        Number(match[3]),
        Number(match[4]),
        Number(match[5]),
        Number(match[6]),
      ),
    );
    const month = date.toLocaleString("en-US", { month: "short" });
    const hours = String(date.getHours()).padStart(2, "0");
    const minutes = String(date.getMinutes()).padStart(2, "0");
    return `${month} ${date.getDate()} · ${hours}:${minutes}`;
  }
  return runId.length > 16 ? runId.slice(-14) : runId;
}

export function stepCountLabel(count: number): string {
  return count === 1 ? "1 step" : `${count} steps`;
}

export function frictionStatusLabel(status: string | null): string | null {
  switch ((status ?? "").toLowerCase()) {
    case "draft":
      return "Draft";
    case "ready":
      return "Ready";
    case "running":
      return "Running";
    case "complete":
    case "completed":
    case "done":
      return "Complete";
    case "failed":
    case "fail":
    case "error":
      return "Failed";
    default:
      return status ? humanize(status) : null;
  }
}
