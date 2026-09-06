/**
 * Prepares image bytes for the terminal: reads the file, decides whether the
 * original PNG is already small enough, otherwise downsamples on a worker
 * thread, and caches the result by file identity and target size.
 */
import fs from "node:fs";
import os from "node:os";
import { Worker } from "node:worker_threads";

import { fitInside, readPngSize, type ImageSize } from "./png.js";
import type { Rect } from "./scale.js";
import { scaleImage, type ScaledFormat } from "./scale.js";
import type { WorkerRequest, WorkerResponse } from "./worker.js";

export interface PreparedImage {
  key: string;
  path: string;
  /** Pixel size of the prepared payload. */
  width: number;
  height: number;
  /** Pixel size of the source file. */
  sourceWidth: number;
  sourceHeight: number;
  format: ScaledFormat;
  data: Buffer;
  /** True when `data` is the untouched file, so a file-path transmission is valid. */
  isOriginal: boolean;
  mtimeMs: number;
  size: number;
}

export interface ImageServiceOptions {
  /** 0 runs decode inline (tests); default is min(2, cpus-1). */
  workers?: number;
  /** Memory budget for prepared payloads. */
  cacheBytes?: number;
  /** A source at most this many times larger than the target is sent as-is. */
  passthroughRatio?: number;
}

interface PendingJob {
  resolve: (value: WorkerResponse) => void;
}

export class ImageService {
  private readonly cache = new Map<string, Promise<PreparedImage>>();
  private readonly cacheOrder: string[] = [];
  private cachedBytes = 0;
  private readonly cacheBytes: number;
  private readonly passthroughRatio: number;
  private readonly workerCount: number;
  private workers: Worker[] = [];
  private nextWorker = 0;
  private nextJobId = 1;
  private readonly pending = new Map<number, PendingJob>();
  private disposed = false;

  constructor(options: ImageServiceOptions = {}) {
    this.workerCount =
      options.workers ?? Math.max(0, Math.min(2, os.availableParallelism() - 1));
    this.cacheBytes = options.cacheBytes ?? 96 * 1024 * 1024;
    this.passthroughRatio = options.passthroughRatio ?? 1.35;
  }

  /** Identity + target size key. Callers can use it for placement bookkeeping. */
  static cacheKey(filePath: string, stat: { mtimeMs: number; size: number }, target: ImageSize, format: ScaledFormat, crop?: Rect): string {
    const cropKey = crop ? `|${Math.round(crop.x)},${Math.round(crop.y)},${Math.round(crop.width)},${Math.round(crop.height)}` : "";
    return `${filePath}|${Math.round(stat.mtimeMs)}|${stat.size}|${target.width}x${target.height}|${format}${cropKey}`;
  }

  async prepare(filePath: string, target: ImageSize, format: ScaledFormat = "png", crop?: Rect): Promise<PreparedImage> {
    const stat = await fs.promises.stat(filePath);
    const key = ImageService.cacheKey(filePath, stat, target, format, crop);
    const cached = this.cache.get(key);
    if (cached) {
      this.touch(key);
      return cached;
    }
    const job = this.build(filePath, stat, target, format, key, crop);
    this.cache.set(key, job);
    this.cacheOrder.push(key);
    job.then(
      (prepared) => this.account(key, prepared.data.length),
      () => this.evict(key),
    );
    return job;
  }

  private async build(
    filePath: string,
    stat: fs.Stats,
    target: ImageSize,
    format: ScaledFormat,
    key: string,
    crop?: Rect,
  ): Promise<PreparedImage> {
    const bytes = await fs.promises.readFile(filePath);
    const source = readPngSize(bytes);
    if (!source) throw new Error(`Not a PNG image: ${filePath}`);
    const base = {
      key,
      path: filePath,
      sourceWidth: source.width,
      sourceHeight: source.height,
      mtimeMs: stat.mtimeMs,
      size: stat.size,
    };
    if (!crop) {
      const fitted = fitInside(source, target);
      const ratio = Math.max(source.width / fitted.width, source.height / fitted.height);
      if (format === "png" && ratio <= this.passthroughRatio) {
        return { ...base, width: source.width, height: source.height, format: "png", data: bytes, isOriginal: true };
      }
    }
    const scaled = await this.scale(bytes, target, format, crop);
    return { ...base, ...scaled, isOriginal: false };
  }

  private async scale(bytes: Buffer, target: ImageSize, format: ScaledFormat, crop?: Rect) {
    if (this.workerCount === 0) {
      const scaled = scaleImage({ bytes, target, format, crop });
      return { width: scaled.width, height: scaled.height, format: scaled.format, data: scaled.data };
    }
    const worker = this.pickWorker();
    const id = this.nextJobId++;
    const payload = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
    const response = await new Promise<WorkerResponse>((resolve) => {
      this.pending.set(id, { resolve });
      const request: WorkerRequest = {
        id,
        bytes: payload,
        targetWidth: target.width,
        targetHeight: target.height,
        format,
        crop,
      };
      worker.postMessage(request, [payload]);
    });
    if (!response.ok) throw new Error(response.error);
    return {
      width: response.width,
      height: response.height,
      format: response.format,
      data: Buffer.from(response.data),
    };
  }

  private pickWorker(): Worker {
    if (this.workers.length < this.workerCount) {
      const worker = new Worker(new URL("./worker.js", import.meta.url));
      worker.unref();
      worker.on("message", (response: WorkerResponse) => {
        const job = this.pending.get(response.id);
        if (!job) return;
        this.pending.delete(response.id);
        job.resolve(response);
      });
      worker.on("error", (error) => {
        for (const [id, job] of this.pending) {
          job.resolve({ id, ok: false, error: error.message });
        }
        this.pending.clear();
        this.workers = this.workers.filter((candidate) => candidate !== worker);
      });
      this.workers.push(worker);
      return worker;
    }
    const worker = this.workers[this.nextWorker % this.workers.length]!;
    this.nextWorker += 1;
    return worker;
  }

  private touch(key: string) {
    const index = this.cacheOrder.indexOf(key);
    if (index !== -1) {
      this.cacheOrder.splice(index, 1);
      this.cacheOrder.push(key);
    }
  }

  private account(key: string, bytes: number) {
    this.cachedBytes += bytes;
    while (this.cachedBytes > this.cacheBytes && this.cacheOrder.length > 1) {
      const oldest = this.cacheOrder[0]!;
      if (oldest === key) break;
      this.evict(oldest);
    }
  }

  private evict(key: string) {
    const job = this.cache.get(key);
    if (!job) return;
    this.cache.delete(key);
    const index = this.cacheOrder.indexOf(key);
    if (index !== -1) this.cacheOrder.splice(index, 1);
    job.then(
      (prepared) => {
        this.cachedBytes -= prepared.data.length;
      },
      () => undefined,
    );
  }

  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;
    await Promise.all(this.workers.map((worker) => worker.terminate()));
    this.workers = [];
    this.cache.clear();
    this.cacheOrder.length = 0;
    this.cachedBytes = 0;
  }
}
