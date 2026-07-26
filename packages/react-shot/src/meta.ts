import type { ReactShotFixture } from "./types.js";

/** Capture controls from a fixture, excluding its React tree. */
export type ShotMeta = Omit<ReactShotFixture, "component">;

const DIALOG_SELECTOR_RE = /role\s*=\s*["']?dialog["']?/i;

export function resolveShotMeta(
  nodeMeta: Partial<ShotMeta>,
  browserMeta: Partial<ShotMeta>,
  cli: { width?: number; height?: number } = {},
): Required<
  Pick<
    ShotMeta,
    | "width"
    | "height"
    | "selector"
    | "settleMs"
    | "fullPage"
    | "stripOverlay"
    | "omitBackground"
  >
> &
  ShotMeta {
  const merged: Partial<ShotMeta> = { ...nodeMeta, ...browserMeta };
  const selector = merged.selector ?? "[data-react-shot-root]";
  const looksLikeDialog = DIALOG_SELECTOR_RE.test(selector);

  return {
    ...merged,
    width: cli.width ?? merged.width ?? 1280,
    height: cli.height ?? merged.height ?? 800,
    selector,
    settleMs: merged.settleMs ?? 150,
    fullPage: merged.fullPage ?? false,
    stripOverlay: merged.stripOverlay ?? looksLikeDialog,
    omitBackground:
      merged.omitBackground ??
      Boolean(merged.stripOverlay ?? looksLikeDialog),
    waitFor: merged.waitFor,
    background: merged.background,
  };
}

export function isDialogSelector(selector: string): boolean {
  return DIALOG_SELECTOR_RE.test(selector);
}
