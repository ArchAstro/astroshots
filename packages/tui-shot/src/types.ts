import type { ReactElement } from "react";

export interface TuiShotFixture {
  /** Real Ink tree rendered into the terminal frame. */
  component: ReactElement;
  /** Visible strings that must exist before a PNG is accepted. */
  expectText?: string[];
  /** Terminal grid dimensions, not image pixels. */
  cols?: number;
  rows?: number;
  background?: string;
  foreground?: string;
  fontFamily?: string;
  fontSize?: number;
  lineHeight?: number;
  padding?: number;
  borderRadius?: number;
  /** PNG device scale factor. Defaults to 2. */
  scale?: number;
}

export interface TuiShotRequest {
  fixturePath: string;
  outPath: string;
  headed?: boolean;
  cols?: number;
  rows?: number;
  scale?: number;
}

export type PtyKey =
  | "enter"
  | "up"
  | "down"
  | "left"
  | "right"
  | "tab"
  | "escape"
  | "backspace"
  | "space"
  | "ctrl-c"
  | "ctrl-d";

export type PtyAction =
  | { waitFor: string; timeoutMs?: number }
  | { waitForExit: true; timeoutMs?: number }
  | { key: PtyKey }
  | { text: string }
  | { pauseMs: number };

export interface PtyShotFixture {
  version: 1;
  /** Executable launched directly, without an intermediary shell. */
  command: string;
  args?: string[];
  /** Working directory, relative to the fixture file by default. */
  cwd?: string;
  env?: Record<string, string>;
  cols?: number;
  rows?: number;
  timeoutMs?: number;
  settleMs?: number;
  /** Permit a child that exits nonzero before capture. Defaults to false. */
  allowNonZeroExit?: boolean;
  actions?: PtyAction[];
  expectText?: string[];
  background?: string;
  foreground?: string;
  fontFamily?: string;
  fontSize?: number;
  lineHeight?: number;
  padding?: number;
  borderRadius?: number;
  scale?: number;
}

export interface PtyShotRequest {
  fixturePath: string;
  outPath: string;
  headed?: boolean;
  cols?: number;
  rows?: number;
  scale?: number;
}

export interface BatchEntry {
  fixture: string;
  out: string;
}

export interface BatchManifest {
  shots: BatchEntry[];
}
