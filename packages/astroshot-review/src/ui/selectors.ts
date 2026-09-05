import type { FrictionLog, FrictionRun, ReviewState, Shot } from "../data/model.js";

export type StreamFilter = "unseen" | "history";

export function reviewStateOf(shot: Shot): ReviewState {
  return shot.review?.state ?? "pending";
}

export function filterShots(shots: Shot[], filter: StreamFilter, moviesOnly: boolean): Shot[] {
  const pool = moviesOnly ? shots.filter((shot) => shot.isMovie) : shots;
  return pool.filter((shot) => (filter === "unseen" ? reviewStateOf(shot) !== "seen" : reviewStateOf(shot) === "seen"));
}

export interface StreamCounts {
  pending: number;
  seen: number;
  movies: number;
}

export function streamCounts(shots: Shot[], moviesOnly: boolean): StreamCounts {
  const pool = moviesOnly ? shots.filter((shot) => shot.isMovie) : shots;
  return {
    pending: pool.filter((shot) => reviewStateOf(shot) !== "seen").length,
    seen: pool.filter((shot) => reviewStateOf(shot) === "seen").length,
    movies: shots.filter((shot) => shot.isMovie).length,
  };
}

export interface StreamGroup {
  /** Path of the last shot in the group: a stable anchor. */
  id: string;
  worktreePath: string;
  worktree: string;
  worktreeShort: string;
  shots: Shot[];
}

/** Contiguous runs of the same worktree, exactly like the app's stream. */
export function contiguousGroups(shots: Shot[]): StreamGroup[] {
  const groups: StreamGroup[] = [];
  for (const shot of shots) {
    const last = groups.at(-1);
    if (last && last.worktreePath === shot.worktreePath) {
      last.shots.push(shot);
      last.id = shot.path;
    } else {
      groups.push({
        id: shot.path,
        worktreePath: shot.worktreePath,
        worktree: shot.worktree,
        worktreeShort: shot.worktreeShort,
        shots: [shot],
      });
    }
  }
  return groups;
}

/** Same worktree, feature, and run; oldest first by sequence, else capture time. */
export function reviewSiblings(all: Shot[], shot: Shot): Shot[] {
  return all
    .filter(
      (candidate) =>
        candidate.worktreePath === shot.worktreePath &&
        candidate.feature === shot.feature &&
        candidate.runId === shot.runId,
    )
    .sort((a, b) => {
      if (a.sequence && b.sequence && a.sequence !== b.sequence) {
        return a.sequence < b.sequence ? -1 : 1;
      }
      return a.capturedAt - b.capturedAt;
    });
}

export function latestRun(log: FrictionLog): FrictionRun | null {
  return log.runs[0] ?? null;
}

export function frictionState(log: FrictionLog): ReviewState {
  return latestRun(log)?.review?.state ?? "pending";
}

export function filterFrictionLogs(logs: FrictionLog[], filter: StreamFilter): FrictionLog[] {
  return logs.filter((log) => (filter === "unseen" ? frictionState(log) !== "seen" : frictionState(log) === "seen"));
}

export function frictionSummary(log: FrictionLog): string {
  const run = latestRun(log);
  const steps = run?.steps.length ?? 0;
  const improve = run?.steps.reduce((total, step) => total + step.improve.length, 0) ?? 0;
  const parts: string[] = [];
  if (steps > 0) parts.push(steps === 1 ? "1 step" : `${steps} steps`);
  if (improve > 0) parts.push(`${improve} improve`);
  return parts.join(" · ");
}
