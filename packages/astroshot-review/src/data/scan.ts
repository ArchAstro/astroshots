/**
 * Discover `.astroshot` trees under the watch roots and load their shots.
 */
import fs from "node:fs";
import path from "node:path";

import { loadFrictionLogs } from "./friction.js";
import type { HashCache } from "./hash-cache.js";
import {
  chaptersOf,
  matchManifestShot,
  parseFeatureStatus,
  parseIsoDate,
  readManifest,
  type FeatureManifest,
} from "./manifest.js";
import type { AstroshotTree, ReviewSnapshot, Shot } from "./model.js";
import {
  ASTROSHOT_DIR,
  FRICTION_DIR,
  MAX_SCAN_DEPTH,
  SKIP_DIRECTORIES,
  VIDEO_EXTENSIONS,
  frictionLogsDir,
  humanize,
  isImageFile,
  sequenceAndSlug,
  worktreeShort,
} from "./paths.js";
import {
  entryNeedsHash,
  readReviewDocument,
  scopedEntry,
  snapshotFromEntry,
  type ReviewDocument,
} from "./review-store.js";

export interface FindOptions {
  maxDepth?: number;
  skip?: Set<string>;
  concurrency?: number;
  onFound?: (astroshotDir: string) => void;
  signal?: AbortSignal;
}

/** Breadth-first walk with bounded concurrency; never descends into a found tree. */
export async function findAstroshotDirs(roots: string[], options: FindOptions = {}): Promise<string[]> {
  const maxDepth = options.maxDepth ?? MAX_SCAN_DEPTH;
  const skip = options.skip ?? SKIP_DIRECTORIES;
  const concurrency = options.concurrency ?? 16;
  const found: string[] = [];
  const queue: Array<{ dir: string; depth: number }> = roots.map((root) => ({ dir: root, depth: 0 }));
  let active = 0;

  await new Promise<void>((resolve) => {
    const pump = () => {
      if (options.signal?.aborted) {
        if (active === 0) resolve();
        return;
      }
      while (active < concurrency && queue.length > 0) {
        const item = queue.shift()!;
        active += 1;
        void visit(item.dir, item.depth).finally(() => {
          active -= 1;
          if (queue.length === 0 && active === 0) resolve();
          else pump();
        });
      }
      if (queue.length === 0 && active === 0) resolve();
    };
    const visit = async (dir: string, depth: number) => {
      let entries: fs.Dirent[];
      try {
        entries = await fs.promises.readdir(dir, { withFileTypes: true });
      } catch {
        return;
      }
      for (const entry of entries) {
        const name = entry.name;
        // Follow a linked `.astroshot` itself, but never descend through other
        // symlinks: they can loop and are rarely where captures live.
        if (!entry.isDirectory() && !(name === ASTROSHOT_DIR && (await kindOf(dir, entry)) === "dir")) continue;
        if (name === ASTROSHOT_DIR) {
          const astroshotDir = path.join(dir, name);
          found.push(astroshotDir);
          options.onFound?.(astroshotDir);
          continue;
        }
        if (name.startsWith(".") || skip.has(name)) continue;
        if (depth + 1 > maxDepth) continue;
        queue.push({ dir: path.join(dir, name), depth: depth + 1 });
      }
    };
    pump();
  });
  return found.sort();
}

export interface ShotContext {
  worktreePath: string;
  worktree: string;
  feature: string;
  featureDir: string;
}

interface FeatureListing {
  files: Set<string>;
}

/** Dirent kinds with symlinks resolved, so linked trees behave like real ones. */
async function kindOf(parent: string, entry: fs.Dirent): Promise<"file" | "dir" | "other"> {
  if (entry.isFile()) return "file";
  if (entry.isDirectory()) return "dir";
  if (!entry.isSymbolicLink()) return "other";
  try {
    const stat = await fs.promises.stat(path.join(parent, entry.name));
    return stat.isFile() ? "file" : stat.isDirectory() ? "dir" : "other";
  } catch {
    return "other";
  }
}

async function listFeature(featureDir: string): Promise<FeatureListing | null> {
  try {
    const entries = await fs.promises.readdir(featureDir, { withFileTypes: true });
    const files = new Set<string>();
    for (const entry of entries) {
      if ((await kindOf(featureDir, entry)) === "file") files.add(entry.name);
    }
    return { files };
  } catch {
    return null;
  }
}

function resolveVideo(entryVideo: string | undefined, fileName: string, files: Set<string>): string | null {
  if (entryVideo && entryVideo.trim()) return entryVideo;
  const stem = fileName.replace(/\.[^.]+$/, "");
  for (const extension of VIDEO_EXTENSIONS) {
    const candidate = `${stem}.${extension}`;
    if (files.has(candidate)) return candidate;
  }
  return null;
}

async function reviewFor(
  document: ReviewDocument | null,
  fileName: string,
  runId: string | null,
  imagePath: string,
  stat: fs.Stats,
  hashes: HashCache,
): Promise<ReviewSnapshot | null> {
  if (!document) return null;
  const entry = scopedEntry(document, fileName, runId);
  let sha: string | null = null;
  if (entryNeedsHash(entry)) {
    try {
      sha = await hashes.hash(imagePath, stat);
    } catch {
      sha = null;
    }
  }
  return snapshotFromEntry(entry, sha);
}

export async function buildShot(
  imagePath: string,
  context: ShotContext,
  manifest: FeatureManifest | null,
  review: ReviewDocument | null,
  listing: FeatureListing,
  hashes: HashCache,
): Promise<Shot | null> {
  let stat: fs.Stats;
  try {
    stat = await fs.promises.stat(imagePath);
  } catch {
    return null;
  }
  const fileName = path.basename(imagePath);
  const entry = matchManifestShot(manifest, fileName);
  const parsed = sequenceAndSlug(fileName);
  const slug = entry?.slug ?? parsed.slug;
  const runId = manifest?.run_id ?? null;
  const videoFileName = resolveVideo(entry?.video, fileName, listing.files);
  const videoExists = videoFileName !== null && listing.files.has(videoFileName);
  const durationMs =
    typeof entry?.duration_ms === "number" && entry.duration_ms > 0 ? entry.duration_ms : null;
  return {
    id: imagePath,
    path: imagePath,
    fileName,
    worktreePath: context.worktreePath,
    worktree: context.worktree,
    worktreeShort: worktreeShort(context.worktree),
    feature: context.feature,
    featureDir: context.featureDir,
    sequence: parsed.sequence ?? entry?.id ?? null,
    slug,
    title: entry?.title ?? humanize(slug),
    description: entry?.description ?? "",
    url: entry?.url ?? null,
    runId,
    status: parseFeatureStatus(manifest?.status),
    capturedAt: parseIsoDate(entry?.captured_at) ?? stat.mtimeMs,
    mtimeMs: stat.mtimeMs,
    isMovie: (entry?.kind ?? "").toLowerCase() === "movie" || videoFileName !== null,
    videoFileName,
    videoPath: videoExists ? path.join(context.featureDir, videoFileName!) : null,
    durationMs,
    source: entry?.source ?? null,
    chapters: chaptersOf(entry),
    review: await reviewFor(review, fileName, runId, imagePath, stat, hashes),
  };
}

async function readReviewSafely(directory: string): Promise<ReviewDocument | null> {
  try {
    return await readReviewDocument(directory);
  } catch {
    return null;
  }
}

/** Every shot inside one feature directory, in directory order. */
export async function scanFeatureDir(
  featureDir: string,
  context: { worktreePath: string; worktree: string },
  hashes: HashCache,
): Promise<Shot[]> {
  const listing = await listFeature(featureDir);
  if (!listing) return [];
  const feature = path.basename(featureDir);
  const [manifest, review] = await Promise.all([readManifest(featureDir), readReviewSafely(featureDir)]);
  const shotContext: ShotContext = { ...context, feature, featureDir };
  const shots: Shot[] = [];
  for (const name of listing.files) {
    if (!isImageFile(name)) continue;
    const shot = await buildShot(path.join(featureDir, name), shotContext, manifest, review, listing, hashes);
    if (shot) shots.push(shot);
  }
  return shots;
}

/** Re-read one shot in place (after its image or sidecars changed). */
export async function rebuildShot(
  imagePath: string,
  context: ShotContext,
  hashes: HashCache,
): Promise<Shot | null> {
  const listing = await listFeature(context.featureDir);
  if (!listing) return null;
  const [manifest, review] = await Promise.all([
    readManifest(context.featureDir),
    readReviewSafely(context.featureDir),
  ]);
  return buildShot(imagePath, context, manifest, review, listing, hashes);
}

export async function scanTree(astroshotDir: string, hashes: HashCache): Promise<AstroshotTree> {
  const worktreePath = path.dirname(astroshotDir);
  const worktree = path.basename(worktreePath);
  const context = { worktreePath, worktree };
  let entries: fs.Dirent[] = [];
  try {
    entries = await fs.promises.readdir(astroshotDir, { withFileTypes: true });
  } catch {
    entries = [];
  }
  const shots: Shot[] = [];
  for (const entry of entries) {
    if (entry.name.startsWith(".") || entry.name === FRICTION_DIR) continue;
    if ((await kindOf(astroshotDir, entry)) !== "dir") continue;
    shots.push(...(await scanFeatureDir(path.join(astroshotDir, entry.name), context, hashes)));
  }
  const frictionLogs = await loadFrictionLogs(frictionLogsDir(astroshotDir), context, hashes);
  return { astroshotDir, worktreePath, worktree, shots, frictionLogs };
}
