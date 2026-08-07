import fs from "node:fs";
import path from "node:path";

import {
  assertKebabCase,
  assertSlug,
  ensureDir,
  featureDir,
  humanize,
  nextSequence,
} from "./paths.js";
import type { SinkMovieRequest, SinkMovieResult } from "./types.js";

interface ManifestShot {
  id: string;
  file: string;
  slug: string;
  title: string;
  description?: string;
  captured_at: string;
  viewport?: string;
  kind: "movie";
  video: string;
  duration_ms: number;
  source: string;
  chapters: { slug: string; t_ms: number; note?: string }[];
}

interface Manifest {
  version: 1;
  feature: string;
  run_id: string;
  status: string;
  description?: string;
  shots: ManifestShot[];
}

function readManifest(manifestPath: string): Manifest | null {
  if (!fs.existsSync(manifestPath)) return null;
  const raw = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as Manifest;
  if (!raw || typeof raw !== "object") return null;
  if (!Array.isArray(raw.shots)) raw.shots = [];
  return raw;
}

function writeAtomic(filePath: string, contents: string): void {
  const dir = path.dirname(filePath);
  ensureDir(dir);
  const tmp = path.join(
    dir,
    `.${path.basename(filePath)}.tmp.${process.pid}.${Date.now()}`,
  );
  fs.writeFileSync(tmp, contents);
  fs.renameSync(tmp, filePath);
}

/**
 * Publish poster + video into .astroshot/<feature>/ and append a movie shot
 * to manifest.json. Poster basename is the review key (matches stills).
 */
export function sinkMovie(request: SinkMovieRequest): SinkMovieResult {
  assertKebabCase(request.feature, "feature");
  assertSlug(request.slug);

  const dir = featureDir(request.root, request.feature);
  ensureDir(dir);
  const sequence = nextSequence(dir);
  const posterName = `${sequence}-${request.slug}.png`;
  const videoExt = path.extname(request.videoPath).toLowerCase() || ".webm";
  const videoName = `${sequence}-${request.slug}${videoExt}`;
  const posterDest = path.join(dir, posterName);
  const videoDest = path.join(dir, videoName);

  if (!fs.existsSync(request.posterPath)) {
    throw new Error(`poster not found: ${request.posterPath}`);
  }
  if (!fs.existsSync(request.videoPath)) {
    throw new Error(`video not found: ${request.videoPath}`);
  }

  fs.copyFileSync(request.posterPath, posterDest);
  fs.copyFileSync(request.videoPath, videoDest);

  const manifestPath = path.join(dir, "manifest.json");
  const existing = readManifest(manifestPath);
  const continueRun =
    existing &&
    existing.run_id === request.runId &&
    existing.status === "running";

  const shot: ManifestShot = {
    id: sequence,
    file: posterName,
    slug: request.slug,
    title: request.title ?? humanize(request.slug),
    description: request.description,
    captured_at: new Date().toISOString(),
    viewport: request.size
      ? `${request.size.width}x${request.size.height}`
      : undefined,
    kind: "movie",
    video: videoName,
    duration_ms: Math.round(request.durationMs),
    source: request.source,
    chapters: request.chapters.map((chapter) => ({
      slug: chapter.slug,
      t_ms: Math.round(chapter.tMs),
      note: chapter.note,
    })),
  };

  const manifest: Manifest = continueRun
    ? {
        ...existing,
        status: request.status ?? existing.status,
        description: request.description ?? existing.description,
        shots: [...existing.shots, shot],
      }
    : {
        version: 1,
        feature: request.feature,
        run_id: request.runId,
        status: request.status ?? "running",
        description: request.description,
        shots: [shot],
      };

  writeAtomic(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

  return {
    sequence,
    posterDest,
    videoDest,
    manifestPath,
    featureDir: dir,
  };
}

export function finalizeManifest(
  root: string,
  feature: string,
  runId: string,
  status: "pass" | "fail" | "idle" | "running",
): void {
  assertKebabCase(feature, "feature");
  const dir = featureDir(root, feature);
  const manifestPath = path.join(dir, "manifest.json");
  const existing = readManifest(manifestPath);
  if (!existing) {
    writeAtomic(
      manifestPath,
      `${JSON.stringify(
        {
          version: 1,
          feature,
          run_id: runId,
          status,
          shots: [],
        },
        null,
        2,
      )}\n`,
    );
    return;
  }
  if (existing.run_id !== runId) {
    throw new Error(
      `manifest run_id ${JSON.stringify(existing.run_id)} does not match ${JSON.stringify(runId)}`,
    );
  }
  existing.status = status;
  writeAtomic(manifestPath, `${JSON.stringify(existing, null, 2)}\n`);
}
