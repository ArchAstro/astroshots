import path from "node:path";

import type { BatchEntry } from "./types.js";

function collisionKey(filePath: string): string {
  // Reject case-only collisions on every platform so a manifest behaves the
  // same on case-sensitive and case-insensitive filesystems.
  return path.resolve(filePath).normalize("NFC").toLowerCase();
}

function safeRelativeOutput(output: string): string {
  const portable = output.replaceAll("\\", "/");
  const normalized = path.posix.normalize(portable);
  if (
    path.posix.isAbsolute(normalized) ||
    path.win32.isAbsolute(output) ||
    normalized === ".." ||
    normalized.startsWith("../")
  ) {
    throw new Error(
      `Batch output must be a safe relative path when --out-dir is used: ${output}`,
    );
  }
  return normalized;
}

/** Resolve batch destinations once, rejecting traversal and overwrite risks. */
export function resolveBatchOutputPaths(
  entries: BatchEntry[],
  manifestDir: string,
  outDir: string | null,
): string[] {
  const destinations = entries.map((entry) =>
    outDir
      ? path.resolve(outDir, safeRelativeOutput(entry.out))
      : path.resolve(manifestDir, entry.out),
  );
  const seen = new Map<string, string>();
  for (const destination of destinations) {
    const key = collisionKey(destination);
    const previous = seen.get(key);
    if (previous) {
      throw new Error(
        `Batch outputs resolve to the same destination: ${previous} and ${destination}`,
      );
    }
    seen.set(key, destination);
  }
  return destinations;
}
