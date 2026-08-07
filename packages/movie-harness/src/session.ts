import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { encodeFrames, posterFromFrames } from "./encode.js";
import {
  assertKebabCase,
  assertSlug,
  defaultRunId,
  ensureDir,
  resolveRoot,
} from "./paths.js";
import { finalizeManifest, sinkMovie } from "./sink.js";
import type {
  ManifestStatus,
  MovieArtifact,
  MovieChapter,
  MovieSessionOptions,
  MovieSourceKind,
  Size,
} from "./types.js";

export class MovieSession {
  readonly feature: string;
  readonly slug: string;
  readonly root: string;
  readonly runId: string;
  readonly title?: string;
  readonly description?: string;
  readonly size: Size;
  readonly fps: number;
  readonly format: "webm" | "mp4";
  readonly source: MovieSourceKind;

  private readonly workDir: string;
  private readonly frameDir: string;
  private readonly startedAtMs: number;
  private frameCount = 0;
  private chapters: MovieChapter[] = [];
  private stopped = false;
  private status: ManifestStatus;

  private constructor(options: MovieSessionOptions, workDir: string) {
    this.feature = options.feature;
    this.slug = options.slug;
    this.root = resolveRoot(options.root);
    this.runId = options.runId ?? defaultRunId(options.feature);
    this.title = options.title;
    this.description = options.description;
    this.size = options.size ?? { width: 1280, height: 720 };
    this.fps = options.fps ?? 15;
    this.format = options.format ?? "webm";
    this.source = options.source;
    this.status = options.status ?? "running";
    this.workDir = workDir;
    this.frameDir = path.join(workDir, "frames");
    this.startedAtMs = Date.now();
    ensureDir(this.frameDir);
  }

  static create(options: MovieSessionOptions): MovieSession {
    assertKebabCase(options.feature, "feature");
    assertSlug(options.slug);
    if (options.fps !== undefined && !(options.fps > 0 && options.fps <= 60)) {
      throw new Error("fps must be in (0, 60]");
    }
    const workDir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-movie-"));
    return new MovieSession(options, workDir);
  }

  /** Elapsed ms since session start (for chapter timestamps). */
  elapsedMs(): number {
    return Date.now() - this.startedAtMs;
  }

  mark(slug: string, note?: string): void {
    this.assertOpen();
    assertSlug(slug);
    this.chapters.push({
      slug,
      tMs: this.elapsedMs(),
      note,
    });
  }

  /**
   * Push a PNG (or JPEG) frame into the session. Bytes are written to disk
   * immediately so a crash still leaves a recoverable sequence.
   */
  pushFrame(image: Buffer, extension: "png" | "jpg" | "jpeg" = "png"): string {
    this.assertOpen();
    if (image.length < 8) {
      throw new Error("pushFrame: empty image buffer");
    }
    const index = String(this.frameCount).padStart(6, "0");
    const file = path.join(this.frameDir, `${index}.${extension}`);
    fs.writeFileSync(file, image);
    this.frameCount += 1;
    return file;
  }

  pushFrameFile(imagePath: string): string {
    this.assertOpen();
    const bytes = fs.readFileSync(imagePath);
    const ext = path.extname(imagePath).toLowerCase().replace(".", "");
    if (ext !== "png" && ext !== "jpg" && ext !== "jpeg") {
      throw new Error(`pushFrameFile: unsupported extension .${ext}`);
    }
    return this.pushFrame(bytes, ext as "png" | "jpg" | "jpeg");
  }

  listFrames(): string[] {
    if (!fs.existsSync(this.frameDir)) return [];
    return fs
      .readdirSync(this.frameDir)
      .filter((name) => /\.(png|jpe?g)$/i.test(name))
      .sort()
      .map((name) => path.join(this.frameDir, name));
  }

  /**
   * Encode collected frames, write poster + video into .astroshot/, update
   * manifest. Cleans the temp work directory afterward.
   */
  async stop(options?: {
    status?: ManifestStatus;
    /** Skip encode and only publish when frames already form the movie via another path. */
    videoPath?: string;
    posterPath?: string;
    durationMs?: number;
  }): Promise<MovieArtifact> {
    this.assertOpen();
    this.stopped = true;
    const status = options?.status ?? this.status;

    try {
      let videoPath = options?.videoPath;
      let posterPath = options?.posterPath;
      let durationMs = options?.durationMs;

      const frames = this.listFrames();

      if (!videoPath) {
        if (frames.length === 0) {
          throw new Error("MovieSession.stop: no frames and no videoPath");
        }
        const outExt = this.format === "mp4" ? ".mp4" : ".webm";
        const encoded = await encodeFrames({
          framePaths: frames,
          outPath: path.join(this.workDir, `movie${outExt}`),
          size: this.size,
          fps: this.fps,
        });
        videoPath = encoded.videoPath;
        durationMs = encoded.durationMs;
      }

      if (!posterPath) {
        if (frames.length === 0) {
          throw new Error("MovieSession.stop: no frames for poster");
        }
        posterPath = posterFromFrames(
          frames,
          path.join(this.workDir, "poster.png"),
        );
      }

      durationMs ??= this.elapsedMs();

      const published = sinkMovie({
        root: this.root,
        feature: this.feature,
        slug: this.slug,
        runId: this.runId,
        title: this.title,
        description: this.description,
        status,
        source: this.source,
        posterPath,
        videoPath,
        durationMs,
        chapters: this.chapters,
        size: this.size,
      });

      return {
        videoPath: published.videoDest,
        posterPath: published.posterDest,
        durationMs,
        chapters: this.chapters,
        source: this.source,
        feature: this.feature,
        slug: this.slug,
        sequence: published.sequence,
        runId: this.runId,
      };
    } finally {
      fs.rmSync(this.workDir, { recursive: true, force: true });
    }
  }

  async finalize(status: "pass" | "fail" | "idle"): Promise<void> {
    finalizeManifest(this.root, this.feature, this.runId, status);
  }

  private assertOpen(): void {
    if (this.stopped) {
      throw new Error("MovieSession is already stopped");
    }
  }
}
