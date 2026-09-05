import fs from "node:fs";

import { sha256File } from "./review-store.js";

export interface HashRecord {
  mtimeMs: number;
  size: number;
  sha256: string;
}

/** Memoizes file hashes by (path, mtime, size) so rescans stay cheap. */
export class HashCache {
  private readonly records = new Map<string, HashRecord>();
  private readonly inFlight = new Map<string, Promise<string>>();

  constructor(initial: Record<string, HashRecord> = {}) {
    for (const [filePath, record] of Object.entries(initial)) {
      this.records.set(filePath, record);
    }
  }

  async hash(filePath: string, stat?: fs.Stats): Promise<string> {
    const info = stat ?? (await fs.promises.stat(filePath));
    const cached = this.records.get(filePath);
    if (cached && cached.mtimeMs === info.mtimeMs && cached.size === info.size) {
      return cached.sha256;
    }
    const pending = this.inFlight.get(filePath);
    if (pending) return pending;
    const job = sha256File(filePath).then((sha256) => {
      this.records.set(filePath, { mtimeMs: info.mtimeMs, size: info.size, sha256 });
      this.inFlight.delete(filePath);
      return sha256;
    });
    this.inFlight.set(filePath, job);
    try {
      return await job;
    } catch (error) {
      this.inFlight.delete(filePath);
      throw error;
    }
  }

  seed(records: Record<string, HashRecord>): void {
    for (const [filePath, record] of Object.entries(records)) {
      if (!this.records.has(filePath)) this.records.set(filePath, record);
    }
  }

  /** Drop records for files that are no longer part of the stream. */
  retain(keep: Set<string>, alsoKeep: (filePath: string) => boolean = () => false): void {
    for (const filePath of [...this.records.keys()]) {
      if (!keep.has(filePath) && !alsoKeep(filePath)) this.records.delete(filePath);
    }
  }

  toJSON(): Record<string, HashRecord> {
    return Object.fromEntries(this.records);
  }
}
