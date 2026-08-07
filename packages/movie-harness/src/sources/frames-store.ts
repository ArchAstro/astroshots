import fs from "node:fs";
import path from "node:path";
import { randomBytes } from "node:crypto";

import { encodeFrames, posterFromFrames } from "../encode.js";
import {
  assertKebabCase,
  assertSlug,
  defaultRunId,
  ensureDir,
  movieStateDir,
  resolveRoot,
} from "../paths.js";
import { sinkMovie } from "../sink.js";
import type {
  MovieArtifact,
  MovieChapter,
  MovieFormat,
  PersistedFrameSession,
  Size,
} from "../types.js";

const STATE_FILE = "session.json";

function sessionDir(root: string, feature: string, id: string): string {
  return path.join(movieStateDir(root, feature), id);
}

function statePath(dir: string): string {
  return path.join(dir, STATE_FILE);
}

function writeState(state: PersistedFrameSession): void {
  const dir = sessionDir(state.root, state.feature, state.id);
  ensureDir(dir);
  ensureDir(state.frameDir);
  const tmp = `${statePath(dir)}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(state, null, 2)}\n`);
  fs.renameSync(tmp, statePath(dir));
}

function readState(dir: string): PersistedFrameSession {
  const raw = JSON.parse(fs.readFileSync(statePath(dir), "utf8")) as PersistedFrameSession;
  if (raw.version !== 1) {
    throw new Error(`unsupported movie session version: ${raw.version}`);
  }
  return raw;
}

/** Resolve the active frames session for a feature (latest if id omitted). */
export function loadFrameSession(
  root: string,
  feature: string,
  id?: string,
): PersistedFrameSession {
  const resolvedRoot = resolveRoot(root);
  assertKebabCase(feature, "feature");
  const base = movieStateDir(resolvedRoot, feature);
  if (!fs.existsSync(base)) {
    throw new Error(`no movie session for feature ${feature}`);
  }
  if (id) {
    const dir = sessionDir(resolvedRoot, feature, id);
    if (!fs.existsSync(statePath(dir))) {
      throw new Error(`movie session not found: ${id}`);
    }
    return readState(dir);
  }
  const ids = fs
    .readdirSync(base)
    .filter((name) => fs.existsSync(statePath(path.join(base, name))))
    .sort();
  if (ids.length === 0) {
    throw new Error(`no movie session for feature ${feature}`);
  }
  return readState(path.join(base, ids.at(-1)!));
}

export function startFrameSession(options: {
  feature: string;
  slug: string;
  root?: string;
  runId?: string;
  title?: string;
  description?: string;
  size?: Size;
  fps?: number;
  format?: MovieFormat;
}): PersistedFrameSession {
  assertKebabCase(options.feature, "feature");
  assertSlug(options.slug);
  const root = resolveRoot(options.root);
  const id = randomBytes(6).toString("hex");
  const dir = sessionDir(root, options.feature, id);
  const frameDir = path.join(dir, "frames");
  ensureDir(frameDir);
  const state: PersistedFrameSession = {
    version: 1,
    id,
    feature: options.feature,
    slug: options.slug,
    root,
    runId: options.runId ?? defaultRunId(options.feature),
    title: options.title,
    description: options.description,
    size: options.size ?? { width: 1280, height: 720 },
    fps: options.fps ?? 15,
    format: options.format ?? "webm",
    source: "frames",
    startedAtMs: Date.now(),
    frameDir,
    frameCount: 0,
    chapters: [],
  };
  writeState(state);
  return state;
}

export function pushFrameToSession(
  state: PersistedFrameSession,
  imagePath: string,
): PersistedFrameSession {
  if (!fs.existsSync(imagePath)) {
    throw new Error(`frame not found: ${imagePath}`);
  }
  const ext = path.extname(imagePath).toLowerCase().replace(".", "") || "png";
  if (ext !== "png" && ext !== "jpg" && ext !== "jpeg") {
    throw new Error(`unsupported frame extension .${ext}`);
  }
  const index = String(state.frameCount).padStart(6, "0");
  const dest = path.join(state.frameDir, `${index}.${ext}`);
  fs.copyFileSync(imagePath, dest);
  const next: PersistedFrameSession = {
    ...state,
    frameCount: state.frameCount + 1,
  };
  writeState(next);
  return next;
}

export function markFrameSession(
  state: PersistedFrameSession,
  slug: string,
  note?: string,
): PersistedFrameSession {
  assertSlug(slug);
  const chapter: MovieChapter = {
    slug,
    tMs: Date.now() - state.startedAtMs,
    note,
  };
  const next: PersistedFrameSession = {
    ...state,
    chapters: [...state.chapters, chapter],
  };
  writeState(next);
  return next;
}

export async function stopFrameSession(
  state: PersistedFrameSession,
  options?: { status?: "running" | "pass" | "fail" | "idle" },
): Promise<MovieArtifact> {
  const frames = fs
    .readdirSync(state.frameDir)
    .filter((name) => /\.(png|jpe?g)$/i.test(name))
    .sort()
    .map((name) => path.join(state.frameDir, name));
  if (frames.length === 0) {
    throw new Error("cannot stop movie session with zero frames");
  }

  const work = path.join(sessionDir(state.root, state.feature, state.id), "out");
  ensureDir(work);
  const outExt = state.format === "mp4" ? ".mp4" : ".webm";
  const encoded = await encodeFrames({
    framePaths: frames,
    outPath: path.join(work, `movie${outExt}`),
    size: state.size,
    fps: state.fps,
  });
  const posterPath = posterFromFrames(frames, path.join(work, "poster.png"));
  const durationMs = encoded.durationMs;

  const published = sinkMovie({
    root: state.root,
    feature: state.feature,
    slug: state.slug,
    runId: state.runId,
    title: state.title,
    description: state.description,
    status: options?.status ?? "running",
    source: "frames",
    posterPath,
    videoPath: encoded.videoPath,
    durationMs,
    chapters: state.chapters,
    size: state.size,
  });

  // Drop session state; artifacts live in .astroshot/.
  fs.rmSync(sessionDir(state.root, state.feature, state.id), {
    recursive: true,
    force: true,
  });

  return {
    videoPath: published.videoDest,
    posterPath: published.posterDest,
    durationMs,
    chapters: state.chapters,
    source: "frames",
    feature: state.feature,
    slug: state.slug,
    sequence: published.sequence,
    runId: state.runId,
  };
}
