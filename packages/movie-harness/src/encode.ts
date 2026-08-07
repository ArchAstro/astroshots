import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

import { chromium } from "playwright";

import { ensureDir } from "./paths.js";
import type { EncodeFramesRequest, Size } from "./types.js";

function hasFfmpeg(): boolean {
  const result = spawnSync("ffmpeg", ["-version"], { encoding: "utf8" });
  return result.status === 0;
}

/**
 * Encode a PNG sequence to WebM/MP4.
 *
 * Prefer ffmpeg when present (fast, high quality). Otherwise replay frames in
 * headless Chromium with Playwright recordVideo — no native deps beyond the
 * Chromium we already ship for stills.
 */
export async function encodeFrames(
  request: EncodeFramesRequest,
): Promise<{ videoPath: string; durationMs: number }> {
  if (request.framePaths.length === 0) {
    throw new Error("encodeFrames requires at least one frame");
  }
  for (const frame of request.framePaths) {
    if (!fs.existsSync(frame)) {
      throw new Error(`frame not found: ${frame}`);
    }
  }

  const outPath = path.resolve(request.outPath);
  ensureDir(path.dirname(outPath));
  const ext = path.extname(outPath).toLowerCase();
  const durationMs =
    request.durationMs ??
    Math.max(1, Math.round((request.framePaths.length / request.fps) * 1000));

  if (hasFfmpeg()) {
    await encodeWithFfmpeg(request, outPath);
    return { videoPath: outPath, durationMs };
  }

  if (ext === ".mp4") {
    // Playwright emits WebM; without ffmpeg we cannot remux to MP4.
    const webmPath = outPath.replace(/\.mp4$/i, ".webm");
    await encodeWithPlaywright(request, webmPath);
    return { videoPath: webmPath, durationMs };
  }

  await encodeWithPlaywright(request, outPath);
  return { videoPath: outPath, durationMs };
}

async function encodeWithFfmpeg(
  request: EncodeFramesRequest,
  outPath: string,
): Promise<void> {
  const listDir = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-movie-ff-"));
  try {
    const listPath = path.join(listDir, "frames.txt");
    const frameDuration = 1 / request.fps;
    const lines: string[] = [];
    for (const frame of request.framePaths) {
      lines.push(`file '${frame.replaceAll("'", "'\\''")}'`);
      lines.push(`duration ${frameDuration.toFixed(6)}`);
    }
    // Last frame needs a trailing file entry for concat demuxer.
    const last = request.framePaths.at(-1)!;
    lines.push(`file '${last.replaceAll("'", "'\\''")}'`);
    fs.writeFileSync(listPath, `${lines.join("\n")}\n`);

    const ext = path.extname(outPath).toLowerCase();
    const args =
      ext === ".mp4"
        ? [
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            listPath,
            "-vf",
            `scale=${request.size.width}:${request.size.height}:flags=neighbor`,
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            outPath,
          ]
        : [
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            listPath,
            "-vf",
            `scale=${request.size.width}:${request.size.height}:flags=neighbor`,
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "2M",
            "-pix_fmt",
            "yuv420p",
            outPath,
          ];

    const result = spawnSync("ffmpeg", args, { encoding: "utf8" });
    if (result.status !== 0) {
      throw new Error(
        `ffmpeg failed (${result.status}): ${result.stderr?.slice(0, 800) ?? ""}`,
      );
    }
  } finally {
    fs.rmSync(listDir, { recursive: true, force: true });
  }
}

async function encodeWithPlaywright(
  request: EncodeFramesRequest,
  outPath: string,
): Promise<void> {
  const videoDir = fs.mkdtempSync(
    path.join(os.tmpdir(), "astroshot-movie-pw-"),
  );
  const size: Size = request.size;
  const frameMs = Math.max(1, Math.round(1000 / request.fps));

  let browser;
  try {
    browser = await chromium.launch({ headless: true });
    const context = await browser.newContext({
      viewport: size,
      deviceScaleFactor: 1,
      recordVideo: {
        dir: videoDir,
        size,
      },
    });
    const page = await context.newPage();

    for (const framePath of request.framePaths) {
      const bytes = fs.readFileSync(framePath);
      const b64 = bytes.toString("base64");
      const mime =
        path.extname(framePath).toLowerCase() === ".jpg" ||
        path.extname(framePath).toLowerCase() === ".jpeg"
          ? "image/jpeg"
          : "image/png";
      await page.setContent(
        `<!doctype html>
        <html>
          <head>
            <meta charset="utf-8" />
            <style>
              html, body {
                margin: 0;
                width: ${size.width}px;
                height: ${size.height}px;
                overflow: hidden;
                background: #000;
              }
              img {
                display: block;
                width: ${size.width}px;
                height: ${size.height}px;
                object-fit: fill;
                image-rendering: pixelated;
              }
            </style>
          </head>
          <body>
            <img src="data:${mime};base64,${b64}" width="${size.width}" height="${size.height}" />
          </body>
        </html>`,
        { waitUntil: "load" },
      );
      await page.waitForTimeout(frameMs);
    }

    // Hold the last frame a beat so the encoder emits it.
    await page.waitForTimeout(Math.max(frameMs, 100));
    await context.close();

    const videos = fs
      .readdirSync(videoDir)
      .filter((name) => name.endsWith(".webm"))
      .map((name) => path.join(videoDir, name));
    if (videos.length === 0) {
      throw new Error("Playwright did not produce a WebM video");
    }
    // Prefer the newest file if multiple.
    videos.sort(
      (a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs,
    );
    fs.copyFileSync(videos[0]!, outPath);
  } finally {
    await browser?.close().catch(() => undefined);
    fs.rmSync(videoDir, { recursive: true, force: true });
  }
}

/** Copy the last frame as the poster PNG. */
export function posterFromFrames(framePaths: string[], outPath: string): string {
  if (framePaths.length === 0) {
    throw new Error("posterFromFrames requires at least one frame");
  }
  const last = framePaths.at(-1)!;
  ensureDir(path.dirname(outPath));
  fs.copyFileSync(last, outPath);
  return outPath;
}
