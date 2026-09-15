export type FeatureStatus = "running" | "pass" | "fail" | "idle";

/** On-disk portable record contract. Existing `version: 1` documents stay valid. */
export const PORTABLE_SCHEMA_VERSION = 1;

export type PortableKind = "shot" | "friction";
export type PortableIdentity = "identified" | "ambiguous-legacy";

export type ImportDiagnosticKind =
  | "malformed-line"
  | "missing-asset"
  | "ambiguous-identity"
  | "unfrozen-prompt";

export interface ImportDiagnostic {
  kind: ImportDiagnosticKind;
  message: string;
  line?: number;
  fileName?: string;
  path?: string;
}

/** Capture-time location and git metadata. Never used as global identity. */
export interface CaptureProvenance {
  sourcePath: string;
  worktree: string;
  repo: string | null;
  commit: string | null;
  branch: string | null;
  /** null when git status was not known. */
  dirty: boolean | null;
  taskId: string | null;
  pr: string | null;
}

export interface PortableAsset {
  assetId: string | null;
  role: "image" | "poster" | "video" | "screenshot";
  fileName: string;
  path: string | null;
  sha256: string | null;
  missing: boolean;
  byteLength: number | null;
}

export interface PortableReviewClaim {
  comments: ReviewComment[];
  /** Poster-only or log.jsonl-only hash from review.json. */
  legacySha256: string | null;
  legacyHashMatches: boolean | null;
  /** Stored content-revision claim, if a producer wrote one. */
  contentRevisionClaim: string | null;
  contentRevisionMatches: boolean | null;
}

export interface PortableEntry {
  entryId: string | null;
  order: number;
  slug: string | null;
  title: string;
  description: string;
  transcript: string;
  capturedAt: string | null;
  assets: PortableAsset[];
  chapters: Chapter[];
  review: PortableReviewClaim | null;
}

export interface PortablePromptSnapshot {
  path: string | null;
  sha256: string | null;
  frozen: boolean;
}

export interface PortableRun {
  runId: string;
  capturedAt: string | null;
  /** Harness/agent execution status only; never a human review. */
  status: string | null;
  promptSnapshot: PortablePromptSnapshot | null;
  contentRevision: string;
  entries: PortableEntry[];
  review: PortableReviewClaim | null;
}

export interface PortableRecord {
  schemaVersion: typeof PORTABLE_SCHEMA_VERSION;
  kind: PortableKind;
  identity: PortableIdentity;
  scenarioId: string | null;
  slug: string;
  title: string;
  description: string;
  provenance: CaptureProvenance;
  runs: PortableRun[];
  diagnostics: ImportDiagnostic[];
}

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
