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

export interface BatchEntry {
  fixture: string;
  out: string;
}

export interface BatchManifest {
  shots: BatchEntry[];
}
