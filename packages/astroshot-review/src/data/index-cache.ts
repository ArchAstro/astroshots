/**
 * Durable index so the tray opens instantly: known `.astroshot` directories,
 * the newest-first arrival order, and memoized image hashes.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import type { HashRecord } from "./hash-cache.js";

export interface IndexDocument {
  version: 1;
  roots: string[];
  astroshotDirs: string[];
  arrivalOrder: string[];
  hashes: Record<string, HashRecord>;
  updatedAt: string;
  /** When the last deep walk of every root finished (ISO), if ever. */
  fullScanAt?: string;
}

export function indexCachePath(env: NodeJS.ProcessEnv = process.env): string {
  if (env.ASTROSHOT_REVIEW_CACHE_DIR) return path.join(env.ASTROSHOT_REVIEW_CACHE_DIR, "index.json");
  const base = env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
  return path.join(base, "astroshot-review", "index.json");
}

function normalizedRoots(roots: string[]): string[] {
  return [...roots].sort();
}

export async function loadIndex(roots: string[], filePath = indexCachePath()): Promise<IndexDocument | null> {
  let raw: string;
  try {
    raw = await fs.promises.readFile(filePath, "utf8");
  } catch {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as Partial<IndexDocument>;
    if (parsed.version !== 1 || !Array.isArray(parsed.roots)) return null;
    if (JSON.stringify(normalizedRoots(parsed.roots)) !== JSON.stringify(normalizedRoots(roots))) return null;
    return {
      version: 1,
      roots: parsed.roots,
      astroshotDirs: Array.isArray(parsed.astroshotDirs) ? parsed.astroshotDirs : [],
      arrivalOrder: Array.isArray(parsed.arrivalOrder) ? parsed.arrivalOrder : [],
      hashes: parsed.hashes && typeof parsed.hashes === "object" ? parsed.hashes : {},
      updatedAt: typeof parsed.updatedAt === "string" ? parsed.updatedAt : "",
      fullScanAt: typeof parsed.fullScanAt === "string" ? parsed.fullScanAt : undefined,
    };
  } catch {
    return null;
  }
}

export async function saveIndex(document: IndexDocument, filePath = indexCachePath()): Promise<void> {
  await fs.promises.mkdir(path.dirname(filePath), { recursive: true });
  const temp = `${filePath}.${process.pid}.tmp`;
  await fs.promises.writeFile(temp, JSON.stringify(document));
  await fs.promises.rename(temp, filePath);
}

/**
 * Keep known paths in their prior relative order; new paths go first, sorted
 * newest capture first. Paths that vanished are dropped.
 */
export function reconcileArrivalOrder(
  previous: string[],
  shots: Array<{ path: string; capturedAt: number }>,
): string[] {
  const present = new Map(shots.map((shot) => [shot.path, shot.capturedAt]));
  const kept = previous.filter((entry) => present.has(entry));
  const known = new Set(kept);
  const fresh = shots
    .filter((shot) => !known.has(shot.path))
    .sort((a, b) => b.capturedAt - a.capturedAt)
    .map((shot) => shot.path);
  return [...fresh, ...kept];
}
