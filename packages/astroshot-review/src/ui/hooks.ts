import { useEffect, useState, useSyncExternalStore } from "react";
import { useStdout } from "ink";

import type { StoreState } from "../data/store.js";
import { useServices } from "./context.js";

export function useStoreState(): StoreState {
  const { store } = useServices();
  return useSyncExternalStore(
    (listener) => store.subscribe(listener),
    () => store.getState(),
    () => store.getState(),
  );
}

export interface TerminalSize {
  columns: number;
  rows: number;
}

export function useTerminalSize(): TerminalSize {
  const { stdout } = useStdout();
  const [size, setSize] = useState<TerminalSize>({
    columns: stdout.columns || 80,
    rows: stdout.rows || 24,
  });
  useEffect(() => {
    const onResize = () => setSize({ columns: stdout.columns || 80, rows: stdout.rows || 24 });
    stdout.on("resize", onResize);
    return () => {
      stdout.off("resize", onResize);
    };
  }, [stdout]);
  return size;
}

/** Re-render every `intervalMs` so relative times stay fresh. */
export function useClock(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    timer.unref();
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}
