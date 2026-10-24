import { execFileSync } from "node:child_process";
import { createHash, randomUUID, type Hash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import type { CaptureProvenance } from "./model.js";

export function newStableId(): string {
  return randomUUID();
}

function gitCwd(sourcePath: string): string {
  try {
    if (fs.statSync(sourcePath).isDirectory()) return sourcePath;
  } catch {
    // fall through to dirname
  }
  return path.dirname(sourcePath);
}

function git(root: string, args: string[]): string | null {
  try {
    const output = execFileSync("git", ["-C", root, ...args], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    return output || null;
  } catch {
    return null;
  }
}

/**
 * Location and git metadata for a capture. Commit is recorded even when dirty;
 * dirty=true means those bytes are not claimed to be the commit.
 */
export function collectProvenance(
  sourcePath: string,
  env: NodeJS.ProcessEnv = process.env,
): CaptureProvenance {
  const resolved = path.resolve(sourcePath);
  const cwd = gitCwd(resolved);
  const inside = git(cwd, ["rev-parse", "--is-inside-work-tree"]);
  const toplevel = git(cwd, ["rev-parse", "--show-toplevel"]);
  const worktree = toplevel ?? cwd;
  if (inside !== "true") {
    return {
      sourcePath: resolved,
      worktree,
      repo: null,
      commit: null,
      branch: null,
      dirty: null,
      taskId: emptyToNull(env.ASTROSHOT_TASK_ID),
      pr: emptyToNull(env.ASTROSHOT_PR),
    };
  }
  const status = git(cwd, ["status", "--porcelain", "--", ":/", ":(exclude,glob)**/.astroshot/**"]);
  return {
    sourcePath: resolved,
    worktree,
    repo: git(cwd, ["remote", "get-url", "origin"]) ?? toplevel,
    commit: git(cwd, ["rev-parse", "HEAD"]),
    branch: git(cwd, ["rev-parse", "--abbrev-ref", "HEAD"]),
    dirty: status === null ? null : status.length > 0,
    taskId: emptyToNull(env.ASTROSHOT_TASK_ID),
    pr: emptyToNull(env.ASTROSHOT_PR),
  };
}

function emptyToNull(value: string | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

export interface ContentRevisionChapter {
  slug: string;
  title: string;
  tMs: number | null;
  note: string;
}

export interface ContentRevisionEntry {
  title: string;
  description: string;
  transcript: string;
  url: string;
  good: string[];
  improve: string[];
  chapters: ContentRevisionChapter[];
  assetDigests: string[];
}

/** Every field is written as `tag`, an 8-byte big-endian byte length, then the bytes. */
function writeField(hash: Hash, tag: string, value: string): void {
  const bytes = Buffer.from(value, "utf8");
  const length = Buffer.alloc(8);
  length.writeBigUInt64BE(BigInt(bytes.length));
  hash.update(`${tag}:`);
  hash.update(length);
  hash.update(bytes);
}

function writeList(hash: Hash, tag: string, values: string[]): void {
  writeField(hash, `${tag}.count`, String(values.length));
  for (const value of values) writeField(hash, tag, value);
}

/**
 * Deterministic digest over the reviewed content: the prompt snapshot digest,
 * then each ordered entry's title, description, transcript, url, notes,
 * chapters and asset digests. Length-prefixed fields keep `["ab","c"]` and
 * `["a","bc"]` distinct. Separate from the legacy poster-only / log-only
 * `image_sha256` claim, which stays its own Seen check.
 */
export function contentRevision(input: {
  entries: ContentRevisionEntry[];
  promptSha256?: string | null;
}): string {
  const hash = createHash("sha256");
  writeField(hash, "version", "astroshot-content-revision-v2");
  writeField(hash, "prompt", input.promptSha256 ?? "");
  writeField(hash, "entries.count", String(input.entries.length));
  for (const entry of input.entries) {
    writeField(hash, "title", entry.title);
    writeField(hash, "description", entry.description);
    writeField(hash, "transcript", entry.transcript);
    writeField(hash, "url", entry.url);
    writeList(hash, "good", entry.good);
    writeList(hash, "improve", entry.improve);
    writeField(hash, "chapters.count", String(entry.chapters.length));
    for (const chapter of entry.chapters) {
      writeField(hash, "chapter.slug", chapter.slug);
      writeField(hash, "chapter.title", chapter.title);
      writeField(hash, "chapter.t_ms", chapter.tMs === null ? "" : String(chapter.tMs));
      writeField(hash, "chapter.note", chapter.note);
    }
    writeList(hash, "asset", entry.assetDigests);
  }
  return hash.digest("hex");
}

export function missingAssetDigest(fileName: string): string {
  return `missing:${fileName}`;
}

export function provenanceFields(provenance: CaptureProvenance): Record<string, unknown> {
  return {
    source_path: provenance.sourcePath,
    worktree: provenance.worktree,
    repo: provenance.repo,
    commit: provenance.commit,
    branch: provenance.branch,
    dirty: provenance.dirty,
    task_id: provenance.taskId,
    pr: provenance.pr,
  };
}
