import path from "node:path";

export const ASTROSHOT_DIR = ".astroshot";
/** User stories (formerly friction logs) live under `.astroshot/stories/`. */
export const STORIES_DIR = "stories";
/** The legacy name of the user stories directory; still read. */
export const FRICTION_DIR = "friction-logs";
/** Names under `.astroshot/` that hold user stories, never shot features. */
export const RESERVED_DIRS: readonly string[] = [STORIES_DIR, FRICTION_DIR];

export function isReservedDir(name: string): boolean {
  return RESERVED_DIRS.includes(name);
}

/** Directories the scanner never descends into (mirrors the macOS app). */
export const SKIP_DIRECTORIES = new Set([
  "node_modules",
  ".git",
  "DerivedData",
  "build",
  ".build",
  "Pods",
  ".next",
  "dist",
  "out",
  "target",
  "vendor",
  "Checkouts",
  "xcuserdata",
  ".turbo",
  ".cache",
  "coverage",
  "tmp",
  ".pnpm-store",
  "Carthage",
  "bazel-bin",
  "bazel-out",
  "bazel-testlogs",
  ".gradle",
]);

export const MAX_SCAN_DEPTH = 10;

export const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "webp", "gif"]);
export const VIDEO_EXTENSIONS = ["webm", "mp4", "mov", "m4v"];

export function extensionOf(fileName: string): string {
  return path.extname(fileName).slice(1).toLowerCase();
}

export function isImageFile(fileName: string): boolean {
  return IMAGE_EXTENSIONS.has(extensionOf(fileName));
}

export interface ShotPath {
  worktreePath: string;
  worktree: string;
  feature: string;
  featureDir: string;
  fileName: string;
}

/** Accept only `<worktree>/.astroshot/<feature>/<image>`; user stories are excluded. */
export function parseShotPath(imagePath: string): ShotPath | null {
  const fileName = path.basename(imagePath);
  if (!isImageFile(fileName)) return null;
  const featureDir = path.dirname(imagePath);
  const feature = path.basename(featureDir);
  if (!feature || feature === ASTROSHOT_DIR || isReservedDir(feature)) return null;
  const astroshotDir = path.dirname(featureDir);
  if (path.basename(astroshotDir) !== ASTROSHOT_DIR) return null;
  if (imagePath.split(path.sep).includes(FRICTION_DIR)) return null;
  const worktreePath = path.dirname(astroshotDir);
  return {
    worktreePath,
    worktree: path.basename(worktreePath),
    feature,
    featureDir,
    fileName,
  };
}

export function sequenceAndSlug(fileName: string): { sequence: string | null; slug: string } {
  const stem = fileName.replace(/\.[^.]+$/, "");
  const dash = stem.indexOf("-");
  if (dash > 0) {
    const head = stem.slice(0, dash);
    if (/^\d+$/.test(head)) return { sequence: head, slug: stem.slice(dash + 1) };
  }
  return { sequence: null, slug: stem };
}

export function humanize(slug: string): string {
  return slug
    .split(/[-_]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

export function worktreeShort(name: string): string {
  const match = /wt\d+/.exec(name);
  if (match) return match[0];
  return name.length <= 8 ? name : name.slice(0, 6);
}

export function isInsideFrictionLogs(filePath: string): boolean {
  const parts = filePath.split(path.sep);
  const index = parts.indexOf(ASTROSHOT_DIR);
  return index !== -1 && parts[index + 1] !== undefined && isReservedDir(parts[index + 1]!);
}

export function frictionLogsDir(astroshotDir: string): string {
  return path.join(astroshotDir, FRICTION_DIR);
}

export function storiesDir(astroshotDir: string): string {
  return path.join(astroshotDir, STORIES_DIR);
}

export function abbreviateHome(filePath: string, home: string): string {
  if (filePath === home) return "~";
  const prefix = home.endsWith(path.sep) ? home : `${home}${path.sep}`;
  return filePath.startsWith(prefix) ? `~${path.sep}${filePath.slice(prefix.length)}` : filePath;
}
