/**
 * Recursive filesystem watching over the roots, routed the same way the app
 * routes FSEvents: image files ingest individually, sidecars refresh their
 * feature, friction-log paths refresh the whole friction namespace, and new
 * or vanished `.astroshot` directories rescan the tree.
 */
import fs from "node:fs";
import path from "node:path";

import { ASTROSHOT_DIR, FRICTION_DIR, VIDEO_EXTENSIONS, extensionOf, isImageFile } from "./paths.js";

export type WatchEvent =
  | { kind: "shot"; path: string; featureDir: string; astroshotDir: string }
  | { kind: "feature"; featureDir: string; astroshotDir: string }
  | { kind: "friction"; astroshotDir: string }
  | { kind: "tree"; astroshotDir: string };

export function classifyPath(fullPath: string): WatchEvent | null {
  const parts = fullPath.split(path.sep);
  const index = parts.indexOf(ASTROSHOT_DIR);
  if (index === -1) return null;
  const astroshotDir = parts.slice(0, index + 1).join(path.sep);
  const rest = parts.slice(index + 1);
  if (rest.length === 0) return { kind: "tree", astroshotDir };
  if (rest[0] === FRICTION_DIR) return { kind: "friction", astroshotDir };
  if (rest[0]!.startsWith(".")) return null;
  const featureDir = path.join(astroshotDir, rest[0]!);
  if (rest.length === 1) return { kind: "feature", featureDir, astroshotDir };
  if (rest.length === 2) {
    const name = rest[1]!;
    if (name === "manifest.json" || name === "review.json") return { kind: "feature", featureDir, astroshotDir };
    if (isImageFile(name)) return { kind: "shot", path: fullPath, featureDir, astroshotDir };
    if (VIDEO_EXTENSIONS.includes(extensionOf(name))) return { kind: "feature", featureDir, astroshotDir };
  }
  return null;
}

export function eventKey(event: WatchEvent): string {
  switch (event.kind) {
    case "shot":
      return `shot:${event.path}`;
    case "feature":
      return `feature:${event.featureDir}`;
    case "friction":
      return `friction:${event.astroshotDir}`;
    case "tree":
      return `tree:${event.astroshotDir}`;
  }
}

export interface WatchOptions {
  settleMs?: number;
  onError?: (root: string, error: Error) => void;
  /**
   * Watch each root recursively. Cheap where the OS offers a single
   * recursive stream (FSEvents on macOS, ReadDirectoryChangesW on Windows);
   * elsewhere every directory costs an inotify watch, so only discovered
   * `.astroshot` trees are watched and new trees need a rescan.
   */
  recursiveRoots?: boolean;
}

export interface RootWatcher {
  close(): void;
  /** Whether at least one watch is live. */
  readonly supported: boolean;
  /** Follow one `.astroshot` tree (no-op when roots are watched recursively). */
  watchTree(astroshotDir: string): void;
}

export function watchRoots(
  roots: string[],
  onEvent: (event: WatchEvent) => void,
  options: WatchOptions = {},
): RootWatcher {
  const settleMs = options.settleMs ?? 250;
  const recursiveRoots = options.recursiveRoots ?? (process.platform === "darwin" || process.platform === "win32");
  const timers = new Map<string, NodeJS.Timeout>();
  const pending = new Map<string, WatchEvent>();
  const watchers = new Map<string, fs.FSWatcher>();
  let supported = true;

  const attach = (target: string, recursive: boolean): boolean => {
    if (watchers.has(target)) return true;
    try {
      const watcher = fs.watch(target, { recursive, persistent: false }, (_type, filename) => {
        if (!filename) return;
        const fullPath = path.join(target, filename.toString());
        const event = classifyPath(fullPath);
        if (event) schedule(event);
      });
      watcher.on("error", (error) => {
        watchers.delete(target);
        watcher.close();
        if (watchers.size === 0) supported = false;
        options.onError?.(target, error);
      });
      watchers.set(target, watcher);
      return true;
    } catch (error) {
      options.onError?.(target, error instanceof Error ? error : new Error(String(error)));
      return false;
    }
  };

  const schedule = (event: WatchEvent) => {
    const key = eventKey(event);
    pending.set(key, event);
    const existing = timers.get(key);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(() => {
      timers.delete(key);
      const queued = pending.get(key);
      pending.delete(key);
      if (queued) onEvent(queued);
    }, settleMs);
    timer.unref();
    timers.set(key, timer);
  };

  let attached = 0;
  for (const root of roots) {
    if (attach(root, recursiveRoots)) attached += 1;
  }
  supported = attached > 0;

  return {
    get supported() {
      return supported;
    },
    watchTree(astroshotDir) {
      if (recursiveRoots) return;
      if (attach(astroshotDir, true)) supported = true;
      // A sibling `.astroshot` appearing next to a known tree shows up too.
      attach(path.dirname(astroshotDir), false);
    },
    close() {
      for (const timer of timers.values()) clearTimeout(timer);
      timers.clear();
      for (const watcher of watchers.values()) watcher.close();
      watchers.clear();
    },
  };
}
