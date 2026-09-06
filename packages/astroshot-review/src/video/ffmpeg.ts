/**
 * Movie playback through ffmpeg: decode the file at a modest frame rate,
 * scaled to the stage, as a stream of PNG frames the terminal can draw with
 * the graphics protocol. Seeking restarts the decoder at the new offset.
 */
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { fitInside, readPngSize, type ImageSize } from "../images/png.js";

const EXTRA_BIN_DIRS = ["/opt/homebrew/bin", "/usr/local/bin"];
const PNG_END = Buffer.from([0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82]);

export interface FfmpegInfo {
  ffmpeg: string | null;
  ffprobe: string | null;
  version: string | null;
}

let cachedInfo: FfmpegInfo | null = null;

function findBinary(name: string, env: NodeJS.ProcessEnv): string | null {
  const dirs = [...(env.PATH ?? "").split(path.delimiter), ...EXTRA_BIN_DIRS].filter(Boolean);
  for (const dir of dirs) {
    const candidate = path.join(dir, name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return candidate;
    } catch {
      continue;
    }
  }
  return null;
}

export function detectFfmpeg(env: NodeJS.ProcessEnv = process.env): FfmpegInfo {
  if (cachedInfo) return cachedInfo;
  const ffmpeg = env.ASTROSHOT_REVIEW_FFMPEG ?? findBinary("ffmpeg", env);
  const ffprobe = findBinary("ffprobe", env);
  let version: string | null = null;
  if (ffmpeg) {
    const result = spawnSync(ffmpeg, ["-version"], { encoding: "utf8" });
    version = /ffmpeg version (\S+)/.exec(result.stdout ?? "")?.[1] ?? null;
  }
  cachedInfo = { ffmpeg, ffprobe, version };
  return cachedInfo;
}

export function resetFfmpegCache(): void {
  cachedInfo = null;
}

export interface VideoInfo {
  width: number;
  height: number;
  durationMs: number | null;
}

export async function probeVideo(videoPath: string, info = detectFfmpeg()): Promise<VideoInfo | null> {
  if (!info.ffprobe) return null;
  return new Promise((resolve) => {
    const child = spawn(info.ffprobe!, [
      "-v",
      "error",
      "-select_streams",
      "v:0",
      "-show_entries",
      "stream=width,height:format=duration",
      "-of",
      "json",
      videoPath,
    ]);
    let output = "";
    child.stdout.on("data", (chunk: Buffer) => {
      output += chunk.toString();
    });
    child.on("error", () => resolve(null));
    child.on("close", () => {
      try {
        const parsed = JSON.parse(output) as {
          streams?: Array<{ width?: number; height?: number }>;
          format?: { duration?: string };
        };
        const stream = parsed.streams?.[0];
        if (!stream?.width || !stream.height) return resolve(null);
        const duration = Number(parsed.format?.duration);
        resolve({
          width: stream.width,
          height: stream.height,
          durationMs: Number.isFinite(duration) && duration > 0 ? Math.round(duration * 1000) : null,
        });
      } catch {
        resolve(null);
      }
    });
  });
}

export interface VideoFrame {
  png: Buffer;
  width: number;
  height: number;
  /** Presentation time in milliseconds. */
  tMs: number;
  index: number;
}

export interface FramePlayerOptions {
  videoPath: string;
  /** Pixel box the frames must fit inside. */
  bounds: ImageSize;
  sourceSize: ImageSize;
  fps?: number;
  startMs?: number;
  onFrame: (frame: VideoFrame) => void;
  onEnd: () => void;
  onError: (error: Error) => void;
  ffmpegPath?: string;
}

/** Splits a concatenated PNG byte stream into whole files. */
export function splitPngStream(buffer: Buffer): { frames: Buffer[]; rest: Buffer } {
  const frames: Buffer[] = [];
  let cursor = 0;
  while (cursor < buffer.length) {
    const end = buffer.indexOf(PNG_END, cursor);
    if (end === -1) break;
    frames.push(buffer.subarray(cursor, end + PNG_END.length));
    cursor = end + PNG_END.length;
  }
  return { frames, rest: buffer.subarray(cursor) };
}

export class FramePlayer {
  private child: ChildProcess | null = null;
  private pending: Buffer = Buffer.alloc(0);
  private index = 0;
  private readonly fps: number;
  private readonly startMs: number;
  private stopped = false;
  readonly frameSize: ImageSize;

  constructor(private readonly options: FramePlayerOptions) {
    this.fps = options.fps ?? 12;
    this.startMs = options.startMs ?? 0;
    this.frameSize = fitInside(options.sourceSize, options.bounds);
    // Even dimensions keep every encoder happy.
    this.frameSize = {
      width: Math.max(2, this.frameSize.width - (this.frameSize.width % 2)),
      height: Math.max(2, this.frameSize.height - (this.frameSize.height % 2)),
    };
  }

  start(): void {
    const ffmpeg = this.options.ffmpegPath ?? detectFfmpeg().ffmpeg;
    if (!ffmpeg) {
      this.options.onError(new Error("ffmpeg is not installed"));
      return;
    }
    const args = [
      "-hide_banner",
      "-loglevel",
      "error",
      "-nostdin",
      "-re",
      "-ss",
      (this.startMs / 1000).toFixed(3),
      "-i",
      this.options.videoPath,
      "-an",
      "-vf",
      `fps=${this.fps},scale=${this.frameSize.width}:${this.frameSize.height}:flags=fast_bilinear`,
      "-f",
      "image2pipe",
      "-vcodec",
      "png",
      "-compression_level",
      "3",
      "-",
    ];
    const child = spawn(ffmpeg, args, { stdio: ["ignore", "pipe", "pipe"] });
    this.child = child;
    let stderr = "";
    child.stderr?.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
    });
    child.stdout?.on("data", (chunk: Buffer) => {
      if (this.stopped) return;
      this.pending = this.pending.length ? Buffer.concat([this.pending, chunk]) : chunk;
      const { frames, rest } = splitPngStream(this.pending);
      this.pending = rest;
      for (const png of frames) {
        const size = readPngSize(png);
        if (!size) continue;
        const frame: VideoFrame = {
          png,
          width: size.width,
          height: size.height,
          tMs: this.startMs + Math.round((this.index * 1000) / this.fps),
          index: this.index,
        };
        this.index += 1;
        this.options.onFrame(frame);
      }
    });
    child.on("error", (error) => {
      if (this.stopped) return;
      this.stopped = true;
      this.options.onError(error);
    });
    child.on("close", (code) => {
      if (this.stopped) return;
      this.stopped = true;
      if (code && code !== 0) {
        this.options.onError(new Error(`ffmpeg exited with ${code}: ${stderr.trim().slice(0, 300)}`));
      } else {
        this.options.onEnd();
      }
    });
  }

  /** Time of the last delivered frame, in milliseconds. */
  get positionMs(): number {
    return this.startMs + Math.round((Math.max(0, this.index - 1) * 1000) / this.fps);
  }

  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    const child = this.child;
    this.child = null;
    if (child && child.exitCode === null) {
      try {
        child.kill("SIGKILL");
      } catch {
        // Already gone.
      }
    }
  }
}
