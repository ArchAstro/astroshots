export type FeatureStatus = "running" | "pass" | "fail" | "idle";

export interface Chapter {
  slug?: string;
  title?: string;
  tMs?: number;
}

export interface ReviewComment {
  id: string;
  body: string;
  createdAt: string;
}

export type ReviewState = "seen" | "pending";

export interface ReviewSnapshot {
  state: ReviewState;
  /** Raw decision from disk, before hash scoping. */
  decision: string | null;
  hashMatches: boolean;
  /** A decision exists but the bytes changed since it was recorded. */
  isStale: boolean;
  comments: ReviewComment[];
  reviewedAt: string | null;
}

export interface Shot {
  /** Absolute image path; stable identity. */
  id: string;
  path: string;
  fileName: string;
  worktreePath: string;
  worktree: string;
  worktreeShort: string;
  feature: string;
  featureDir: string;
  sequence: string | null;
  slug: string;
  title: string;
  description: string;
  url: string | null;
  runId: string | null;
  status: FeatureStatus | null;
  /** Epoch milliseconds. */
  capturedAt: number;
  mtimeMs: number;
  isMovie: boolean;
  videoFileName: string | null;
  /** Absolute path when the video exists on disk. */
  videoPath: string | null;
  durationMs: number | null;
  source: string | null;
  chapters: Chapter[];
  review: ReviewSnapshot | null;
}

export interface FrictionStep {
  id: string;
  step: number;
  stepId: string;
  title: string;
  description: string;
  transcript: string;
  /** Absolute paths that exist on disk. */
  screenshots: string[];
  good: string[];
  improve: string[];
  url: string | null;
  capturedAt: string | null;
}

export interface FrictionRun {
  runId: string;
  directory: string;
  logPath: string | null;
  capturedAt: number;
  status: string | null;
  steps: FrictionStep[];
  review: ReviewSnapshot | null;
}

export interface FrictionLog {
  /** `<worktreePath>::<slug>` */
  id: string;
  slug: string;
  directory: string;
  worktreePath: string;
  worktree: string;
  worktreeShort: string;
  title: string;
  description: string;
  status: string | null;
  updatedAt: number;
  promptPath: string | null;
  runs: FrictionRun[];
}

export interface AstroshotTree {
  astroshotDir: string;
  worktreePath: string;
  worktree: string;
  shots: Shot[];
  frictionLogs: FrictionLog[];
}
