/**
 * Normalize on-disk captures into versioned portable records.
 *
 * Legacy `manifest.json` / `log.jsonl` / `review.json` stay readable. Additive
 * IDs travel with copies; path and git metadata are provenance only. Import
 * reports malformed lines and missing assets instead of inventing them.
 */
import fs from "node:fs";
import path from "node:path";

import { contentRevision, collectProvenance, missingAssetDigest, newStableId, provenanceFields } from "./identity.js";
import {
  chaptersOf,
  matchManifestShot,
  readManifest,
  type FeatureManifest,
  type ManifestShot,
} from "./manifest.js";
import type {
  CaptureProvenance,
  ImportDiagnostic,
  PortableAsset,
  PortableEntry,
  PortablePromptSnapshot,
  PortableRecord,
  PortableReviewClaim,
  PortableRun,
} from "./model.js";
import { PORTABLE_SCHEMA_VERSION } from "./model.js";
import { humanize, isImageFile, isInsideFrictionLogs, sequenceAndSlug } from "./paths.js";
import {
  LOG_FILE,
  META_FILE,
  PROMPT_FILE,
  RUNS_DIR,
  parseJsonlDetailed,
  type JsonlParseResult,
} from "./friction.js";
import {
  readReviewDocument,
  scopedEntry,
  sha256File,
  type ReviewDocument,
} from "./review-store.js";

export const PROMPT_SNAPSHOT_FILE = "prompt.md";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function asBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

async function exists(filePath: string): Promise<boolean> {
  try {
    await fs.promises.access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function readJsonRecord(filePath: string): Promise<Record<string, unknown> | null> {
  try {
    const parsed: unknown = JSON.parse(await fs.promises.readFile(filePath, "utf8"));
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function provenanceFrom(record: Record<string, unknown> | null, sourcePath: string, worktree: string): CaptureProvenance {
  const stored = record && isRecord(record.provenance) ? record.provenance : null;
  return {
    sourcePath: asString(stored?.source_path) ?? sourcePath,
    worktree: asString(stored?.worktree) ?? worktree,
    repo: asString(stored?.repo),
    commit: asString(stored?.commit),
    branch: asString(stored?.branch),
    dirty: asBoolean(stored?.dirty),
    taskId: asString(stored?.task_id),
    pr: asString(stored?.pr),
  };
}

async function hashIfPresent(filePath: string): Promise<{ sha256: string; byteLength: number } | null> {
  try {
    const stat = await fs.promises.stat(filePath);
    if (!stat.isFile()) return null;
    return { sha256: await sha256File(filePath), byteLength: stat.size };
  } catch {
    return null;
  }
}

async function assetOf(
  directory: string,
  fileName: string,
  role: PortableAsset["role"],
  assetId: string | null,
  diagnostics: ImportDiagnostic[],
): Promise<PortableAsset> {
  const target = path.join(directory, path.basename(fileName));
  const hashed = await hashIfPresent(target);
  if (!hashed) {
    diagnostics.push({
      kind: "missing-asset",
      message: `missing ${role} ${fileName}`,
      fileName,
      path: target,
    });
    return {
      assetId,
      role,
      fileName: path.basename(fileName),
      path: null,
      sha256: null,
      missing: true,
      byteLength: null,
    };
  }
  return {
    assetId,
    role,
    fileName: path.basename(fileName),
    path: target,
    sha256: hashed.sha256,
    missing: false,
    byteLength: hashed.byteLength,
  };
}

async function reviewClaim(
  directory: string,
  fileName: string,
  runId: string | null,
  currentSha: string | null,
  computedRevision: string,
): Promise<PortableReviewClaim | null> {
  let document: ReviewDocument | null;
  try {
    document = await readReviewDocument(directory);
  } catch {
    return null;
  }
  if (!document) return null;
  const entry = scopedEntry(document, fileName, runId);
  if (!entry) {
    const empty = Object.keys(document.reviews).length === 0 && !document.run_id;
    return empty ? null : {
      comments: [],
      legacySha256: null,
      legacyHashMatches: null,
      contentRevisionClaim: null,
      contentRevisionMatches: null,
    };
  }
  const comments = (entry.comments ?? []).map((comment) => ({
    id: comment.id,
    body: comment.body,
    createdAt: comment.created_at,
  }));
  const legacySha256 = entry.image_sha256 ?? null;
  const contentRevisionClaim = entry.content_revision ?? null;
  return {
    comments,
    legacySha256,
    legacyHashMatches: legacySha256 ? legacySha256 === currentSha : null,
    contentRevisionClaim,
    contentRevisionMatches: contentRevisionClaim ? contentRevisionClaim === computedRevision : null,
  };
}

function runRevision(entries: PortableEntry[], promptSha256: string | null): string {
  return contentRevision({
    promptSha256,
    entries: entries.map((entry) => ({
      title: entry.title,
      description: entry.description,
      transcript: entry.transcript,
      assetDigests: entry.assets.map((asset) => asset.sha256 ?? missingAssetDigest(asset.fileName)),
    })),
  });
}

async function listImageFiles(directory: string): Promise<string[]> {
  try {
    const entries = await fs.promises.readdir(directory);
    return entries.filter((name) => isImageFile(name) && !name.startsWith(".")).sort();
  } catch {
    return [];
  }
}

function shotTitle(entry: ManifestShot | null, fileName: string): string {
  if (entry?.title) return entry.title;
  return humanize(entry?.slug ?? sequenceAndSlug(fileName).slug);
}

async function entryFromShot(
  featureDir: string,
  fileName: string,
  order: number,
  manifest: FeatureManifest | null,
  diagnostics: ImportDiagnostic[],
): Promise<PortableEntry> {
  const entry = matchManifestShot(manifest, fileName);
  const parsed = sequenceAndSlug(fileName);
  const assets: PortableAsset[] = [];
  const isMovie = (entry?.kind ?? "").toLowerCase() === "movie" || Boolean(entry?.video);
  const imageRole = isMovie ? "poster" : "image";
  assets.push(await assetOf(featureDir, fileName, imageRole, asString(entry?.asset_id), diagnostics));
  if (entry?.video) {
    assets.push(await assetOf(featureDir, entry.video, "video", asString(entry.video_asset_id), diagnostics));
  }
  return {
    entryId: asString(entry?.entry_id),
    order,
    slug: entry?.slug ?? parsed.slug,
    title: shotTitle(entry, fileName),
    description: entry?.description ?? "",
    transcript: "",
    capturedAt: entry?.captured_at ?? null,
    assets,
    chapters: chaptersOf(entry),
    review: null,
  };
}

export async function importShotRecord(featureDir: string): Promise<PortableRecord> {
  const diagnostics: ImportDiagnostic[] = [];
  const resolved = path.resolve(featureDir);
  const slug = path.basename(resolved);
  const worktree = path.basename(path.dirname(path.dirname(resolved)));
  const manifest = await readManifest(resolved);
  const scenarioId = asString(manifest?.scenario_id);
  if (!scenarioId) {
    diagnostics.push({
      kind: "ambiguous-identity",
      message: `legacy shot feature ${slug} has no scenario_id; path/slug are not a global identity`,
      path: resolved,
    });
  }

  const orderedFiles = (manifest?.shots ?? [])
    .map((shot) => shot.file ?? shot.poster)
    .filter((name): name is string => Boolean(name));
  const files = orderedFiles.length > 0 ? orderedFiles : await listImageFiles(resolved);
  const entries: PortableEntry[] = [];
  for (const [index, fileName] of files.entries()) {
    entries.push(await entryFromShot(resolved, fileName, index + 1, manifest, diagnostics));
  }

  const runId = asString(manifest?.run_id) ?? "latest";
  const revision = runRevision(entries, null);
  const posterName = entries[0]?.assets[0]?.fileName ?? null;
  const posterSha = entries[0]?.assets[0]?.sha256 ?? null;
  const runReview = posterName ? await reviewClaim(resolved, posterName, asString(manifest?.run_id), posterSha, revision) : null;
  if (entries[0]) entries[0].review = runReview;

  const run: PortableRun = {
    runId,
    capturedAt: entries[0]?.capturedAt ?? null,
    status: asString(manifest?.status),
    promptSnapshot: null,
    contentRevision: revision,
    entries,
    review: runReview,
  };

  return {
    schemaVersion: PORTABLE_SCHEMA_VERSION,
    kind: "shot",
    identity: scenarioId ? "identified" : "ambiguous-legacy",
    scenarioId,
    slug,
    title: asString(manifest?.feature) ?? humanize(slug),
    description: asString(manifest?.description) ?? "",
    provenance: provenanceFrom(manifest as Record<string, unknown> | null, resolved, worktree),
    runs: [run],
    diagnostics,
  };
}

async function promptSnapshot(
  scenarioDir: string,
  runDir: string,
  diagnostics: ImportDiagnostic[],
): Promise<PortablePromptSnapshot | null> {
  const frozenPath = path.join(runDir, PROMPT_SNAPSHOT_FILE);
  const livePath = path.join(scenarioDir, PROMPT_FILE);
  if (await exists(frozenPath)) {
    const hashed = await hashIfPresent(frozenPath);
    return { path: frozenPath, sha256: hashed?.sha256 ?? null, frozen: true };
  }
  if (await exists(livePath)) {
    diagnostics.push({
      kind: "unfrozen-prompt",
      message: "run has no prompt snapshot; live prompt.md can change after the attempt",
      path: livePath,
    });
    const hashed = await hashIfPresent(livePath);
    return { path: livePath, sha256: hashed?.sha256 ?? null, frozen: false };
  }
  return null;
}

async function entriesFromJsonl(
  parsed: JsonlParseResult,
  runDir: string,
  diagnostics: ImportDiagnostic[],
): Promise<PortableEntry[]> {
  diagnostics.push(...parsed.diagnostics);
  const entries: PortableEntry[] = [];
  for (const step of parsed.steps) {
    const assets: PortableAsset[] = [];
    for (const fileName of step.referencedScreenshots) {
      const existing = step.screenshots.find((filePath) => path.basename(filePath) === path.basename(fileName));
      if (existing) {
        const hashed = await hashIfPresent(existing);
        assets.push({
          assetId: step.assetIds[fileName] ?? null,
          role: "screenshot",
          fileName: path.basename(fileName),
          path: existing,
          sha256: hashed?.sha256 ?? null,
          missing: !hashed,
          byteLength: hashed?.byteLength ?? null,
        });
      } else {
        assets.push(await assetOf(runDir, fileName, "screenshot", step.assetIds[fileName] ?? null, diagnostics));
      }
    }
    entries.push({
      entryId: step.entryId,
      order: step.step,
      slug: step.stepId,
      title: step.title,
      description: step.description,
      transcript: step.transcript,
      capturedAt: step.capturedAt,
      assets,
      chapters: [],
      review: null,
    });
  }
  return entries.sort((a, b) => a.order - b.order);
}

async function importFrictionRun(
  scenarioDir: string,
  runDir: string,
  runId: string,
  diagnostics: ImportDiagnostic[],
): Promise<PortableRun | null> {
  const logPath = path.join(runDir, LOG_FILE);
  const hasLog = await exists(logPath);
  let parsed: JsonlParseResult = { steps: [], diagnostics: [], referenced: [] };
  if (hasLog) {
    parsed = await parseJsonlDetailed(await fs.promises.readFile(logPath, "utf8"), runDir);
  } else {
    const images = await listImageFiles(runDir);
    if (images.length === 0) return null;
  }
  const entries = hasLog
    ? await entriesFromJsonl(parsed, runDir, diagnostics)
    : await Promise.all(
        (await listImageFiles(runDir)).map(async (fileName, index) => {
          const asset = await assetOf(runDir, fileName, "screenshot", null, diagnostics);
          return {
            entryId: null,
            order: index + 1,
            slug: sequenceAndSlug(fileName).slug,
            title: humanize(sequenceAndSlug(fileName).slug),
            description: "",
            transcript: "",
            capturedAt: null,
            assets: [asset],
            chapters: [],
            review: null,
          } satisfies PortableEntry;
        }),
      );
  if (entries.length === 0 && !hasLog) return null;

  const meta = await readJsonRecord(path.join(runDir, META_FILE));
  const snapshot = await promptSnapshot(scenarioDir, runDir, diagnostics);
  const revision = runRevision(entries, snapshot?.sha256 ?? null);
  const logSha = hasLog ? (await hashIfPresent(logPath))?.sha256 ?? null : null;
  const review = hasLog ? await reviewClaim(runDir, LOG_FILE, runId, logSha, revision) : null;
  return {
    runId: asString(meta?.run_id) ?? runId,
    capturedAt: asString(meta?.captured_at) ?? entries[0]?.capturedAt ?? null,
    status: asString(meta?.status),
    promptSnapshot: snapshot,
    contentRevision: revision,
    entries,
    review,
  };
}

export async function importFrictionRecord(scenarioDir: string): Promise<PortableRecord> {
  const diagnostics: ImportDiagnostic[] = [];
  const resolved = path.resolve(scenarioDir);
  const slug = path.basename(resolved);
  const worktree = path.basename(path.dirname(path.dirname(path.dirname(resolved))));
  const meta = await readJsonRecord(path.join(resolved, META_FILE));
  const scenarioId = asString(meta?.scenario_id);
  if (!scenarioId) {
    diagnostics.push({
      kind: "ambiguous-identity",
      message: `legacy friction scenario ${slug} has no scenario_id; copies must not be merged by slug`,
      path: resolved,
    });
  }

  const runs: PortableRun[] = [];
  const runsDir = path.join(resolved, RUNS_DIR);
  let runEntries: fs.Dirent[] = [];
  try {
    runEntries = await fs.promises.readdir(runsDir, { withFileTypes: true });
  } catch {
    runEntries = [];
  }
  for (const entry of runEntries) {
    if (entry.name.startsWith(".")) continue;
    const runDir = path.join(runsDir, entry.name);
    let isDir = entry.isDirectory();
    if (entry.isSymbolicLink()) {
      try {
        isDir = (await fs.promises.stat(runDir)).isDirectory();
      } catch {
        isDir = false;
      }
    }
    if (!isDir) continue;
    const run = await importFrictionRun(resolved, runDir, entry.name, diagnostics);
    if (run) runs.push(run);
  }
  if (runs.length === 0 && (await exists(path.join(resolved, LOG_FILE)))) {
    const flat = await importFrictionRun(resolved, resolved, "latest", diagnostics);
    if (flat) runs.push(flat);
  }

  return {
    schemaVersion: PORTABLE_SCHEMA_VERSION,
    kind: "friction",
    identity: scenarioId ? "identified" : "ambiguous-legacy",
    scenarioId,
    slug,
    title: asString(meta?.title) ?? humanize(slug),
    description: asString(meta?.description) ?? "",
    provenance: provenanceFrom(meta, resolved, worktree),
    runs,
    diagnostics,
  };
}

export async function importPortableRecord(directory: string): Promise<PortableRecord> {
  const resolved = path.resolve(directory);
  if (isInsideFrictionLogs(resolved) || (await exists(path.join(resolved, PROMPT_FILE)))) {
    return importFrictionRecord(resolved);
  }
  return importShotRecord(resolved);
}

/** Copy the live scenario prompt into a run so later edits cannot rewrite history. */
export async function snapshotFrictionPrompt(scenarioDir: string, runDir: string): Promise<PortablePromptSnapshot> {
  const live = path.join(scenarioDir, PROMPT_FILE);
  const dest = path.join(runDir, PROMPT_SNAPSHOT_FILE);
  await fs.promises.copyFile(live, dest);
  const hashed = await hashIfPresent(dest);
  return { path: dest, sha256: hashed?.sha256 ?? null, frozen: true };
}

export interface FrictionRunStepInput {
  id: string;
  title: string;
  description: string;
  transcript: string;
  screenshotPath: string;
  screenshotName: string;
  good?: string[];
  improve?: string[];
  url?: string;
}

/** Local producer used by the documented friction-run contract. */
export async function writeFrictionAttempt(
  scenarioDir: string,
  runId: string,
  steps: FrictionRunStepInput[],
): Promise<{ runDir: string; runId: string }> {
  const resolved = path.resolve(scenarioDir);
  const runDir = path.join(resolved, RUNS_DIR, runId);
  await fs.promises.mkdir(runDir, { recursive: true });
  const snapshot = await snapshotFrictionPrompt(resolved, runDir);
  const metaPath = path.join(resolved, META_FILE);
  const existing = (await readJsonRecord(metaPath)) ?? {};
  const scenarioId = asString(existing.scenario_id) ?? newStableId();
  const provenance = collectProvenance(resolved);
  const nextMeta = {
    ...existing,
    version: 1,
    schema_version: 1,
    slug: path.basename(resolved),
    scenario_id: scenarioId,
    provenance: provenanceFields(provenance),
    status: "complete",
    updated_at: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  };
  await fs.promises.writeFile(metaPath, `${JSON.stringify(nextMeta, null, 2)}\n`);

  const lines: string[] = [];
  for (const [index, step] of steps.entries()) {
    const dest = path.join(runDir, step.screenshotName);
    await fs.promises.copyFile(step.screenshotPath, dest);
    lines.push(
      JSON.stringify({
        step: index + 1,
        id: step.id,
        entry_id: newStableId(),
        title: step.title,
        description: step.description,
        transcript: step.transcript,
        screenshots: [step.screenshotName],
        asset_ids: { [step.screenshotName]: newStableId() },
        good: step.good ?? [],
        improve: step.improve ?? [],
        url: step.url,
        captured_at: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
      }),
    );
  }
  await fs.promises.writeFile(path.join(runDir, LOG_FILE), `${lines.join("\n")}\n`);
  await fs.promises.writeFile(
    path.join(runDir, META_FILE),
    `${JSON.stringify(
      {
        version: 1,
        schema_version: 1,
        run_id: runId,
        status: "complete",
        prompt_sha256: snapshot.sha256,
        captured_at: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
      },
      null,
      2,
    )}\n`,
  );
  return { runDir, runId };
}
