import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
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
  const status = git(cwd, ["status", "--porcelain"]);
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

export interface ContentRevisionEntry {
  title: string;
  description: string;
  transcript: string;
  assetDigests: string[];
}

/** Deterministic digest over ordered entry text and asset hashes. */
export function contentRevision(input: {
  entries: ContentRevisionEntry[];
  promptSha256?: string | null;
}): string {
  const hash = createHash("sha256");
  hash.update("astroshot-content-revision-v1\n");
  if (input.promptSha256) {
    hash.update("prompt\n");
    hash.update(input.promptSha256);
    hash.update("\n");
  }
  for (const [index, entry] of input.entries.entries()) {
    hash.update(`entry ${index}\n`);
    hash.update(entry.title);
    hash.update("\n");
    hash.update(entry.description);
    hash.update("\n");
    hash.update(entry.transcript);
    hash.update("\n");
    for (const digest of entry.assetDigests) {
      hash.update(digest);
      hash.update("\n");
    }
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
