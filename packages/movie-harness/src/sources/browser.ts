import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { chromium, type Browser, type Page } from "playwright";

import { MovieSession } from "../session.js";
import type { BrowserMovieOptions, MovieArtifact, MovieSessionOptions } from "../types.js";

export type BrowserMovieSessionOptions = Omit<MovieSessionOptions, "source"> &
  BrowserMovieOptions;

/**
 * Record a headless (or headed) browser journey with Playwright recordVideo.
 * Optional `scriptPath` should export `default async function(page)`.
 */
export async function recordBrowserMovie(
  options: BrowserMovieSessionOptions,
): Promise<MovieArtifact> {
  const size = options.size ?? { width: 1280, height: 720 };
  const session = MovieSession.create({
    ...options,
    size,
    source: "browser",
  });

  const videoDir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-bw-"));
  let browser: Browser | undefined;
  try {
    browser = await chromium.launch({ headless: !options.headed });
    const context = await browser.newContext({
      viewport: size,
      deviceScaleFactor: 1,
      recordVideo: {
        dir: videoDir,
        size,
      },
    });
    const page = await context.newPage();

    if (options.url) {
      await page.goto(options.url, {
        waitUntil: "domcontentloaded",
        timeout: 60_000,
      });
    }

    if (options.scriptPath) {
      await runScript(options.scriptPath, page);
    } else if (!options.url) {
      // Default demo surface so a bare call still produces a movie.
      await page.setContent(
        `<!doctype html>
        <html><body style="margin:0;display:grid;place-items:center;height:100vh;background:#090a12;color:#b9a8ff;font:28px ui-monospace,monospace">
          <div>astroshot-movie browser</div>
        </body></html>`,
        { waitUntil: "load" },
      );
      await page.waitForTimeout(400);
    }

    if (options.settleMs && options.settleMs > 0) {
      await page.waitForTimeout(options.settleMs);
    }

    const posterPath = path.join(videoDir, "poster.png");
    await page.screenshot({ path: posterPath, type: "png" });
    // Keep a frame so stop() has a poster fallback even if video path is set.
    session.pushFrame(fs.readFileSync(posterPath));

    await context.close();

    const videos = fs
      .readdirSync(videoDir)
      .filter((name) => name.endsWith(".webm"))
      .map((name) => path.join(videoDir, name));
    if (videos.length === 0) {
      throw new Error("Playwright did not produce a browser video");
    }
    videos.sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);

    return await session.stop({
      videoPath: videos[0],
      posterPath,
      durationMs: session.elapsedMs(),
      status: options.status ?? "running",
    });
  } finally {
    await browser?.close().catch(() => undefined);
    fs.rmSync(videoDir, { recursive: true, force: true });
  }
}

async function runScript(scriptPath: string, page: Page): Promise<void> {
  const absolute = path.resolve(scriptPath);
  if (!fs.existsSync(absolute)) {
    throw new Error(`browser script not found: ${absolute}`);
  }
  const mod = (await import(pathToFileURL(absolute).href)) as {
    default?: (page: Page) => Promise<void> | void;
    run?: (page: Page) => Promise<void> | void;
  };
  const runner = mod.default ?? mod.run;
  if (typeof runner !== "function") {
    throw new Error(
      `browser script must export default or run async function(page): ${absolute}`,
    );
  }
  await runner(page);
}
