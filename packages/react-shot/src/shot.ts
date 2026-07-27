import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { chromium, type Browser, type Page } from "playwright";
import { findConfigPath, loadConfig, resolvePackageRoot } from "./config.js";
import { startShotServer } from "./create-server.js";
import { resolveShotMeta, type ShotMeta } from "./meta.js";
import type { ReactShotFixture, ShotRequest } from "./types.js";

let sharedBrowser: Browser | null = null;
let sharedBrowserHeaded: boolean | null = null;
let shotQueue: Promise<void> = Promise.resolve();

async function getBrowser(headed: boolean): Promise<Browser> {
  if (sharedBrowser?.isConnected() && sharedBrowserHeaded === headed) {
    return sharedBrowser;
  }
  if (sharedBrowser) await closeBrowserNow();
  try {
    sharedBrowser = await chromium.launch({ headless: !headed });
    sharedBrowserHeaded = headed;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(
      `Could not launch Chromium. Install it with "npx --@archastro:registry=https://registry.npmjs.org @archastro/astroshot install-browser".\n${message}`,
    );
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

async function readFixtureMeta(
  fixturePath: string,
): Promise<Partial<ReactShotFixture>> {
  try {
    const url = `${pathToFileURL(path.resolve(fixturePath)).href}?reactShot=${Date.now()}`;
    const module = await import(url);
    const fixture = (module.default ?? module.fixture) as ReactShotFixture;
    if (!fixture || typeof fixture !== "object") return {};
    const {
      width,
      height,
      selector,
      waitFor,
      settleMs,
      background,
      fullPage,
      stripOverlay,
      omitBackground,
    } = fixture;
    return {
      width,
      height,
      selector,
      waitFor,
      settleMs,
      background,
      fullPage,
      stripOverlay,
      omitBackground,
    };
  } catch {
    // TSX and application imports often cannot execute in Node. The browser
    // reports the same metadata after Vite transforms the fixture.
    return {};
  }
}

async function takeIsolatedShot(request: ShotRequest): Promise<string> {
  const fixturePath = path.resolve(request.fixturePath);
  if (!fs.existsSync(fixturePath)) {
    throw new Error(`Fixture not found: ${fixturePath}`);
  }
  if (path.extname(request.outPath).toLowerCase() !== ".png") {
    throw new Error(`Output must use a .png extension: ${request.outPath}`);
  }
  for (const [name, value] of [
    ["width", request.width],
    ["height", request.height],
  ] as const) {
    if (
      value !== undefined &&
      (!Number.isInteger(value) || value < 1 || value > 10_000)
    ) {
      throw new Error(`${name} must be an integer between 1 and 10000`);
    }
  }

  const configPath =
    request.configPath ?? findConfigPath(path.dirname(fixturePath));
  const config = await loadConfig(configPath);
  const packageRoot = resolvePackageRoot(fixturePath, request.root, config);
  const nodeMeta = await readFixtureMeta(fixturePath);
  let width = request.width ?? nodeMeta.width ?? 1280;
  let height = request.height ?? nodeMeta.height ?? 800;
  const outPath = path.resolve(request.outPath);
  fs.mkdirSync(path.dirname(outPath), { recursive: true });

  const { server, url } = await startShotServer({
    fixturePath,
    packageRoot,
    config,
  });
  let page: Page | undefined;

  try {
    const browser = await getBrowser(Boolean(request.headed));
    page = await browser.newPage({
      viewport: { width, height },
      deviceScaleFactor: 1,
    });
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") pageErrors.push(message.text());
    });

    await page.goto(url, { waitUntil: "domcontentloaded", timeout: 60_000 });
    try {
      await page.waitForFunction(
        () =>
          (window as unknown as { __REACT_SHOT_READY__?: boolean })
            .__REACT_SHOT_READY__ === true,
        undefined,
        { timeout: 45_000 },
      );
    } catch (error) {
      const bootError = await page
        .evaluate(
          () =>
            (window as unknown as { __REACT_SHOT_ERROR__?: string })
              .__REACT_SHOT_ERROR__ ?? null,
        )
        .catch(() => null);
      const body = await page.locator("body").innerText().catch(() => "");
      throw new Error(
        [
          `Fixture page never became ready: ${error instanceof Error ? error.message : error}`,
          bootError ? `Boot error: ${bootError}` : null,
          pageErrors.length
            ? `Browser errors:\n${[...new Set(pageErrors)].slice(0, 10).map((item) => `  - ${item}`).join("\n")}`
            : null,
          body ? `Rendered body:\n${body.slice(0, 800)}` : null,
        ]
          .filter(Boolean)
          .join("\n"),
      );
    }

    const bootError = await page.evaluate(
      () =>
        (window as unknown as { __REACT_SHOT_ERROR__?: string })
          .__REACT_SHOT_ERROR__ ?? null,
    );
    if (bootError) throw new Error(`Fixture threw while mounting:\n${bootError}`);

    const browserMeta = (await page.evaluate(
      () =>
        (window as unknown as { __REACT_SHOT_META__?: ShotMeta })
          .__REACT_SHOT_META__ ?? {},
    )) as ShotMeta;
    const meta = resolveShotMeta(nodeMeta, browserMeta, {
      width: request.width,
      height: request.height,
    });
    width = meta.width;
    height = meta.height;
    if (
      !Number.isInteger(width) ||
      !Number.isInteger(height) ||
      width < 1 ||
      height < 1 ||
      width > 10_000 ||
      height > 10_000
    ) {
      throw new Error(
        `Fixture dimensions must be integers between 1 and 10000; received ${width}x${height}`,
      );
    }

    const viewport = page.viewportSize();
    if (viewport && (viewport.width !== width || viewport.height !== height)) {
      await page.setViewportSize({ width, height });
    }

    if (meta.waitFor) {
      if (meta.waitFor.startsWith("text=") || meta.waitFor.startsWith("text/")) {
        const text = meta.waitFor.replace(/^text[=/]/, "");
        await page.getByText(text, { exact: false }).first().waitFor({
          state: "visible",
          timeout: 15_000,
        });
      } else {
        await page.waitForSelector(meta.waitFor, {
          state: "visible",
          timeout: 15_000,
        });
      }
    } else {
      await page.waitForSelector(meta.selector, {
        state: "visible",
        timeout: 15_000,
      });
    }

    if (meta.settleMs > 0) await page.waitForTimeout(meta.settleMs);

    const seriousErrors = pageErrors.filter(
      (error) =>
        !/Download the React DevTools/i.test(error) &&
        !/React does not recognize/i.test(error) &&
        !/\[vite\]/i.test(error),
    );
    if (seriousErrors.length > 0) {
      throw new Error(
        `Fixture rendered with browser errors:\n${[...new Set(seriousErrors)]
          .slice(0, 8)
          .map((error) => `  - ${error}`)
          .join("\n")}`,
      );
    }

    if (meta.stripOverlay) {
      if ((await page.locator(meta.selector).count()) === 0) {
        throw new Error(
          `stripOverlay is enabled but ${JSON.stringify(meta.selector)} matched nothing`,
        );
      }
      await page.evaluate((selector) => {
        const target = document.querySelector(selector);
        if (!(target instanceof HTMLElement)) return;
        const clone = target.cloneNode(true) as HTMLElement;
        document.documentElement.style.cssText =
          "background:transparent !important;";
        document.body.replaceChildren();
        document.body.style.cssText =
          "margin:0;padding:0;background:transparent !important;display:block;width:fit-content;height:fit-content;";
        for (const [property, value] of [
          ["position", "relative"],
          ["inset", "auto"],
          ["transform", "none"],
          ["margin", "0"],
          ["max-height", "none"],
          ["left", "auto"],
          ["top", "auto"],
          ["box-shadow", "none"],
          ["filter", "none"],
        ]) {
          clone.style.setProperty(property, value, "important");
        }
        clone.setAttribute("data-react-shot-isolated", "true");
        document.body.appendChild(clone);
      }, meta.selector);
      await page.waitForTimeout(80);
    }

    if (meta.fullPage) {
      await page.screenshot({
        path: outPath,
        fullPage: true,
        omitBackground: meta.omitBackground,
      });
      return outPath;
    }

    const target = page
      .locator(
        meta.stripOverlay ? "[data-react-shot-isolated]" : meta.selector,
      )
      .first();
    await target.waitFor({ state: "visible", timeout: 10_000 });
    const box = await target.boundingBox();
    if (!box || box.width < 2 || box.height < 2) {
      throw new Error(
        `Screenshot target has an empty bounding box: ${meta.selector}`,
      );
    }
    if (
      meta.stripOverlay &&
      box.width >= width * 0.9 &&
      box.height >= height * 0.9
    ) {
      throw new Error(
        `Overlay removal produced a viewport-sized target (${Math.round(box.width)}x${Math.round(box.height)}); check selector ${JSON.stringify(meta.selector)}`,
      );
    }

    await target.screenshot({
      path: outPath,
      omitBackground: meta.omitBackground,
    });
    return outPath;
  } finally {
    await page?.close();
    await server.close();
  }
}

export function takeShot(request: ShotRequest): Promise<string> {
  const shot = shotQueue.then(() => takeIsolatedShot(request));
  shotQueue = shot.then(
    () => undefined,
    () => undefined,
  );
  return shot;
}
