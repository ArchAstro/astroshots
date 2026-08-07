export type MovieSourceKind =
  | "frames"
  | "browser"
  | "pty"
  | "desktop.window"
  | "desktop.display"
  | "desktop.region";

export type MovieFormat = "webm" | "mp4";

export type ManifestStatus = "running" | "pass" | "fail" | "idle";

export interface Size {
  width: number;
  height: number;
}

export interface MovieChapter {
  slug: string;
  tMs: number;
  note?: string;
}

export interface MovieArtifact {
  videoPath: string;
  posterPath: string;
  durationMs: number;
  chapters: MovieChapter[];
  source: MovieSourceKind;
  feature: string;
  slug: string;
  sequence: string;
  runId: string;
}

export interface MovieSessionOptions {
  /** Feature directory under .astroshot/ (kebab-case). */
  feature: string;
  /** Filename slug for this movie. */
  slug: string;
  /** Worktree / project root that contains (or will contain) .astroshot/. */
  root?: string;
  /** Stable run id; defaults to feature-timestamp-pid. */
  runId?: string;
  title?: string;
  description?: string;
  size?: Size;
  /** Target encode frame rate. Default 15. */
  fps?: number;
  /** Output container. Default webm (Playwright-native). */
  format?: MovieFormat;
  /** Manifest status while recording. Default running. */
  status?: ManifestStatus;
  source: MovieSourceKind;
}

export interface EncodeFramesRequest {
  framePaths: string[];
  outPath: string;
  size: Size;
  fps: number;
  /** Wall-clock duration override when frame timestamps are irregular. */
  durationMs?: number;
}

export interface SinkMovieRequest {
  root: string;
  feature: string;
  slug: string;
  runId: string;
  title?: string;
  description?: string;
  status?: ManifestStatus;
  source: MovieSourceKind;
  posterPath: string;
  videoPath: string;
  durationMs: number;
  chapters: MovieChapter[];
  size?: Size;
}

export interface SinkMovieResult {
  sequence: string;
  posterDest: string;
  videoDest: string;
  manifestPath: string;
  featureDir: string;
}

/** On-disk state for multi-process frames CLI (start / push / mark / stop). */
export interface PersistedFrameSession {
  version: 1;
  id: string;
  feature: string;
  slug: string;
  root: string;
  runId: string;
  title?: string;
  description?: string;
  size: Size;
  fps: number;
  format: MovieFormat;
  source: "frames";
  startedAtMs: number;
  frameDir: string;
  frameCount: number;
  chapters: MovieChapter[];
}

export type PtyKey =
  | "enter"
  | "up"
  | "down"
  | "left"
  | "right"
  | "tab"
  | "escape"
  | "backspace"
  | "space"
  | "ctrl-c"
  | "ctrl-d";

export type PtyAction =
  | { waitFor: string; timeoutMs?: number }
  | { waitForExit: true; timeoutMs?: number }
  | { key: PtyKey }
  | { text: string }
  | { pauseMs: number };

export interface PtyMovieFixture {
  version: 1;
  command: string;
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
  cols?: number;
  rows?: number;
  timeoutMs?: number;
  settleMs?: number;
  allowNonZeroExit?: boolean;
  actions?: PtyAction[];
  expectText?: string[];
  background?: string;
  foreground?: string;
  fontFamily?: string;
  fontSize?: number;
  lineHeight?: number;
  padding?: number;
  borderRadius?: number;
  scale?: number;
  /** Sample rate for the movie. Defaults to session fps. */
  movieFps?: number;
}

export interface BrowserMovieOptions {
  /** Navigate here when the harness owns the page. */
  url?: string;
  /** Optional script module that exports `default` async (page) => void. */
  scriptPath?: string;
  /** Reuse an existing headed browser (debug). Default headless. */
  headed?: boolean;
  /** How long to keep recording after script ends (ms). Default 0. */
  settleMs?: number;
}
