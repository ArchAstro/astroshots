/**
 * The tray shell: header, tabs, the active pane, and the keymap. Navigation
 * follows the macOS app — Detail pages the whole stream newest-first while
 * full-screen review pages run siblings oldest-first.
 */
import fs from "node:fs";
import path from "node:path";

import { Box, Text, useApp, useInput } from "ink";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { FrictionLog, FrictionRun, Shot } from "../data/model.js";
import { HintBar, Toast, type KeyHint } from "./chrome.js";
import { useServices } from "./context.js";
import { DetailPane } from "./detail.js";
import {
  FRICTION_ROW_HEIGHT,
  FrictionFilterBar,
  FrictionList,
  FrictionLogDetail,
  FrictionStepDetail,
  FrictionStepTakeover,
  frictionScroll,
} from "./friction.js";
import { HelpOverlay } from "./help.js";
import { useClock, useStoreState, useTerminalSize } from "./hooks.js";
import type { PlaybackState } from "./movie-player.js";
import {
  contiguousGroups,
  filterFrictionLogs,
  filterShots,
  frictionState,
  latestRun,
  reviewSiblings,
  reviewStateOf,
  streamCounts,
  type StreamFilter,
} from "./selectors.js";
import { SettingsPane } from "./settings.js";
import { FilterBar, StreamList, flattenStream, nextNavigable, scrollWindow } from "./stream.js";
import { copyImageToClipboard, openWithDefaultApp, revealInFileManager } from "./system.js";
import { ReviewTakeover } from "./takeover.js";
import { theme, truncate } from "./theme.js";

type Tab = "shots" | "frictionLogs";
type Pane = "stream" | "detail" | "settings" | "frictionLog" | "frictionStep";
type Takeover = { kind: "shot" } | { kind: "step" } | null;

interface UiState {
  tab: Tab;
  pane: Pane;
  streamFilter: StreamFilter;
  moviesOnly: boolean;
  collapsed: Set<string>;
  cursor: number;
  scrollTop: number;
  selectedShot: string | null;
  frictionFilter: StreamFilter;
  frictionCursor: number;
  frictionScrollTop: number;
  selectedLog: string | null;
  selectedRun: string | null;
  stepCursor: number;
  stepIndex: number;
  imageIndex: number;
  promptOpen: boolean;
  prompt: string | null;
  takeover: Takeover;
  composer: boolean;
  help: boolean;
  toast: { message: string; at: number; durationMs: number } | null;
  busy: boolean;
  bulkBusy: boolean;
  error: string | null;
  inlinePlayer: boolean;
  playback: PlaybackState;
}

const SPLIT_BREAKPOINT = 120;
const SEEK_STEP_MS = 5000;

function initialPlayback(): PlaybackState {
  return { playing: false, positionMs: 0, durationMs: null, seekToken: 0, error: null, ended: false };
}

const initialState: UiState = {
  tab: "shots",
  pane: "stream",
  streamFilter: "unseen",
  moviesOnly: false,
  collapsed: new Set(),
  cursor: 0,
  scrollTop: 0,
  selectedShot: null,
  frictionFilter: "unseen",
  frictionCursor: 0,
  frictionScrollTop: 0,
  selectedLog: null,
  selectedRun: null,
  stepCursor: 0,
  stepIndex: 0,
  imageIndex: 0,
  promptOpen: false,
  prompt: null,
  takeover: null,
  composer: false,
  help: false,
  toast: null,
  busy: false,
  bulkBusy: false,
  error: null,
  inlinePlayer: false,
  playback: initialPlayback(),
};

export interface AppProps {
  onQuit?: () => void;
}

export function App({ onQuit }: AppProps) {
  const { store, layer, capabilities } = useServices();
  const { exit } = useApp();
  const storeState = useStoreState();
  const { columns, rows } = useTerminalSize();
  const now = useClock(30_000);
  const [ui, setUi] = useState<UiState>(initialState);
  const uiRef = useRef(ui);
  uiRef.current = ui;
  const patch = useCallback((update: Partial<UiState> | ((previous: UiState) => Partial<UiState>)) => {
    setUi((previous) => ({ ...previous, ...(typeof update === "function" ? update(previous) : update) }));
  }, []);

  const toast = useCallback(
    (message: string, durationMs = 1600) => patch({ toast: { message, at: Date.now(), durationMs } }),
    [patch],
  );

  // Toasts clear themselves like the app's 1.6 s pill; arrivals linger like the overlay.
  useEffect(() => {
    if (!ui.toast) return;
    const timer = setTimeout(
      () => patch((previous) => (previous.toast === ui.toast ? { toast: null } : {})),
      ui.toast.durationMs,
    );
    return () => clearTimeout(timer);
  }, [ui.toast, patch]);

  // Overlay stand-in: announce arrivals while the tray is open.
  const lastEventRef = useRef<number>(0);
  useEffect(() => {
    const event = storeState.lastEvent;
    if (!event || event.at === lastEventRef.current) return;
    lastEventRef.current = event.at;
    if (event.kind === "new-shot") toast(`New · ${event.shot.worktreeShort} · ${event.shot.feature} · ${event.shot.title}`, 5500);
  }, [storeState.lastEvent, toast]);

  useEffect(() => {
    layer.invalidate();
  }, [columns, rows, layer]);

  // ---- Derived stream data -------------------------------------------------
  const shots = storeState.shots;
  const filtered = useMemo(() => filterShots(shots, ui.streamFilter, ui.moviesOnly), [shots, ui.streamFilter, ui.moviesOnly]);
  const groups = useMemo(() => contiguousGroups(filtered), [filtered]);
  const items = useMemo(() => flattenStream(groups, ui.collapsed), [groups, ui.collapsed]);
  const counts = useMemo(() => streamCounts(shots, ui.moviesOnly), [shots, ui.moviesOnly]);
  const cursor = items.length === 0 ? 0 : nextNavigable(items, ui.collapsed, Math.min(ui.cursor, items.length - 1), 1);
  const cursorItem = items[cursor] ?? null;
  const cursorShot = cursorItem?.kind === "shot" ? cursorItem.shot : null;
  const selectedShot = useMemo(
    () => (ui.selectedShot ? shots.find((shot) => shot.path === ui.selectedShot) ?? null : null),
    [shots, ui.selectedShot],
  );
  const activeShot = ui.pane === "detail" || ui.takeover?.kind === "shot" ? (selectedShot ?? cursorShot) : (cursorShot ?? selectedShot);

  const frictionLogs = storeState.frictionLogs;
  const frictionFiltered = useMemo(() => filterFrictionLogs(frictionLogs, ui.frictionFilter), [frictionLogs, ui.frictionFilter]);
  const frictionCounts = useMemo(
    () => ({
      pending: frictionLogs.filter((log) => frictionState(log) !== "seen").length,
      seen: frictionLogs.filter((log) => frictionState(log) === "seen").length,
    }),
    [frictionLogs],
  );
  const frictionCursor = Math.min(ui.frictionCursor, Math.max(0, frictionFiltered.length - 1));
  const selectedLog: FrictionLog | null = useMemo(
    () => (ui.selectedLog ? frictionLogs.find((log) => log.id === ui.selectedLog) ?? null : null) ?? frictionFiltered[frictionCursor] ?? null,
    [frictionLogs, frictionFiltered, frictionCursor, ui.selectedLog],
  );
  const selectedRun: FrictionRun | null = useMemo(() => {
    if (!selectedLog) return null;
    return selectedLog.runs.find((run) => run.runId === ui.selectedRun) ?? latestRun(selectedLog);
  }, [selectedLog, ui.selectedRun]);

  // ---- Layout ----------------------------------------------------------------
  const split = columns >= SPLIT_BREAKPOINT;
  const bodyHeight = Math.max(6, rows - 4);
  const listWidth = split ? Math.max(52, Math.floor(columns * 0.42)) : columns;
  const paneWidth = split ? columns - listWidth - 1 : columns;
  const listHeight = bodyHeight;

  const scrollTop = useMemo(() => scrollWindow(items, cursor, ui.scrollTop, listHeight), [items, cursor, ui.scrollTop, listHeight]);
  useEffect(() => {
    if (scrollTop !== ui.scrollTop) patch({ scrollTop });
  }, [scrollTop, ui.scrollTop, patch]);
  const frictionTop = frictionScroll(frictionFiltered.length, frictionCursor, ui.frictionScrollTop, listHeight);
  useEffect(() => {
    if (frictionTop !== ui.frictionScrollTop) patch({ frictionScrollTop: frictionTop });
  }, [frictionTop, ui.frictionScrollTop, patch]);

  // Reset per-shot transient state when the active shot changes.
  const activePathRef = useRef<string | null>(null);
  useEffect(() => {
    const current = activeShot?.path ?? null;
    if (current !== activePathRef.current) {
      activePathRef.current = current;
      patch({ inlinePlayer: false, playback: initialPlayback(), composer: false, error: null });
    }
  }, [activeShot?.path, patch]);

  // ---- Actions ----------------------------------------------------------------
  const quit = useCallback(() => {
    onQuit?.();
    exit();
  }, [exit, onQuit]);

  const stepDetail = useCallback(
    (delta: number) => {
      const current = uiRef.current;
      const currentPath = current.selectedShot ?? cursorShot?.path ?? null;
      const index = shots.findIndex((shot) => shot.path === currentPath);
      if (index === -1) return;
      const next = index + delta;
      if (next < 0 || next >= shots.length) return;
      patch({ selectedShot: shots[next]!.path });
    },
    [shots, cursorShot, patch],
  );

  const stepTakeover = useCallback(
    (delta: number) => {
      if (!activeShot) return;
      const siblings = reviewSiblings(shots, activeShot);
      const index = siblings.findIndex((shot) => shot.path === activeShot.path);
      const next = index + delta;
      if (next < 0 || next >= siblings.length) return;
      patch({ selectedShot: siblings[next]!.path });
    },
    [activeShot, shots, patch],
  );

  const markSeen = useCallback(
    async (shot: Shot, comment?: string) => {
      if (reviewStateOf(shot) === "seen" && !comment?.trim()) {
        toast("Already seen");
        return;
      }
      patch({ busy: true, error: null });
      try {
        await store.markShotSeen(shot, comment);
        toast("Seen");
        patch((previous) => {
          // Only leave the surface the user was on for THIS shot; if they paged
          // on while the write was queued, stay put.
          const stillHere = previous.selectedShot === shot.path;
          return {
            busy: false,
            composer: stillHere ? false : previous.composer,
            takeover: stillHere && previous.takeover?.kind === "shot" ? null : previous.takeover,
            pane: stillHere && previous.pane === "detail" ? "stream" : previous.pane,
          };
        });
      } catch (error) {
        patch({ busy: false, error: error instanceof Error ? error.message : String(error) });
      }
    },
    [store, patch, toast],
  );

  const sendFeedback = useCallback(
    async (shot: Shot, body: string) => {
      if (!body.trim()) {
        patch({ composer: false });
        return;
      }
      patch({ busy: true, error: null });
      try {
        await store.addShotComment(shot, body);
        toast("Comment added");
        patch((previous) => ({ busy: false, composer: previous.selectedShot === shot.path ? false : previous.composer }));
      } catch (error) {
        patch({ busy: false, error: error instanceof Error ? error.message : String(error) });
      }
    },
    [store, patch, toast],
  );

  const seenAll = useCallback(
    async (pool: Shot[]) => {
      const targets = pool.filter((shot) => reviewStateOf(shot) !== "seen");
      if (targets.length === 0) return;
      patch({ bulkBusy: true });
      const result = await store.markManySeen(targets);
      patch({ bulkBusy: false });
      if (result.failed === 0) toast(result.ok === 1 ? "Marked 1 frame seen" : `Marked ${result.ok} frames seen`);
      else if (result.ok === 0) toast("Couldn’t mark frames as seen");
      else toast(`Marked ${result.ok} seen; ${result.failed} failed`);
    },
    [store, patch, toast],
  );

  const markLogSeen = useCallback(
    async (log: FrictionLog) => {
      const run = latestRun(log);
      if (!run) return;
      if (frictionState(log) === "seen") {
        toast("Already seen");
        return;
      }
      try {
        await store.markFrictionRunSeen(log, run);
        toast("Marked 1 log seen");
      } catch (error) {
        toast(error instanceof Error ? error.message : "Couldn’t mark logs as seen");
      }
    },
    [store, toast],
  );

  const seenAllLogs = useCallback(
    async (logs: FrictionLog[]) => {
      const targets = logs.filter((log) => frictionState(log) !== "seen" && latestRun(log));
      if (targets.length === 0) return;
      let ok = 0;
      for (const log of targets) {
        try {
          await store.markFrictionRunSeen(log, latestRun(log)!);
          ok += 1;
        } catch {
          // counted below
        }
      }
      toast(ok === 0 ? "Couldn’t mark logs as seen" : ok === 1 ? "Marked 1 log seen" : `Marked ${ok} logs seen`);
    },
    [store, toast],
  );

  const desktop = useCallback(
    async (task: Promise<void>, success: string, failure: string) => {
      try {
        await task;
        if (success) toast(success);
      } catch {
        toast(failure);
      }
    },
    [toast],
  );

  const togglePrompt = useCallback(() => {
    const log = selectedLog;
    if (!log?.promptPath) return;
    if (uiRef.current.promptOpen) {
      patch({ promptOpen: false });
      return;
    }
    let prompt = "";
    try {
      prompt = fs.readFileSync(log.promptPath, "utf8");
    } catch {
      prompt = "(prompt.md could not be read)";
    }
    patch({ promptOpen: true, prompt });
  }, [selectedLog, patch]);

  const updatePlayback = useCallback(
    (update: Partial<PlaybackState>) => patch((previous) => ({ playback: { ...previous.playback, ...update } })),
    [patch],
  );

  const seekBy = useCallback(
    (deltaMs: number) => {
      patch((previous) => {
        const duration = previous.playback.durationMs ?? activeShot?.durationMs ?? null;
        const target = Math.max(0, duration ? Math.min(duration, previous.playback.positionMs + deltaMs) : previous.playback.positionMs + deltaMs);
        return { playback: { ...previous.playback, positionMs: target, seekToken: previous.playback.seekToken + 1, ended: false } };
      });
    },
    [patch, activeShot],
  );

  const seekChapter = useCallback(
    (direction: -1 | 1) => {
      if (!activeShot) return;
      const marks = activeShot.chapters.map((chapter) => chapter.tMs).filter((value): value is number => typeof value === "number").sort((a, b) => a - b);
      if (marks.length === 0) return;
      patch((previous) => {
        const position = previous.playback.positionMs;
        const target = direction > 0 ? marks.find((mark) => mark > position + 200) : [...marks].reverse().find((mark) => mark < position - 200);
        const resolved = target ?? (direction > 0 ? marks.at(-1)! : 0);
        return { playback: { ...previous.playback, positionMs: resolved, seekToken: previous.playback.seekToken + 1, ended: false } };
      });
    },
    [activeShot, patch],
  );

  const canInlinePlay = capabilities.graphics === "kitty";
  const togglePlay = useCallback(() => {
    if (!activeShot?.videoPath) {
      toast(activeShot?.isMovie ? "Video missing on disk" : "Not a movie");
      return;
    }
    if (!canInlinePlay) {
      toast("In-tray playback needs Kitty graphics — press O to open the movie");
      return;
    }
    patch((previous) => ({
      inlinePlayer: true,
      playback: previous.playback.ended
        ? { ...previous.playback, playing: true, positionMs: 0, seekToken: previous.playback.seekToken + 1, ended: false, error: null }
        : { ...previous.playback, playing: !previous.playback.playing, error: null },
    }));
  }, [activeShot, canInlinePlay, patch, toast]);

  // ---- Keymap -------------------------------------------------------------------
  useInput(
    (input, key) => {
      const state = uiRef.current;
      if (state.help) {
        if (input === "?" || key.escape || input === "q") patch({ help: false });
        return;
      }
      if (input === "?") {
        patch({ help: true });
        return;
      }
      if (key.ctrl && input === "c") {
        quit();
        return;
      }

      // Full-screen takeovers.
      if (state.takeover?.kind === "shot" && activeShot) {
        if (key.escape || input === "q") {
          patch({ takeover: null, composer: false });
          return;
        }
        if (key.leftArrow || input === "h") return stepTakeover(-1);
        if (key.rightArrow || input === "l") return stepTakeover(1);
        if (input === "c") return patch({ composer: true, error: null });
        if (input === "s") return void markSeen(activeShot);
        if (input === " ") return togglePlay();
        if (input === ",") return seekBy(-SEEK_STEP_MS);
        if (input === ".") return seekBy(SEEK_STEP_MS);
        if (input === "[") return seekChapter(-1);
        if (input === "]") return seekChapter(1);
        if (input === "y") return void desktop(copyImageToClipboard(activeShot.path), "Copied image", "Couldn’t copy image");
        if (input === "o") return void desktop(revealInFileManager(activeShot.path), "", "Couldn’t reveal file");
        if (input === "O" && activeShot.videoPath) return void desktop(openWithDefaultApp(activeShot.videoPath), "", "Couldn’t open movie");
        return;
      }
      if (state.takeover?.kind === "step" && selectedRun) {
        if (key.escape || input === "q") return patch({ takeover: null });
        if (key.leftArrow || input === "h") return patch({ stepIndex: Math.max(0, state.stepIndex - 1), imageIndex: 0 });
        if (key.rightArrow || input === "l") return patch({ stepIndex: Math.min(selectedRun.steps.length - 1, state.stepIndex + 1), imageIndex: 0 });
        if (input === "[") return patch({ imageIndex: Math.max(0, state.imageIndex - 1) });
        if (input === "]") {
          const count = selectedRun.steps[state.stepIndex]?.screenshots.length ?? 0;
          return patch({ imageIndex: Math.min(Math.max(0, count - 1), state.imageIndex + 1) });
        }
        return;
      }

      // Global keys.
      if (input === "q") return quit();
      // Detail-pane seek keys outrank the global settings toggle while a movie is loaded.
      if (state.tab === "shots" && state.pane === "detail" && state.inlinePlayer && activeShot?.videoPath) {
        if (input === ",") return seekBy(-SEEK_STEP_MS);
        if (input === ".") return seekBy(SEEK_STEP_MS);
      }
      if (input === "1") return patch({ tab: "shots", pane: "stream", takeover: null });
      if (input === "2") return patch({ tab: "frictionLogs", pane: "stream", takeover: null });
      if (key.tab) return patch((previous) => ({ tab: previous.tab === "shots" ? "frictionLogs" : "shots", pane: "stream" }));
      if (input === ",") return patch((previous) => ({ pane: previous.pane === "settings" ? "stream" : "settings" }));
      if (input === "r") {
        toast("Scanning…");
        void store.rescan({ force: true });
        return;
      }

      if (state.pane === "settings") {
        if (key.escape || input === "h" || key.leftArrow) patch({ pane: "stream" });
        return;
      }

      if (state.tab === "shots") {
        if (state.pane === "detail" && activeShot) {
          if (key.escape || (input === "h" && !split)) return patch({ pane: "stream", composer: false });
          if (key.leftArrow) return stepDetail(1);
          if (key.rightArrow) return stepDetail(-1);
          if (key.return || input === "f") return patch({ takeover: { kind: "shot" } });
          if (input === "c") return patch({ composer: true, error: null });
          if (input === "s") return void markSeen(activeShot);
          if (input === "p") {
            if (!activeShot.videoPath) return toast(activeShot.isMovie ? "Video missing on disk" : "Not a movie");
            if (!canInlinePlay) return toast("In-tray playback needs Kitty graphics — press O to open the movie");
            return patch((previous) => ({
              inlinePlayer: !previous.inlinePlayer,
              playback: { ...previous.playback, playing: !previous.inlinePlayer, error: null },
            }));
          }
          if (input === " ") return togglePlay();
          if (input === ",") return seekBy(-SEEK_STEP_MS);
          if (input === ".") return seekBy(SEEK_STEP_MS);
          if (input === "[") return seekChapter(-1);
          if (input === "]") return seekChapter(1);
          if (input === "o") return void desktop(revealInFileManager(activeShot.path), "", "Couldn’t reveal file");
          if (input === "O" && activeShot.videoPath) return void desktop(openWithDefaultApp(activeShot.videoPath), "", "Couldn’t open movie");
          if (input === "y") return void desktop(copyImageToClipboard(activeShot.path), "Copied image", "Couldn’t copy image");
          return;
        }
        // Stream list.
        store.markOpened();
        const move = (delta: number) => {
          const target = Math.max(0, Math.min(items.length - 1, cursor + delta));
          return patch({ cursor: nextNavigable(items, state.collapsed, target, delta >= 0 ? 1 : -1) });
        };
        if (key.downArrow || input === "j") return move(1);
        if (key.upArrow || input === "k") return move(-1);
        if (key.pageDown) return move(Math.max(1, Math.floor(listHeight / 4)));
        if (key.pageUp) return move(-Math.max(1, Math.floor(listHeight / 4)));
        if (input === "g" || key.home) return patch({ cursor: nextNavigable(items, state.collapsed, 0, 1) });
        if (input === "G" || key.end) return patch({ cursor: nextNavigable(items, state.collapsed, Math.max(0, items.length - 1), -1) });
        if (input === "u") return patch((previous) => ({ streamFilter: previous.streamFilter === "unseen" ? "history" : "unseen", cursor: 0 }));
        if (input === "m") return patch((previous) => ({ moviesOnly: !previous.moviesOnly, cursor: 0 }));
        if (input === "z" && cursorItem) {
          const id = cursorItem.group.id;
          return patch((previous) => {
            const collapsed = new Set(previous.collapsed);
            if (collapsed.has(id)) collapsed.delete(id);
            else collapsed.add(id);
            return { collapsed };
          });
        }
        if (cursorItem?.kind === "header") {
          if (key.return || key.leftArrow || key.rightArrow || input === " ") {
            const id = cursorItem.group.id;
            return patch((previous) => {
              const collapsed = new Set(previous.collapsed);
              if (collapsed.has(id)) collapsed.delete(id);
              else collapsed.add(id);
              return { collapsed };
            });
          }
          if (input === "S") return void seenAll(cursorItem.group.shots);
          return;
        }
        if (input === "S") return void seenAll(filtered);
        if (input === "A" && cursorItem) return void seenAll(cursorItem.group.shots);
        if (!cursorShot) return;
        if (key.return) return patch(split ? { selectedShot: cursorShot.path, takeover: { kind: "shot" } } : { selectedShot: cursorShot.path, pane: "detail" });
        if (input === "f" || input === " ") return patch({ selectedShot: cursorShot.path, takeover: { kind: "shot" } });
        if (input === "s") return void markSeen(cursorShot);
        if (input === "c") return patch({ selectedShot: cursorShot.path, composer: true, error: null, ...(split ? {} : { pane: "detail" as Pane }) });
        if (input === "p") {
          if (!cursorShot.videoPath) return toast(cursorShot.isMovie ? "Video missing on disk" : "Not a movie");
          if (!canInlinePlay) {
            patch({ selectedShot: cursorShot.path, ...(split ? {} : { pane: "detail" as Pane }) });
            toast("In-tray playback needs Kitty graphics — press O to open the movie");
            return;
          }
          return patch(split ? { selectedShot: cursorShot.path, inlinePlayer: true, playback: { ...initialPlayback(), playing: true } } : { selectedShot: cursorShot.path, pane: "detail", inlinePlayer: true, playback: { ...initialPlayback(), playing: true } });
        }
        if (input === "o") return void desktop(revealInFileManager(cursorShot.path), "", "Couldn’t reveal file");
        if (input === "O" && cursorShot.videoPath) return void desktop(openWithDefaultApp(cursorShot.videoPath), "", "Couldn’t open movie");
        if (input === "y") return void desktop(copyImageToClipboard(cursorShot.path), "Copied image", "Couldn’t copy image");
        return;
      }

      // Friction Logs tab.
      if (state.pane === "frictionStep" && selectedLog && selectedRun) {
        if (key.escape || input === "h") return patch({ pane: "frictionLog" });
        if (key.leftArrow) return patch({ stepIndex: Math.max(0, state.stepIndex - 1), imageIndex: 0, stepCursor: Math.max(0, state.stepIndex - 1) });
        if (key.rightArrow) return patch({ stepIndex: Math.min(selectedRun.steps.length - 1, state.stepIndex + 1), imageIndex: 0, stepCursor: Math.min(selectedRun.steps.length - 1, state.stepIndex + 1) });
        if (input === "[") return patch({ imageIndex: Math.max(0, state.imageIndex - 1) });
        if (input === "]") {
          const count = selectedRun.steps[state.stepIndex]?.screenshots.length ?? 0;
          return patch({ imageIndex: Math.min(Math.max(0, count - 1), state.imageIndex + 1) });
        }
        if (key.return || input === "f" || input === " ") return patch({ takeover: { kind: "step" } });
        const shotPath = selectedRun.steps[state.stepIndex]?.screenshots[state.imageIndex];
        if (input === "o" && shotPath) return void desktop(revealInFileManager(shotPath), "", "Couldn’t reveal file");
        if (input === "y" && shotPath) return void desktop(copyImageToClipboard(shotPath), "Copied image", "Couldn’t copy image");
        return;
      }
      if (state.pane === "frictionLog" && selectedLog) {
        if (key.escape || input === "h") return patch({ pane: "stream", promptOpen: false });
        const steps = selectedRun?.steps ?? [];
        if (key.downArrow || input === "j") return patch({ stepCursor: Math.min(Math.max(0, steps.length - 1), state.stepCursor + 1) });
        if (key.upArrow || input === "k") return patch({ stepCursor: Math.max(0, state.stepCursor - 1) });
        if ((key.return || key.rightArrow) && steps.length > 0) return patch({ pane: "frictionStep", stepIndex: state.stepCursor, imageIndex: 0 });
        if (input === "p") return togglePrompt();
        if (input === "s") return void markLogSeen(selectedLog);
        if ((input === "[" || input === "]") && selectedLog.runs.length > 1 && selectedRun) {
          const index = selectedLog.runs.indexOf(selectedRun);
          const next = input === "]" ? Math.min(selectedLog.runs.length - 1, index + 1) : Math.max(0, index - 1);
          return patch({ selectedRun: selectedLog.runs[next]!.runId, stepCursor: 0 });
        }
        return;
      }
      // Friction list.
      if (key.downArrow || input === "j") return patch({ frictionCursor: Math.min(Math.max(0, frictionFiltered.length - 1), frictionCursor + 1) });
      if (key.upArrow || input === "k") return patch({ frictionCursor: Math.max(0, frictionCursor - 1) });
      if (input === "g" || key.home) return patch({ frictionCursor: 0 });
      if (input === "G" || key.end) return patch({ frictionCursor: Math.max(0, frictionFiltered.length - 1) });
      if (input === "u") return patch((previous) => ({ frictionFilter: previous.frictionFilter === "unseen" ? "history" : "unseen", frictionCursor: 0 }));
      if (input === "S") return void seenAllLogs(frictionFiltered);
      const log = frictionFiltered[frictionCursor];
      if (!log) return;
      if (key.return || key.rightArrow || input === "l") return patch({ selectedLog: log.id, selectedRun: latestRun(log)?.runId ?? null, pane: "frictionLog", stepCursor: 0, promptOpen: false });
      if (input === "s") return void markLogSeen(log);
      if (input === "o") return void desktop(revealInFileManager(log.directory), "", "Couldn’t reveal folder");
    },
    { isActive: !ui.composer },
  );

  // ---- Render ---------------------------------------------------------------------
  const status = storeState.roots.length === 0
    ? { text: "Choose watch folders", color: theme.amber }
    : storeState.scanning
      ? { text: storeState.phase === "full" ? "Scanning for captures (deep)" : "Scanning for captures", color: theme.amber }
      : shots.length === 0 && frictionLogs.length === 0
        ? { text: "Waiting for captures", color: theme.muted }
        : { text: "Live review stream", color: theme.green };

  const hints: KeyHint[] = ui.takeover?.kind === "shot"
    ? [{ key: "esc", label: "close" }, { key: "← →", label: "older · newer" }, { key: "c", label: "feedback" }, { key: "s", label: "seen" }, ...(activeShot?.isMovie ? [{ key: "space", label: "play" }, { key: ", .", label: "seek" }, { key: "[ ]", label: "chapter" }] : []), { key: "y", label: "copy" }, { key: "o", label: "reveal" }]
    : ui.takeover?.kind === "step"
      ? [{ key: "esc", label: "close" }, { key: "← →", label: "steps" }, { key: "[ ]", label: "images" }]
      : ui.pane === "settings"
        ? [{ key: "esc", label: "back" }, { key: "r", label: "rescan" }, { key: "?", label: "help" }, { key: "q", label: "quit" }]
        : ui.tab === "shots"
    ? ui.pane === "detail"
      ? [{ key: "esc", label: "back" }, { key: "← →", label: "older · newer" }, { key: "f", label: "full screen" }, { key: "s", label: "seen" }, { key: "c", label: "feedback" }, { key: "p", label: "play" }, { key: "?", label: "help" }]
      : [{ key: "↑↓", label: "move" }, { key: "⏎", label: split ? "review" : "detail" }, { key: "f", label: "full screen" }, { key: "s", label: "seen" }, { key: "c", label: "feedback" }, { key: "u", label: ui.streamFilter === "unseen" ? "history" : "unseen" }, { key: "m", label: "movies" }, { key: "?", label: "help" }, { key: "q", label: "quit" }]
    : ui.pane === "frictionStep"
      ? [{ key: "esc", label: "back" }, { key: "← →", label: "steps" }, { key: "[ ]", label: "images" }, { key: "f", label: "full screen" }]
      : ui.pane === "frictionLog"
        ? [{ key: "esc", label: "back" }, { key: "↑↓", label: "steps" }, { key: "⏎", label: "open step" }, { key: "[ ]", label: "runs" }, { key: "p", label: "prompt" }, { key: "s", label: "seen" }]
        : [{ key: "↑↓", label: "move" }, { key: "⏎", label: "open" }, { key: "s", label: "seen" }, { key: "u", label: ui.frictionFilter === "unseen" ? "history" : "unseen" }, { key: "?", label: "help" }, { key: "q", label: "quit" }];

  let body;
  if (ui.help) {
    body = <HelpOverlay width={columns} height={bodyHeight + 1} />;
  } else if (ui.takeover?.kind === "shot" && activeShot) {
    const siblings = reviewSiblings(shots, activeShot);
    body = (
      <ReviewTakeover
        shot={activeShot}
        position={{ index: siblings.findIndex((shot) => shot.path === activeShot.path) + 1, count: siblings.length }}
        width={columns}
        height={bodyHeight + 1}
        composer={ui.composer}
        onComposerSubmit={(text) => void sendFeedback(activeShot, text)}
        onComposerCancel={() => patch({ composer: false })}
        playback={ui.playback}
        onPlayback={updatePlayback}
        playing={ui.inlinePlayer}
        busy={ui.busy}
        error={ui.error}
      />
    );
  } else if (ui.takeover?.kind === "step" && selectedLog && selectedRun && selectedRun.steps.length > 0) {
    body = (
      <FrictionStepTakeover
        log={selectedLog}
        run={selectedRun}
        stepIndex={Math.min(ui.stepIndex, selectedRun.steps.length - 1)}
        imageIndex={ui.imageIndex}
        width={columns}
        height={bodyHeight + 1}
        mode="takeover"
      />
    );
  } else if (ui.pane === "settings") {
    body = <SettingsPane roots={storeState.roots} width={columns} height={bodyHeight + 1} />;
  } else if (ui.tab === "shots") {
    const detail = activeShot ? (
      <DetailPane
        shot={activeShot}
        position={{ index: shots.findIndex((shot) => shot.path === activeShot.path) + 1, count: shots.length }}
        width={paneWidth}
        height={bodyHeight}
        mode={split ? "pane" : "page"}
        composer={ui.composer}
        onComposerSubmit={(text) => void sendFeedback(activeShot, text)}
        onComposerCancel={() => patch({ composer: false })}
        inlinePlayer={ui.inlinePlayer}
        playback={ui.playback}
        onPlayback={updatePlayback}
        busy={ui.busy}
        error={ui.error}
      />
    ) : (
      <Box flexShrink={0} width={paneWidth} height={bodyHeight} alignItems="center" justifyContent="center">
        <Text color={theme.muted}>Select a frame to review it here</Text>
      </Box>
    );
    if (!split && ui.pane === "detail") {
      body = (
        <Box flexShrink={0} flexDirection="column" width={columns} height={bodyHeight + 1}>
          <Box flexShrink={0} height={1} />
          {detail}
        </Box>
      );
    } else {
      body = (
        <Box flexShrink={0} flexDirection="column" width={columns} height={bodyHeight + 1}>
          <Box flexShrink={0} flexDirection="row" width={columns} height={1}>
            <FilterBar filter={ui.streamFilter} moviesOnly={ui.moviesOnly} counts={counts} width={listWidth} bulkBusy={ui.bulkBusy} />
          </Box>
          <Box flexShrink={0} flexDirection="row" width={columns} height={bodyHeight}>
            <StreamList
              items={items}
              collapsed={ui.collapsed}
              cursor={cursor}
              scrollTop={scrollTop}
              width={listWidth}
              height={listHeight}
              filter={ui.streamFilter}
              moviesOnly={ui.moviesOnly}
              counts={counts}
              focused
              scanning={storeState.scanning}
              hasRoots={storeState.roots.length > 0}
              totalShots={shots.length}
              bulkBusy={ui.bulkBusy}
            />
            {split ? (
              <>
                <Box flexShrink={0} width={1} height={bodyHeight} flexDirection="column">
                  {Array.from({ length: bodyHeight }, (_, index) => (
                    <Text key={index} color={theme.faint}>
                      │
                    </Text>
                  ))}
                </Box>
                {detail}
              </>
            ) : null}
          </Box>
        </Box>
      );
    }
  } else if (ui.pane === "frictionStep" && selectedLog && selectedRun && selectedRun.steps.length > 0) {
    body = (
      <Box flexShrink={0} flexDirection="column" width={columns} height={bodyHeight + 1}>
        <Box flexShrink={0} height={1} />
        <FrictionStepDetail log={selectedLog} run={selectedRun} stepIndex={Math.min(ui.stepIndex, selectedRun.steps.length - 1)} imageIndex={ui.imageIndex} width={columns} height={bodyHeight} mode="page" />
      </Box>
    );
  } else if (ui.pane === "frictionLog" && selectedLog) {
    body = (
      <Box flexShrink={0} flexDirection="column" width={columns} height={bodyHeight + 1}>
        <Box flexShrink={0} height={1} />
        <FrictionLogDetail log={selectedLog} run={selectedRun} stepCursor={ui.stepCursor} promptOpen={ui.promptOpen} prompt={ui.prompt} width={columns} height={bodyHeight} />
      </Box>
    );
  } else {
    body = (
      <Box flexShrink={0} flexDirection="column" width={columns} height={bodyHeight + 1}>
        <FrictionFilterBar filter={ui.frictionFilter} counts={frictionCounts} width={columns} />
        <FrictionList
          logs={frictionFiltered}
          cursor={frictionCursor}
          scrollTop={frictionTop}
          width={columns}
          height={listHeight}
          filter={ui.frictionFilter}
          counts={frictionCounts}
          focused
          total={frictionLogs.length}
          now={now}
        />
      </Box>
    );
  }

  const shotsUnseen = counts.pending;
  const showTabs = !ui.takeover && !ui.help && ui.pane !== "settings";
  const rowsUsed = FRICTION_ROW_HEIGHT; // keep import used for row-based paging parity
  void rowsUsed;

  return (
    <Box flexShrink={0} flexDirection="column" width={columns} height={rows}>
      <Box flexShrink={0} height={1} width={columns} paddingX={1} justifyContent="space-between">
        <Text wrap="truncate">
          <Text color={theme.brand} bold>
            ● Astroshots
          </Text>
          <Text color={status.color}>  {status.text}</Text>
          {storeState.unreadCount > 0 ? <Text color={theme.amber}>  {storeState.unreadCount} new</Text> : null}
          {storeState.error ? <Text color={theme.red}>  {truncate(storeState.error, 40)}</Text> : null}
        </Text>
        <Text color={theme.muted}>
          {storeState.treeCount} trees · {storeState.roots.length} {storeState.roots.length === 1 ? "root" : "roots"}
          {storeState.watching ? "" : " · not watching"}
        </Text>
      </Box>
      <Box flexShrink={0} height={1} width={columns} paddingX={1}>
        {showTabs ? (
          <Text>
            <Text color={ui.tab === "shots" ? theme.text : theme.muted} bold={ui.tab === "shots"} inverse={ui.tab === "shots"}>
              {" 1 Shots "}
            </Text>
            {shotsUnseen > 0 ? <Text color={theme.amber} bold>{` ${shotsUnseen}`}</Text> : null}
            <Text>   </Text>
            <Text color={ui.tab === "frictionLogs" ? theme.text : theme.muted} bold={ui.tab === "frictionLogs"} inverse={ui.tab === "frictionLogs"}>
              {" 2 Friction Logs "}
            </Text>
            {frictionCounts.pending > 0 ? <Text color={theme.amber} bold>{` ${frictionCounts.pending}`}</Text> : null}
          </Text>
        ) : (
          <Text color={theme.muted}>{ui.help ? "Help" : ui.pane === "settings" ? "Settings" : ui.takeover?.kind === "shot" ? "Full-screen review" : "Friction step review"}</Text>
        )}
      </Box>
      {body}
      {ui.toast ? <Toast message={ui.toast.message} width={columns} /> : <HintBar hints={hints} width={columns} />}
    </Box>
  );
}

export function frictionLogDirectory(log: FrictionLog): string {
  return path.dirname(log.directory);
}
