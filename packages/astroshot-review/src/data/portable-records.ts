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
      url: entry.url ?? "",
      good: entry.good,
      improve: entry.improve,
      chapters: entry.chapters.map((chapter) => ({
        slug: chapter.slug ?? "",
        title: chapter.title ?? "",
        tMs: chapter.tMs ?? null,
        note: chapter.note ?? "",
      })),
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
    url: asString(entry?.url),
    good: [],
    improve: [],
    capturedAt: entry?.captured_at ?? null,
    assets,
    chapters: chaptersOf(entry),
    review: null,
  };
}

function storedProvenance(value: unknown): CaptureProvenance | null {
  if (!isRecord(value)) return null;
  const p = provenanceFrom({ provenance: value }, "", "");
  return { ...p, sourcePath: asString(value.source_path) ?? "unknown", worktree: asString(value.worktree) ?? "unknown" };
}

async function shotRun(
  featureDir: string,
  manifest: FeatureManifest | null,
  run: {
    runId: string | null;
    status: string | null;
    provenance: CaptureProvenance | null;
    shots: ManifestShot[];
    listImagesWhenEmpty: boolean;
  },
  diagnostics: ImportDiagnostic[],
): Promise<PortableRun> {
  const orderedFiles = run.shots
    .map((shot) => shot.file ?? shot.poster)
    .filter((name): name is string => Boolean(name));
  const files = orderedFiles.length > 0 || !run.listImagesWhenEmpty ? orderedFiles : await listImageFiles(featureDir);
  const scoped: FeatureManifest = { ...manifest, shots: run.shots };
  const entries: PortableEntry[] = [];
  for (const [index, fileName] of files.entries()) {
    entries.push(await entryFromShot(featureDir, fileName, index + 1, scoped, diagnostics));
  }
  const revision = runRevision(entries, null);
  const poster = entries[0]?.assets[0] ?? null;
  const review = poster ? await reviewClaim(featureDir, poster.fileName, run.runId, poster.sha256, revision) : null;
  if (entries[0]) entries[0].review = review;
  return {
    runId: run.runId ?? "latest",
    capturedAt: entries[0]?.capturedAt ?? null,
    status: run.status,
    provenance: run.provenance,
    promptSnapshot: null,
    contentRevision: revision,
    entries,
    review,
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

  const runs: PortableRun[] = [];
  // Archived attempts first (oldest to newest), then the attempt that owns the directory now.
  for (const previous of manifest?.previous_runs ?? []) {
    runs.push(
      await shotRun(
        resolved,
        manifest,
        {
          runId: asString(previous.run_id),
          status: asString(previous.status),
          provenance: storedProvenance(previous.provenance),
          shots: previous.shots ?? [],
          listImagesWhenEmpty: false,
        },
        diagnostics,
      ),
    );
  }
  runs.push(
    await shotRun(
      resolved,
      manifest,
      {
        runId: asString(manifest?.run_id),
        status: asString(manifest?.status),
        provenance: storedProvenance(manifest?.provenance),
        shots: manifest?.shots ?? [],
        listImagesWhenEmpty: (manifest?.previous_runs ?? []).length === 0,
      },
      diagnostics,
    ),
  );
  reportDuplicateRunIds(runs, diagnostics, resolved);

  return {
    schemaVersion: PORTABLE_SCHEMA_VERSION,
    kind: "shot",
    identity: scenarioId ? "identified" : "ambiguous-legacy",
    scenarioId,
    slug,
    title: asString(manifest?.feature) ?? humanize(slug),
    description: asString(manifest?.description) ?? "",
    provenance: provenanceFrom(manifest as Record<string, unknown> | null, resolved, worktree),
    runs,
    diagnostics,
  };
}

function reportDuplicateRunIds(runs: PortableRun[], diagnostics: ImportDiagnostic[], where: string): void {
  const seen = new Set<string>();
  for (const run of runs) {
    if (seen.has(run.runId)) {
      diagnostics.push({
        kind: "duplicate-run-id",
        message: `run id ${run.runId} appears more than once; distinct attempts must not share an id`,
        path: where,
      });
    }
    seen.add(run.runId);
  }
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
      url: step.url,
      good: step.good,
      improve: step.improve,
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
            url: null,
            good: [],
            improve: [],
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
    provenance: isRecord(meta?.provenance) ? provenanceFrom(meta, "", "") : null,
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
  runs.sort((a, b) => a.runId.localeCompare(b.runId));
  if (runs.length === 0 && (await exists(path.join(resolved, LOG_FILE)))) {
    const flat = await importFrictionRun(resolved, resolved, "latest", diagnostics);
    if (flat) runs.push(flat);
  }

  reportDuplicateRunIds(runs, diagnostics, resolved);

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

function requireBasename(value: string, label: string): string {
  if (
    !value ||
    value !== path.basename(value) ||
    value === "." ||
    value === ".." ||
    value.startsWith(".") ||
    value.includes("\\")
  ) {
    throw new Error(`${label} must be a plain file name, got ${JSON.stringify(value)}`);
  }
  return value;
}

/** Copy the live scenario prompt into a run directory so later edits cannot rewrite history. */
export async function snapshotFrictionPrompt(scenarioDir: string, runDir: string): Promise<PortablePromptSnapshot> {
  const live = path.join(scenarioDir, PROMPT_FILE);
  const dest = path.join(runDir, PROMPT_SNAPSHOT_FILE);
  // COPYFILE_EXCL: an existing snapshot is history, never replaced.
  await fs.promises.copyFile(live, dest, fs.constants.COPYFILE_EXCL);
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

export interface WriteFrictionAttemptOptions {
  /**
   * Execution status of this attempt as the caller observed it (for example
   * `running`, `complete`, `failed`). The writer never infers one.
   */
  status: string;
  /** Overrides git and environment discovery, mainly for tests. */
  provenance?: CaptureProvenance;
}

async function writeJsonAtomic(filePath: string, value: unknown): Promise<void> {
  const tmp = `${filePath}.${process.pid}.${newStableId()}.tmp`;
  await fs.promises.writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`);
  await fs.promises.rename(tmp, filePath);
}

const timestamp = () => new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

/**
 * Local producer used by the documented friction-run contract.
 *
 * Builds the run in a hidden staging directory and renames it into
 * `runs/<runId>/` once complete, so a reader never sees a half-written attempt
 * and a failed attempt leaves nothing behind. An existing run id is refused.
 */
export async function writeFrictionAttempt(
  scenarioDir: string,
  runId: string,
  steps: FrictionRunStepInput[],
  options: WriteFrictionAttemptOptions,
): Promise<{ runDir: string; runId: string }> {
  const resolved = path.resolve(scenarioDir);
  await fs.promises.mkdir(resolved, { recursive: true });
  return withScenarioLock(resolved, () => writeFrictionAttemptLocked(resolved, runId, steps, options));
}

const LOCK_DIR = ".attempt.lock";

function processAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

/**
 * Scenario-level lock: an atomic `mkdir` holding the owner pid. Concurrent first
 * attempts serialize, so only one mints the scenario id. A lock whose owner
 * died is taken over.
 */
async function withScenarioLock<T>(scenarioDir: string, fn: () => Promise<T>): Promise<T> {
  const lock = path.join(scenarioDir, LOCK_DIR);
  const deadline = Date.now() + 30_000;
  for (;;) {
    try {
      await fs.promises.mkdir(lock);
      await fs.promises.writeFile(path.join(lock, "pid"), String(process.pid));
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      const owner = Number.parseInt(await fs.promises.readFile(path.join(lock, "pid"), "utf8").catch(() => ""), 10);
      if (Number.isInteger(owner) && owner > 0 && !processAlive(owner)) {
        await fs.promises.rm(lock, { recursive: true, force: true });
        continue;
      }
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${lock}`);
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
  try {
    return await fn();
  } finally {
    await fs.promises.rm(lock, { recursive: true, force: true });
  }
}

/** The scenario id recorded by any earlier run, for recovery after a crash before scenario meta was written. */
async function scenarioIdFromRuns(runsDir: string): Promise<string | null> {
  let names: string[];
  try {
    names = (await fs.promises.readdir(runsDir)).filter((name) => !name.startsWith(".")).sort();
  } catch {
    return null;
  }
  for (const name of names) {
    const id = asString((await readJsonRecord(path.join(runsDir, name, META_FILE)))?.scenario_id);
    if (id) return id;
  }
  return null;
}

async function writeFrictionAttemptLocked(
  resolved: string,
  runId: string,
  steps: FrictionRunStepInput[],
  options: WriteFrictionAttemptOptions,
): Promise<{ runDir: string; runId: string }> {
  requireBasename(runId, "runId");
  if (!options?.status?.trim()) throw new Error("status is required: pass the attempt's execution status");
  const names = steps.map((step) => requireBasename(step.screenshotName, "screenshotName"));
  for (const name of names) {
    if ([LOG_FILE, META_FILE, PROMPT_SNAPSHOT_FILE].includes(name) || !isImageFile(name)) {
      throw new Error(`screenshotName ${JSON.stringify(name)} must be an image file name that does not collide with run files`);
    }
  }
  if (new Set(names).size !== names.length) throw new Error("screenshotName values must be unique within a run");

  const runsDir = path.join(resolved, RUNS_DIR);
  const runDir = path.join(runsDir, runId);
  if (await lstatExists(runDir)) {
    throw new Error(`run ${runId} already exists at ${runDir}; attempts are never overwritten`);
  }
  await fs.promises.mkdir(runsDir, { recursive: true });

  const staging = path.join(runsDir, `.staging-${runId}-${newStableId()}`);
  await fs.promises.mkdir(staging);
  try {
    const snapshot = await snapshotFrictionPrompt(resolved, staging);
    const metaPath = path.join(resolved, META_FILE);
    const existing = (await readJsonRecord(metaPath)) ?? {};
    if (Object.keys(existing).length === 0 && (await lstatExists(metaPath)) && (await fs.promises.readFile(metaPath, "utf8")).trim() !== "") {
      throw new Error(`${metaPath} exists but is not a JSON object; refusing to replace the scenario identity`);
    }
    const scenarioId = asString(existing.scenario_id) ?? (await scenarioIdFromRuns(runsDir)) ?? newStableId();
    const provenance = provenanceFields(options.provenance ?? collectProvenance(resolved));
    const capturedAt = timestamp();

    const lines: string[] = [];
    for (const [index, step] of steps.entries()) {
      const name = names[index]!;
      await fs.promises.copyFile(step.screenshotPath, path.join(staging, name), fs.constants.COPYFILE_EXCL);
      lines.push(
        JSON.stringify({
          step: index + 1,
          id: step.id,
          entry_id: newStableId(),
          title: step.title,
          description: step.description,
          transcript: step.transcript,
          screenshots: [name],
          asset_ids: { [name]: newStableId() },
          good: step.good ?? [],
          improve: step.improve ?? [],
          url: step.url,
          captured_at: timestamp(),
        }),
      );
    }
    await fs.promises.writeFile(path.join(staging, LOG_FILE), lines.length ? `${lines.join("\n")}\n` : "");
    await writeJsonAtomic(path.join(staging, META_FILE), {
      version: 1,
      schema_version: 1,
      run_id: runId,
      scenario_id: scenarioId,
      status: options.status,
      provenance,
      prompt_sha256: snapshot.sha256,
      captured_at: capturedAt,
    });

    // Re-check right before publishing: rename onto an empty directory would succeed.
    if (await lstatExists(runDir)) {
      throw new Error(`run ${runId} already exists at ${runDir}; attempts are never overwritten`);
    }
    await fs.promises.rename(staging, runDir);

    // Scenario meta keeps the identity and the provenance of the first attempt;
    // each run's own provenance lives in its run meta.
    await writeJsonAtomic(metaPath, {
      ...existing,
      version: 1,
      schema_version: 1,
      slug: path.basename(resolved),
      scenario_id: scenarioId,
      provenance: existing.provenance ?? provenance,
      status: options.status,
      updated_at: capturedAt,
    });
  } finally {
    await fs.promises.rm(staging, { recursive: true, force: true });
  }
  return { runDir, runId };
}

async function lstatExists(filePath: string): Promise<boolean> {
  try {
    await fs.promises.lstat(filePath);
    return true;
  } catch {
    return false;
  }
}
