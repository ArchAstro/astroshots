/**
 * `review.json` — the human side of the on-disk contract.
 *
 * Reads mirror the macOS app exactly: version gate, run-id gate, then hash
 * scoping. Writes mirror it too: sorted keys, second-precision UTC
 * timestamps, uppercase UUID comment ids, run reset on a run-id change, and
 * an atomic temp-file rename.
 */
import { createHash, randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import type { ReviewComment, ReviewSnapshot } from "./model.js";

export const REVIEW_FILE = "review.json";

export interface ReviewEntry {
  decision?: string;
  reviewed_at?: string;
  image_sha256?: string;
  comments?: ReviewComment[];
}

export interface ReviewDocument {
  version: number;
  run_id?: string;
  updated_at?: string;
  reviews: Record<string, StoredEntry>;
}

interface StoredComment {
  id: string;
  body: string;
  created_at: string;
}

interface StoredEntry {
  decision?: string;
  reviewed_at?: string;
  image_sha256?: string;
  comments?: StoredComment[];
}

export class UnsupportedReviewVersion extends Error {
  constructor(version: unknown) {
    super(`Unsupported review.json version ${String(version)}`);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function emptyDocument(): ReviewDocument {
  return { version: 1, reviews: {} };
}

/** Missing file → empty document. Malformed JSON → null (treated as unreadable). */
export async function readReviewDocument(directory: string): Promise<ReviewDocument | null> {
  let raw: string;
  try {
    raw = await fs.promises.readFile(path.join(directory, REVIEW_FILE), "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return emptyDocument();
    return null;
  }
  return parseReviewDocument(raw);
}

export function parseReviewDocument(raw: string): ReviewDocument | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isRecord(parsed)) return null;
  if (parsed.version !== 1) throw new UnsupportedReviewVersion(parsed.version);
  const reviews: Record<string, StoredEntry> = {};
  if (isRecord(parsed.reviews)) {
    for (const [fileName, entry] of Object.entries(parsed.reviews)) {
      if (!isRecord(entry)) continue;
      const comments = Array.isArray(entry.comments)
        ? entry.comments.filter(isRecord).map((comment) => ({
            id: String(comment.id ?? ""),
            body: String(comment.body ?? ""),
            created_at: String(comment.created_at ?? ""),
          }))
        : undefined;
      reviews[fileName] = {
        decision: typeof entry.decision === "string" ? entry.decision : undefined,
        reviewed_at: typeof entry.reviewed_at === "string" ? entry.reviewed_at : undefined,
        image_sha256: typeof entry.image_sha256 === "string" ? entry.image_sha256 : undefined,
        comments,
      };
    }
  }
  return {
    version: 1,
    run_id: typeof parsed.run_id === "string" ? parsed.run_id : undefined,
    updated_at: typeof parsed.updated_at === "string" ? parsed.updated_at : undefined,
    reviews,
  };
}

export function sha256File(filePath: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const stream = fs.createReadStream(filePath);
    stream.on("error", reject);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest("hex")));
  });
}

export function sha256Bytes(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex");
}

/** The entry that applies to `fileName`, or null when the run gate rejects it. */
export function scopedEntry(
  document: ReviewDocument,
  fileName: string,
  expectedRunId: string | null,
): StoredEntry | null {
  if (expectedRunId !== null && document.run_id !== expectedRunId) return null;
  return document.reviews[fileName] ?? null;
}

/** Whether validating this entry needs the image's current hash. */
export function entryNeedsHash(entry: StoredEntry | null): boolean {
  return Boolean(entry?.image_sha256);
}

/**
 * The seen/stale truth table from the app's `ReviewSnapshot`:
 * hash mismatch hides the decision but keeps comments and flags staleness.
 */
export function snapshotFromEntry(
  entry: StoredEntry | null,
  currentSha256: string | null,
): ReviewSnapshot {
  const hashMatches = Boolean(entry?.image_sha256) && entry?.image_sha256 === currentSha256;
  const decision = entry?.decision ?? null;
  const effective = hashMatches ? decision : null;
  return {
    state: effective === "seen" || effective === "approved" ? "seen" : "pending",
    decision,
    hashMatches,
    isStale: decision !== null && !hashMatches,
    comments: (entry?.comments ?? []).map((comment) => ({
      id: comment.id,
      body: comment.body,
      createdAt: comment.created_at,
    })),
    reviewedAt: entry?.reviewed_at ?? null,
  };
}

export function nowIso(date = new Date()): string {
  return date.toISOString().replace(/\.\d{3}Z$/, "Z");
}

export function newCommentId(): string {
  return randomUUID().toUpperCase();
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (isRecord(value)) {
    const sorted: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) {
      const inner = value[key];
      if (inner !== undefined) sorted[key] = sortKeys(inner);
    }
    return sorted;
  }
  return value;
}

export function serializeReviewDocument(document: ReviewDocument): string {
  return `${JSON.stringify(sortKeys(document), null, 2)}\n`;
}

export async function writeReviewDocument(directory: string, document: ReviewDocument): Promise<void> {
  const target = path.join(directory, REVIEW_FILE);
  const temp = path.join(directory, `.review.tmp.${randomUUID()}`);
  await fs.promises.writeFile(temp, serializeReviewDocument(document), { flag: "wx" });
  try {
    await fs.promises.rename(temp, target);
  } catch (error) {
    await fs.promises.rm(temp, { force: true });
    throw error;
  }
}

/** A run-id change starts a fresh review map, exactly like the app. */
export function resetReviewsIfNeeded(document: ReviewDocument, runId: string | null): void {
  if (runId !== null && document.run_id !== runId) {
    document.run_id = runId;
    document.reviews = {};
  }
}

export interface ReviewWriteRequest {
  /** Directory holding review.json (feature dir, or friction run dir). */
  directory: string;
  /** Key inside `reviews` (image file name, or `log.jsonl`). */
  fileName: string;
  runId: string | null;
  /** Absolute path of the bytes to hash when marking seen. */
  targetPath: string;
}

interface FileStamp {
  mtimeMs: number;
  size: number;
}

async function stampOf(filePath: string): Promise<FileStamp | null> {
  try {
    const stat = await fs.promises.stat(filePath);
    return { mtimeMs: stat.mtimeMs, size: stat.size };
  } catch {
    return null;
  }
}

function sameStamp(a: FileStamp | null, b: FileStamp | null): boolean {
  return a?.mtimeMs === b?.mtimeMs && a?.size === b?.size;
}

async function loadForWrite(directory: string): Promise<{ document: ReviewDocument; stamp: FileStamp | null }> {
  const stamp = await stampOf(path.join(directory, REVIEW_FILE));
  const document = await readReviewDocument(directory);
  if (document === null) {
    throw new Error(`review.json in ${directory} is not valid JSON; refusing to overwrite it`);
  }
  return { document, stamp };
}

/**
 * Read → mutate → write, re-reading when another writer (the macOS app, a
 * second tray) changed the file in between so neither side's update is lost.
 */
async function mutateReviewDocument<T>(
  directory: string,
  mutate: (document: ReviewDocument) => Promise<T> | T,
): Promise<T> {
  const target = path.join(directory, REVIEW_FILE);
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const { document, stamp } = await loadForWrite(directory);
    const result = await mutate(document);
    if (!sameStamp(stamp, await stampOf(target))) continue;
    await writeReviewDocument(directory, document);
    return result;
  }
  throw new Error(`review.json in ${directory} kept changing underneath this write; try again`);
}

function appendComment(entry: StoredEntry, body: string, now: string): ReviewComment {
  const trimmed = body.trim();
  if (!trimmed) throw new Error("Feedback cannot be empty");
  const comment: StoredComment = { id: newCommentId(), body: trimmed, created_at: now };
  entry.comments = [...(entry.comments ?? []), comment];
  return { id: comment.id, body: comment.body, createdAt: comment.created_at };
}

export async function markSeen(
  request: ReviewWriteRequest,
  options: { comment?: string; now?: Date } = {},
): Promise<ReviewSnapshot> {
  const sha = await sha256File(request.targetPath);
  return mutateReviewDocument(request.directory, (document) => {
    resetReviewsIfNeeded(document, request.runId);
    const now = nowIso(options.now);
    const entry: StoredEntry = document.reviews[request.fileName] ?? {};
    if (options.comment?.trim()) appendComment(entry, options.comment, now);
    entry.decision = "seen";
    entry.reviewed_at = now;
    entry.image_sha256 = sha;
    document.reviews[request.fileName] = entry;
    document.updated_at = now;
    return snapshotFromEntry(entry, sha);
  });
}

export async function addComment(
  request: ReviewWriteRequest,
  body: string,
  options: { currentSha256?: string | null; now?: Date } = {},
): Promise<ReviewSnapshot> {
  if (!body.trim()) throw new Error("Feedback cannot be empty");
  const entry = await mutateReviewDocument(request.directory, (document) => {
    resetReviewsIfNeeded(document, request.runId);
    const now = nowIso(options.now);
    const stored: StoredEntry = document.reviews[request.fileName] ?? {};
    appendComment(stored, body, now);
    document.reviews[request.fileName] = stored;
    document.updated_at = now;
    return stored;
  });
  const sha = entry.image_sha256
    ? (options.currentSha256 ?? (await sha256File(request.targetPath)))
    : null;
  return snapshotFromEntry(entry, sha);
}
