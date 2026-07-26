import fs from "node:fs";
import path from "node:path";

import type { BatchEntry } from "./types.js";

function collisionKey(filePath: string): string {
  // Keep manifests portable across case-sensitive and case-insensitive hosts.
  let existingAncestor = path.resolve(filePath);
  const missingSegments: string[] = [];
  while (!fs.existsSync(existingAncestor)) {
    const parent = path.dirname(existingAncestor);
    if (parent === existingAncestor) break;
    missingSegments.unshift(path.basename(existingAncestor));
    existingAncestor = parent;
  }
  const canonicalAncestor = fs.realpathSync.native(existingAncestor);
  return path
    .join(canonicalAncestor, ...missingSegments)
    .normalize("NFC")
    .toLowerCase();
}

/** Resolve every destination before capture and reject accidental overwrites. */
export function resolveBatchOutputPaths(
  entries: BatchEntry[],
  manifestDirectory: string,
): string[] {
  const destinations = entries.map((entry) =>
    path.resolve(manifestDirectory, entry.out),
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
