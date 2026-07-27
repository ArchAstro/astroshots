import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { chromium, type Browser } from "playwright";
import { tsImport } from "tsx/esm/api";

import { renderInkFrame, type InkRuntime } from "./render-ink.js";
import { ansiFrameToHtml } from "./terminal-html.js";
import type { TuiShotFixture, TuiShotRequest } from "./types.js";

let sharedBrowser: Browser | null = null;
let sharedBrowserHeaded: boolean | null = null;
let shotQueue: Promise<void> = Promise.resolve();

async function browserFor(headed: boolean): Promise<Browser> {
  if (sharedBrowser?.isConnected() && sharedBrowserHeaded === headed) {
    return sharedBrowser;
  }
  if (sharedBrowser) await closeBrowserNow();
  try {
    sharedBrowser = await chromium.launch({ headless: !headed });
    sharedBrowserHeaded = headed;
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    if (detail.includes("Executable doesn't exist")) {
      throw new Error(
        "Chromium is not installed. Run `npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot install-browser` and retry.",
        { cause: error },
      );
    }
    throw error;
  }
  return sharedBrowser;
}

async function closeBrowserNow(): Promise<void> {
  if (!sharedBrowser) return;
  try {
    await sharedBrowser.close();
  } finally {
    sharedBrowser = null;
    sharedBrowserHeaded = null;
  }
}

export function closeSharedBrowser(): Promise<void> {
  const close = shotQueue.then(closeBrowserNow);
  shotQueue = close.then(
    () => undefined,
    () => undefined,
  );
  return close;
}

interface FixtureContext {
  fixture: TuiShotFixture;
  ink: InkRuntime;
  react: unknown;
}

async function loadFixture(fixturePath: string): Promise<FixtureContext> {
  const absolute = path.resolve(fixturePath);
  if (!fs.existsSync(absolute)) {
    throw new Error(`Fixture not found: ${absolute}`);
  }
  const fixtureRequire = createRequire(absolute);
  let inkEntry: string;
  let reactEntry: string;
  try {
    inkEntry = fixtureRequire.resolve("ink");
    reactEntry = fixtureRequire.resolve("react");
  } catch (error) {
    throw new Error(
      `Could not resolve Ink and React next to fixture ${absolute}. Install ink@7 and react@19 in the fixture project.`,
      { cause: error },
    );
  }
  const inkRequire = createRequire(inkEntry);
  const [module, inkModule, reactModule, chalkModule] = await Promise.all([
    tsImport(absolute, import.meta.url) as Promise<{
      default?: TuiShotFixture;
      fixture?: TuiShotFixture;
    }>,
    import(pathToFileURL(inkEntry).href) as Promise<{
      render: InkRuntime["render"];
    }>,
    import(pathToFileURL(reactEntry).href),
    import(pathToFileURL(inkRequire.resolve("chalk")).href) as Promise<{
      default: { level: number };
    }>,
  ]);
  const fixtureModule = module as {
    default?: TuiShotFixture;
    fixture?: TuiShotFixture;
  };
  const fixture = fixtureModule.default ?? fixtureModule.fixture;
  if (!fixture?.component) {
    throw new Error(
      `Fixture ${absolute} must default-export a TuiShotFixture with a component.`,
    );
  }
  return {
    fixture,
    ink: { render: inkModule.render, chalk: chalkModule.default },
    react: (reactModule as { default?: unknown }).default ?? reactModule,
  };
}

function validPositive(
  value: number,
  name: string,
  options: { integer?: boolean; maximum: number },
): number {
  if (
    !Number.isFinite(value) ||
    value <= 0 ||
    value > options.maximum ||
    (options.integer && !Number.isInteger(value))
  ) {
    throw new Error(
      `${name} must be a positive${options.integer ? " integer" : ""} no greater than ${options.maximum}`,
    );
  }
  return value;
}

async function takeIsolatedTuiShot(request: TuiShotRequest): Promise<string> {
  const context = await loadFixture(request.fixturePath);
  const previousReact = Object.getOwnPropertyDescriptor(globalThis, "React");
  Object.defineProperty(globalThis, "React", {
    configurable: true,
    value: context.react,
    writable: true,
  });

  try {
    return await renderTuiShot(request, context);
  } finally {
    if (previousReact) {
      Object.defineProperty(globalThis, "React", previousReact);
    } else {
      Reflect.deleteProperty(globalThis, "React");
    }
  }
}

async function renderTuiShot(
  request: TuiShotRequest,
  context: FixtureContext,
): Promise<string> {
  const { fixture } = context;
  const cols = validPositive(request.cols ?? fixture.cols ?? 100, "cols", {
    integer: true,
    maximum: 1_000,
  });
  const rows = validPositive(request.rows ?? fixture.rows ?? 30, "rows", {
    integer: true,
    maximum: 1_000,
  });
  const scale = validPositive(request.scale ?? fixture.scale ?? 2, "scale", {
    maximum: 4,
  });
  const background = fixture.background ?? "#090a12";
  const foreground = fixture.foreground ?? "#e8e8f2";
  const fontFamily =
    fixture.fontFamily ??
    '"SFMono-Regular", "Cascadia Code", "Roboto Mono", Menlo, Consolas, monospace';
  const fontSize = validPositive(fixture.fontSize ?? 15, "fontSize", {
    maximum: 200,
  });
  const lineHeight = validPositive(fixture.lineHeight ?? 1.32, "lineHeight", {
    maximum: 10,
  });
  const padding = fixture.padding ?? 22;
  const borderRadius = fixture.borderRadius ?? 12;
  if (!Number.isFinite(padding) || padding < 0 || padding > 1_000) {
    throw new Error("padding must be between 0 and 1000");
  }
  if (
    !Number.isFinite(borderRadius) ||
    borderRadius < 0 ||
    borderRadius > 1_000
  ) {
    throw new Error("borderRadius must be between 0 and 1000");
  }
  const ansi = renderInkFrame(fixture, cols, rows, context.ink);
  const plainFrame = ansi
    .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\x1b[@-_]/g, "");
  for (const expected of fixture.expectText ?? []) {
    if (!plainFrame.includes(expected)) {
      throw new Error(
        `Fixture did not render expected text ${JSON.stringify(expected)}. ` +
          `Visible frame:\n${plainFrame.slice(0, 1_200)}`,
      );
    }
  }
  const terminalRows = await ansiFrameToHtml(ansi, {
    cols,
    rows,
    foreground,
    background,
  });
  const cssWidth = Math.ceil(cols * fontSize * 0.62 + padding * 2);
  const cssHeight = Math.ceil(rows * fontSize * lineHeight + padding * 2);
  const outPath = path.resolve(request.outPath);
  if (path.extname(outPath).toLowerCase() !== ".png") {
    throw new Error(`Output must use a .png extension: ${outPath}`);
  }
  fs.mkdirSync(path.dirname(outPath), { recursive: true });

  const browser = await browserFor(Boolean(request.headed));
  let page;
  try {
    page = await browser.newPage({
      viewport: { width: cssWidth + 32, height: cssHeight + 32 },
      deviceScaleFactor: scale,
    });
  } catch (error) {
    await closeBrowserNow();
    throw error;
  }
  try {
    await page.setContent(
      `<!doctype html>
      <html>
        <head>
          <meta charset="utf-8" />
          <style>
            html, body { margin: 0; padding: 0; background: transparent; }
            body { display: inline-block; padding: 16px; }
            [data-tui-shot] {
              box-sizing: border-box;
              width: ${cssWidth}px;
              height: ${cssHeight}px;
              overflow: hidden;
              padding: ${padding}px;
              border: 1px solid rgba(185, 168, 255, .18);
              border-radius: ${borderRadius}px;
              background: ${background};
              color: ${foreground};
              box-shadow: 0 18px 44px rgba(0, 0, 0, .28);
              font-family: ${fontFamily};
              font-size: ${fontSize}px;
              font-variant-ligatures: none;
              font-weight: 400;
              line-height: ${lineHeight};
              text-rendering: geometricPrecision;
            }
            .tui-row {
              height: ${lineHeight}em;
              overflow: hidden;
              white-space: pre;
            }
          </style>
        </head>
        <body>
          <div data-tui-shot role="img" aria-label="Terminal screenshot">${terminalRows}</div>
        </body>
      </html>`,
      { waitUntil: "load" },
    );
    await page.locator("[data-tui-shot]").screenshot({
      path: outPath,
      omitBackground: true,
    });
  } finally {
    await page.close();
  }
  return outPath;
}

export function takeTuiShot(request: TuiShotRequest): Promise<string> {
  const shot = shotQueue.then(() => takeIsolatedTuiShot(request));
  shotQueue = shot.then(
    () => undefined,
    () => undefined,
  );
  return shot;
}
