/**
 * The tray's model: every shot and friction log under the roots, newest
 * arrival first, kept fresh by the filesystem watcher, with the review
 * actions the UI exposes. Mutations run one at a time so a rescan and a
 * live event can never interleave.
 */
import path from "node:path";

import { LOG_FILE, loadFrictionLogs } from "./friction.js";
import { HashCache } from "./hash-cache.js";
import { loadIndex, reconcileArrivalOrder, saveIndex, type IndexDocument } from "./index-cache.js";
import type { AstroshotTree, FrictionLog, FrictionRun, Shot } from "./model.js";
import { MAX_SCAN_DEPTH, frictionLogsDir } from "./paths.js";
import { addComment, markSeen } from "./review-store.js";
import { findAstroshotDirs, rebuildShot, scanFeatureDir, scanTree } from "./scan.js";
import { watchRoots, type RootWatcher, type WatchEvent } from "./watcher.js";

export type ScanPhase = "idle" | "warm" | "shallow" | "full";

/** Depth of the quick pass that finds repo-level trees almost instantly. */
export const SHALLOW_DEPTH = 3;
/** A deep walk this recent is skipped at startup; `r` always forces one. */
export const FULL_SCAN_TTL_MS = 30 * 60 * 1000;

export interface StoreEvent {
  kind: "new-shot" | "updated-shot";
  shot: Shot;
  at: number;
}

export interface StoreState {
  roots: string[];
  shots: Shot[];
  frictionLogs: FrictionLog[];
  treeCount: number;
  scanning: boolean;
  phase: ScanPhase;
  watching: boolean;
  unreadCount: number;
  lastEvent: StoreEvent | null;
  error: string | null;
  revision: number;
}

export interface StoreOptions {
  roots: string[];
  indexPath?: string;
  /** Skip the durable index (tests). */
  useIndex?: boolean;
  watch?: boolean;
  settleMs?: number;
  onLog?: (message: string) => void;
}

type Listener = () => void;

export class ReviewStore {
  private state: StoreState;
  private readonly listeners = new Set<Listener>();
  private readonly trees = new Map<string, AstroshotTree>();
  private readonly shotsByPath = new Map<string, Shot>();
  private arrivalOrder: string[] = [];
  private readonly hashes: HashCache;
  private watcher: RootWatcher | null = null;
  private queue: Promise<void> = Promise.resolve();
  private readonly options: StoreOptions;
  private index: IndexDocument | null = null;
  private disposed = false;
  private saveTimer: NodeJS.Timeout | null = null;
  private fullScanAt: string | null = null;
  /** Arrival order frozen at scan start so batches do not reorder the stream. */
  private scanBaseOrder: string[] | null = null;

  constructor(options: StoreOptions) {
    this.options = options;
    this.hashes = new HashCache();
    this.state = {
      roots: options.roots,
      shots: [],
      frictionLogs: [],
      treeCount: 0,
      scanning: false,
      phase: "idle",
      watching: false,
      unreadCount: 0,
      lastEvent: null,
      error: null,
      revision: 0,
    };
  }

  getState(): StoreState {
    return this.state;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private publish(patch: Partial<StoreState> = {}): void {
    this.state = { ...this.state, ...patch, revision: this.state.revision + 1 };
    for (const listener of this.listeners) listener();
  }

  private log(message: string): void {
    this.options.onLog?.(message);
  }

  /** Serialize mutations. */
  private enqueue<T>(task: () => Promise<T>): Promise<T> {
    const run = this.queue.then(task, task);
    this.queue = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  async start(): Promise<void> {
    if (this.options.useIndex !== false) {
      this.index = await loadIndex(this.options.roots, this.options.indexPath);
      if (this.index) {
        this.arrivalOrder = this.index.arrivalOrder;
        this.hashes.seed(this.index.hashes);
        this.fullScanAt = this.index.fullScanAt ?? null;
      }
    }
    if (this.options.watch !== false) {
      this.watcher = watchRoots(
        this.options.roots,
        (event) => void this.handleEvent(event),
        {
          settleMs: this.options.settleMs,
          onError: (root, error) => {
            this.log(`watch ${root}: ${error.message}`);
            this.publish({ watching: this.watcher?.supported ?? false, error: `watch: ${error.message}` });
          },
        },
      );
      this.publish({ watching: this.watcher.supported });
    }
    await this.rescan();
  }

  /**
   * Warm scan from the index, a shallow walk for repo-level trees, then a
   * deep walk when the index is stale (or when forced by the user).
   */
  rescan(options: { force?: boolean } = {}): Promise<void> {
    return this.enqueue(async () => {
      const cachedDirs = this.index?.astroshotDirs ?? [];
      const known = new Set<string>();
      this.scanBaseOrder = [...this.arrivalOrder];
      if (cachedDirs.length > 0) {
        this.publish({ scanning: true, phase: "warm" });
        await this.scanTrees(cachedDirs);
        for (const dir of cachedDirs) if (this.trees.has(dir)) known.add(dir);
        this.recompute();
      }

      const discover = async (maxDepth: number, phase: ScanPhase): Promise<Set<string>> => {
        this.publish({ scanning: true, phase });
        const discovered = new Set<string>();
        const pending: string[] = [];
        let flushing: Promise<void> = Promise.resolve();
        await findAstroshotDirs(this.options.roots, {
          maxDepth,
          concurrency: phase === "full" ? 6 : 16,
          onFound: (astroshotDir) => {
            discovered.add(astroshotDir);
            if (known.has(astroshotDir)) return;
            known.add(astroshotDir);
            pending.push(astroshotDir);
            // Stream results into the tray while the walk continues.
            flushing = flushing.then(async () => {
              const batch = pending.splice(0, pending.length);
              if (batch.length === 0) return;
              await this.scanTrees(batch);
              this.recompute();
            });
          },
        });
        await flushing;
        return discovered;
      };

      const shallow = await discover(SHALLOW_DEPTH, "shallow");
      const stale =
        options.force ||
        !this.fullScanAt ||
        Date.now() - Date.parse(this.fullScanAt) > FULL_SCAN_TTL_MS ||
        Number.isNaN(Date.parse(this.fullScanAt));
      let complete = shallow;
      if (stale) {
        complete = await discover(MAX_SCAN_DEPTH, "full");
        this.fullScanAt = new Date().toISOString();
      }
      // Trees that vanished from disk leave the stream; cached deep trees
      // survive a shallow-only start because they were re-verified above.
      for (const dir of [...this.trees.keys()]) {
        if (!complete.has(dir) && (stale || !cachedDirs.includes(dir))) this.trees.delete(dir);
      }
      // Cached trees may have changed while the tray was closed.
      await this.scanTrees(cachedDirs.filter((dir) => this.trees.has(dir)));
      this.recompute();
      this.scanBaseOrder = null;
      this.publish({ scanning: false, phase: "idle" });
      this.scheduleSave();
    });
  }

  private async scanTrees(dirs: string[]): Promise<void> {
    const concurrency = 6;
    let cursor = 0;
    const worker = async () => {
      while (cursor < dirs.length) {
        const dir = dirs[cursor++]!;
        try {
          const tree = await scanTree(dir, this.hashes);
          this.trees.set(dir, tree);
          this.watcher?.watchTree(dir);
        } catch (error) {
          this.log(`scan ${dir}: ${error instanceof Error ? error.message : String(error)}`);
        }
        if (cursor % 8 === 0) this.recompute();
      }
    };
    await Promise.all(Array.from({ length: Math.min(concurrency, dirs.length) }, worker));
  }

  /** Rebuild the flat, ordered shot and friction lists from the trees. */
  private recompute(): void {
    const all: Shot[] = [];
    const frictionLogs: FrictionLog[] = [];
    for (const tree of this.trees.values()) {
      all.push(...tree.shots);
      frictionLogs.push(...tree.frictionLogs);
    }
    this.shotsByPath.clear();
    for (const shot of all) this.shotsByPath.set(shot.path, shot);
    this.arrivalOrder = reconcileArrivalOrder(this.scanBaseOrder ?? this.arrivalOrder, all);
    frictionLogs.sort((a, b) => b.updatedAt - a.updatedAt);
    this.publish({
      shots: this.orderedShots(),
      frictionLogs,
      treeCount: this.trees.size,
    });
  }

  private orderedShots(): Shot[] {
    const shots: Shot[] = [];
    for (const shotPath of this.arrivalOrder) {
      const shot = this.shotsByPath.get(shotPath);
      if (shot) shots.push(shot);
    }
    return shots;
  }

  private scheduleSave(): void {
    if (this.options.useIndex === false) return;
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      this.saveTimer = null;
      void this.saveIndexNow();
    }, 500);
    this.saveTimer.unref();
  }

  private async saveIndexNow(): Promise<void> {
    this.hashes.retain(new Set(this.shotsByPath.keys()), (filePath) => filePath.endsWith("log.jsonl"));
    const document: IndexDocument = {
      version: 1,
      roots: this.options.roots,
      astroshotDirs: [...this.trees.keys()].sort(),
      arrivalOrder: this.arrivalOrder,
      hashes: this.hashes.toJSON(),
      updatedAt: new Date().toISOString(),
      fullScanAt: this.fullScanAt ?? undefined,
    };
    try {
      await saveIndex(document, this.options.indexPath);
      this.index = document;
    } catch (error) {
      this.log(`index save: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  handleEvent(event: WatchEvent): Promise<void> {
    return this.enqueue(async () => {
      try {
        switch (event.kind) {
          case "shot":
            await this.ingestShot(event);
            break;
          case "feature":
            await this.refreshFeature(event.featureDir, event.astroshotDir);
            break;
          case "friction":
            await this.refreshFriction(event.astroshotDir);
            break;
          case "tree":
            await this.refreshTree(event.astroshotDir);
            break;
        }
      } catch (error) {
        this.log(`event ${event.kind}: ${error instanceof Error ? error.message : String(error)}`);
      }
      this.scheduleSave();
    });
  }

  private treeFor(astroshotDir: string): AstroshotTree {
    const existing = this.trees.get(astroshotDir);
    if (existing) return existing;
    const worktreePath = path.dirname(astroshotDir);
    const tree: AstroshotTree = {
      astroshotDir,
      worktreePath,
      worktree: path.basename(worktreePath),
      shots: [],
      frictionLogs: [],
    };
    this.trees.set(astroshotDir, tree);
    return tree;
  }

  private async ingestShot(event: Extract<WatchEvent, { kind: "shot" }>): Promise<void> {
    const tree = this.treeFor(event.astroshotDir);
    const shot = await rebuildShot(
      event.path,
      {
        worktreePath: tree.worktreePath,
        worktree: tree.worktree,
        feature: path.basename(event.featureDir),
        featureDir: event.featureDir,
      },
      this.hashes,
    );
    const wasKnown = this.shotsByPath.has(event.path);
    tree.shots = tree.shots.filter((candidate) => candidate.path !== event.path);
    if (!shot) {
      // Image vanished: drop it everywhere.
      this.shotsByPath.delete(event.path);
      this.arrivalOrder = this.arrivalOrder.filter((entry) => entry !== event.path);
      this.publish({ shots: this.orderedShots() });
      return;
    }
    tree.shots.push(shot);
    this.shotsByPath.set(shot.path, shot);
    this.arrivalOrder = [shot.path, ...this.arrivalOrder.filter((entry) => entry !== shot.path)];
    this.publish({
      shots: this.orderedShots(),
      treeCount: this.trees.size,
      unreadCount: wasKnown ? this.state.unreadCount : this.state.unreadCount + 1,
      lastEvent: { kind: wasKnown ? "updated-shot" : "new-shot", shot, at: Date.now() },
    });
  }

  private async refreshFeature(featureDir: string, astroshotDir: string): Promise<void> {
    const tree = this.treeFor(astroshotDir);
    const shots = await scanFeatureDir(featureDir, { worktreePath: tree.worktreePath, worktree: tree.worktree }, this.hashes);
    const previous = new Set(tree.shots.filter((shot) => shot.featureDir === featureDir).map((shot) => shot.path));
    tree.shots = [...tree.shots.filter((shot) => shot.featureDir !== featureDir), ...shots];
    const fresh = shots.filter((shot) => !previous.has(shot.path));
    this.recompute();
    if (fresh.length > 0) {
      const newest = fresh.sort((a, b) => b.capturedAt - a.capturedAt)[0]!;
      this.publish({
        unreadCount: this.state.unreadCount + fresh.length,
        lastEvent: { kind: "new-shot", shot: newest, at: Date.now() },
      });
    }
  }

  private async refreshFriction(astroshotDir: string): Promise<void> {
    const tree = this.treeFor(astroshotDir);
    tree.frictionLogs = await loadFrictionLogs(
      frictionLogsDir(astroshotDir),
      { worktreePath: tree.worktreePath, worktree: tree.worktree },
      this.hashes,
    );
    this.recompute();
  }

  private async refreshTree(astroshotDir: string): Promise<void> {
    try {
      const tree = await scanTree(astroshotDir, this.hashes);
      if (tree.shots.length === 0 && tree.frictionLogs.length === 0) {
        this.trees.delete(astroshotDir);
      } else {
        this.trees.set(astroshotDir, tree);
      }
    } catch {
      this.trees.delete(astroshotDir);
    }
    this.recompute();
  }

  /** The user opened the stream: clear the unread badge. */
  markOpened(): void {
    if (this.state.unreadCount !== 0) this.publish({ unreadCount: 0 });
  }

  async markShotSeen(shot: Shot, comment?: string): Promise<Shot> {
    await markSeen(
      { directory: shot.featureDir, fileName: shot.fileName, runId: shot.runId, targetPath: shot.path },
      { comment },
    );
    return this.reloadShot(shot);
  }

  async addShotComment(shot: Shot, body: string): Promise<Shot> {
    await addComment(
      { directory: shot.featureDir, fileName: shot.fileName, runId: shot.runId, targetPath: shot.path },
      body,
    );
    return this.reloadShot(shot);
  }

  private reloadShot(shot: Shot): Promise<Shot> {
    return this.enqueue(async () => {
      const tree = this.treeFor(path.dirname(shot.featureDir));
      const rebuilt = await rebuildShot(
        shot.path,
        { worktreePath: shot.worktreePath, worktree: shot.worktree, feature: shot.feature, featureDir: shot.featureDir },
        this.hashes,
      );
      if (!rebuilt) return shot;
      tree.shots = tree.shots.map((candidate) => (candidate.path === shot.path ? rebuilt : candidate));
      if (!tree.shots.includes(rebuilt)) tree.shots.push(rebuilt);
      this.shotsByPath.set(rebuilt.path, rebuilt);
      // Sidecar writes for one shot also re-scope siblings on a run change.
      this.publish({ shots: this.orderedShots() });
      return rebuilt;
    });
  }

  /** Mark many shots seen; failures are counted, never fatal. */
  async markManySeen(shots: Shot[]): Promise<{ ok: number; failed: number }> {
    let ok = 0;
    let failed = 0;
    for (const shot of shots) {
      try {
        await this.markShotSeen(shot);
        ok += 1;
      } catch {
        failed += 1;
      }
    }
    // Reload every touched feature so run resets propagate.
    const features = new Map<string, Shot>();
    for (const shot of shots) features.set(shot.featureDir, shot);
    for (const shot of features.values()) {
      await this.handleEvent({ kind: "feature", featureDir: shot.featureDir, astroshotDir: path.dirname(shot.featureDir) });
    }
    return { ok, failed };
  }

  async markFrictionRunSeen(log: FrictionLog, run: FrictionRun): Promise<void> {
    if (!run.logPath) throw new Error("This run has no log.jsonl to acknowledge");
    await markSeen({ directory: run.directory, fileName: LOG_FILE, runId: run.runId, targetPath: run.logPath });
    await this.handleEvent({ kind: "friction", astroshotDir: path.join(log.worktreePath, ".astroshot") });
  }

  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;
    this.watcher?.close();
    if (this.saveTimer) {
      clearTimeout(this.saveTimer);
      this.saveTimer = null;
      await this.saveIndexNow();
    }
  }
}
