import fs from "node:fs";
import path from "node:path";

import { collectProvenance, newStableId, type CaptureProvenance } from "./identity.js";
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
  entry_id: string;
  asset_id: string;
  video_asset_id: string;
}

interface PreviousRun {
  run_id: string;
  status: string;
  provenance?: CaptureProvenance;
  shots: ManifestShot[];
}

interface Manifest {
  version: 1;
  schema_version: 1;
  feature: string;
  scenario_id: string;
  run_id: string;
  status: string;
  description?: string;
  provenance?: CaptureProvenance;
  shots: ManifestShot[];
  previous_runs?: PreviousRun[];
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
  const manifestPath = path.join(dir, "manifest.json");
  const existing = readManifest(manifestPath);
  // The same run id always continues that attempt; a different id starts a new
  // attempt and must not reuse an id the feature has already recorded.
  const continueRun = Boolean(existing && existing.run_id === request.runId);
  if (
    existing &&
    !continueRun &&
    (existing.previous_runs ?? []).some((run) => run.run_id === request.runId)
  ) {
    throw new Error(
      `run id ${request.runId} was already recorded for feature ${request.feature}; pick a new run id`,
    );
  }
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
    entry_id: newStableId(),
    asset_id: newStableId(),
    video_asset_id: newStableId(),
  };

  const provenance = collectProvenance(dir);
  const scenarioId = existing?.scenario_id ?? newStableId();
  const archived: PreviousRun[] =
    existing && !continueRun && existing.run_id
      ? [
          ...(existing.previous_runs ?? []),
          {
            run_id: existing.run_id,
            status: existing.status,
            ...(existing.provenance ? { provenance: existing.provenance } : {}),
            shots: existing.shots,
          },
        ]
      : (existing?.previous_runs ?? []);
  const manifest: Manifest =
    existing && continueRun
      ? {
          ...existing,
          schema_version: 1,
          scenario_id: scenarioId,
          status: request.status ?? existing.status,
          description: request.description ?? existing.description,
          provenance: existing.provenance ?? provenance,
          shots: [...existing.shots, shot],
        }
      : {
          version: 1,
          schema_version: 1,
          feature: request.feature,
          scenario_id: scenarioId,
          run_id: request.runId,
          status: request.status ?? "running",
          description: request.description,
          provenance,
          shots: [shot],
          ...(archived.length > 0 ? { previous_runs: archived } : {}),
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
          schema_version: 1,
          feature,
          scenario_id: newStableId(),
          run_id: runId,
          status,
          provenance: collectProvenance(dir),
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
