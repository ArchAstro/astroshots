import type { ReactNode } from "react";

/**
 * Default export shape for a react-shot fixture module.
 *
 * @example
 * ```tsx
 * export default {
 *   width: 720,
 *   height: 900,
 *   background: "#faf9f6",
 *   component: <MyModal />,
 * } satisfies ReactShotFixture;
 * ```
 */
export interface ReactShotFixture {
  /** React tree to mount. */
  component: ReactNode;
  /** Viewport width in CSS pixels. Defaults to 1280. */
  width?: number;
  /** Viewport height in CSS pixels. Defaults to 800. */
  height?: number;
  /** Page or canvas background. Defaults to transparent. */
  background?: string;
  /** CSS selector to capture. Defaults to `[data-react-shot-root]`. */
  selector?: string;
  /** Optional selector or `text=...` value that must become visible. */
  waitFor?: string;
  /** Additional layout settling time in milliseconds. Defaults to 150. */
  settleMs?: number;
  /** Capture the full page instead of the selected element. */
  fullPage?: boolean;
  /**
   * Remove a full-screen overlay around the target before capture.
   * Defaults to true when `selector` targets a dialog.
   */
  stripOverlay?: boolean;
  /**
   * Preserve transparency in the PNG. Defaults to true for isolated dialogs.
   */
  omitBackground?: boolean;
}

/** Package-level config exported from `react-shot.config.*`. */
export interface ReactShotConfig {
  /** Package root used for module resolution. Relative paths are config-relative. */
  root?: string;
  /** Vite aliases, such as `@` to an application's source directory. */
  alias?: Record<string, string>;
  /** CSS files imported into every fixture host page. */
  styles?: string[];
  /** Path to a PostCSS config file. */
  postcssConfig?: string;
  /** Additional dependencies Vite should deduplicate. */
  dedupe?: string[];
  /** Module names that should resolve to an empty browser-safe stub. */
  stubModules?: string[];
}

export interface ShotRequest {
  fixturePath: string;
  outPath: string;
  root?: string;
  configPath?: string;
  headed?: boolean;
  width?: number;
  height?: number;
}

export interface BatchEntry {
  fixture: string;
  out: string;
  root?: string;
  config?: string;
  width?: number;
  height?: number;
}

export interface BatchManifest {
  root?: string;
  config?: string;
  shots: BatchEntry[];
}
