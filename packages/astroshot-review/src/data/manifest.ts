import fs from "node:fs";
import path from "node:path";

import type { Chapter, FeatureStatus } from "./model.js";
import { sequenceAndSlug } from "./paths.js";

export interface ManifestChapter {
  slug?: string;
  title?: string;
  t_ms?: number;
  tMs?: number;
}

export interface ManifestShot {
  id?: string;
  file?: string;
  slug?: string;
  title?: string;
  description?: string;
  captured_at?: string;
  url?: string;
  viewport?: unknown;
  kind?: string;
  video?: string;
  poster?: string;
  duration_ms?: number;
  source?: string;
  chapters?: ManifestChapter[];
}

export interface FeatureManifest {
  version?: number;
  feature?: string;
  run_id?: string;
  status?: string;
  description?: string;
  shots?: ManifestShot[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export async function readManifest(featureDir: string): Promise<FeatureManifest | null> {
  let raw: string;
  try {
    raw = await fs.promises.readFile(path.join(featureDir, "manifest.json"), "utf8");
  } catch {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) return null;
    const manifest = parsed as FeatureManifest;
    if (manifest.shots !== undefined && !Array.isArray(manifest.shots)) manifest.shots = [];
    return manifest;
  } catch {
    return null;
  }
}

/** First match wins: file, poster, id (sequence), slug. */
export function matchManifestShot(
  manifest: FeatureManifest | null,
  fileName: string,
): ManifestShot | null {
  const shots = manifest?.shots ?? [];
  if (shots.length === 0) return null;
  const { sequence, slug } = sequenceAndSlug(fileName);
  return (
    shots.find((shot) => shot.file === fileName) ??
    shots.find((shot) => shot.poster === fileName) ??
    (sequence ? shots.find((shot) => shot.id === sequence) : undefined) ??
    shots.find((shot) => shot.slug === slug) ??
    null
  );
}

export function parseFeatureStatus(raw: string | undefined | null): FeatureStatus | null {
  switch ((raw ?? "").toLowerCase()) {
    case "running":
    case "run":
    case "in_progress":
    case "in-progress":
      return "running";
    case "pass":
    case "passed":
    case "ok":
    case "success":
      return "pass";
    case "fail":
    case "failed":
    case "error":
      return "fail";
    case "idle":
    case "pending":
      return "idle";
    default:
      return null;
  }
}

export function parseIsoDate(value: string | undefined | null): number | null {
  if (!value) return null;
  const time = Date.parse(value);
  return Number.isFinite(time) ? time : null;
}

export function chaptersOf(entry: ManifestShot | null): Chapter[] {
  if (!entry?.chapters || !Array.isArray(entry.chapters)) return [];
  return entry.chapters.filter(isRecord).map((chapter) => ({
    slug: typeof chapter.slug === "string" ? chapter.slug : undefined,
    title: typeof chapter.title === "string" ? chapter.title : undefined,
    tMs:
      typeof chapter.t_ms === "number"
        ? chapter.t_ms
        : typeof chapter.tMs === "number"
          ? chapter.tMs
          : undefined,
  }));
}

/** `2.8s` under a minute, `m:ss` above; null when unknown. */
export function durationLabel(durationMs: number | null | undefined): string | null {
  if (durationMs === null || durationMs === undefined || durationMs <= 0) return null;
  const seconds = durationMs / 1000;
  if (seconds < 60) {
    const rounded = Math.round(seconds * 10) / 10;
    return Number.isInteger(rounded) ? `${rounded}s` : `${rounded.toFixed(1)}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const rest = Math.floor(seconds % 60);
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

/** Chapter timestamp label used by the detail card. */
export function chapterTimeLabel(tMs: number | undefined): string {
  if (tMs === undefined || tMs < 0) return "—";
  const seconds = tMs / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = Math.floor(seconds % 60);
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}
