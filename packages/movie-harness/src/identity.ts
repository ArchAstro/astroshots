import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export interface CaptureProvenance {
  source_path: string;
  worktree: string;
  repo: string | null;
  commit: string | null;
  branch: string | null;
  dirty: boolean | null;
  task_id: string | null;
  pr: string | null;
}

function gitCwd(sourcePath: string): string {
  try {
    if (fs.statSync(sourcePath).isDirectory()) return sourcePath;
  } catch {
    // fall through
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

function emptyToNull(value: string | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

export function newStableId(): string {
  return randomUUID();
}

/** Commit is stored even when dirty; dirty=true means bytes are not that commit. */
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
      source_path: resolved,
      worktree,
      repo: null,
      commit: null,
      branch: null,
      dirty: null,
      task_id: emptyToNull(env.ASTROSHOT_TASK_ID),
      pr: emptyToNull(env.ASTROSHOT_PR),
    };
  }
  const status = git(cwd, ["status", "--porcelain", "--", ":/", ":(exclude,glob)**/.astroshot/**"]);
  return {
    source_path: resolved,
    worktree,
    repo: git(cwd, ["remote", "get-url", "origin"]) ?? toplevel,
    commit: git(cwd, ["rev-parse", "HEAD"]),
    branch: git(cwd, ["rev-parse", "--abbrev-ref", "HEAD"]),
    dirty: status === null ? null : status.length > 0,
    task_id: emptyToNull(env.ASTROSHOT_TASK_ID),
    pr: emptyToNull(env.ASTROSHOT_PR),
  };
}
