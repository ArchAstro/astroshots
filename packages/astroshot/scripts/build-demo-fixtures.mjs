#!/usr/bin/env node
/**
 * Development-only generator for the bundled `astroshot demo` fixtures.
 *
 * `astroshot demo` must run with zero prerequisites (no Chromium download, no
 * ffmpeg, no user assets), so the demo payload ships as committed bytes under
 * fixtures/demo/. This script regenerates those bytes and requires the full
 * capture toolchain — it is never invoked by the CLI.
 *
 * Usage:
 *   npm run build:demo-fixtures --workspace @archastro/astroshot
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { encodeFrames } from "@archastro/movie-harness";
import { chromium } from "playwright";

const packageRoot = path.dirname(
  path.dirname(fileURLToPath(import.meta.url)),
);
const fixturesDir = path.join(packageRoot, "fixtures", "demo");
const POSTER_FRAME = 18;
const FRAME_COUNT = 30;
const FPS = 10;
const WIDTH = 720;
const HEIGHT = 450;

function shell() {
  return `
    * { box-sizing: border-box; margin: 0; }
    body {
      width: ${WIDTH}px;
      height: ${HEIGHT}px;
      display: flex;
      align-items: center;
      justify-content: center;
      background: radial-gradient(120% 120% at 12% 0%, #1b1f3b 0%, #0a0c18 62%);
      color: #f8fafc;
      font-family: -apple-system, "SF Pro Text", "Segoe UI", system-ui, sans-serif;
    }
    .card {
      width: 620px;
      padding: 32px 36px;
      border-radius: 20px;
      background: rgba(15, 19, 38, 0.86);
      border: 1px solid rgba(124, 92, 255, 0.45);
      box-shadow: 0 26px 60px rgba(2, 4, 12, 0.6);
    }
    .kicker {
      font-size: 12px;
      letter-spacing: 0.16em;
      text-transform: uppercase;
      color: #a78bfa;
      font-weight: 700;
    }
    h1 { font-size: 30px; margin: 10px 0 12px; letter-spacing: -0.02em; }
    p { font-size: 16px; line-height: 1.5; color: #c7cbe0; }
    code {
      font-family: "SF Mono", ui-monospace, Menlo, monospace;
      font-size: 14px;
      color: #f8fafc;
      background: rgba(124, 92, 255, 0.18);
      border-radius: 6px;
      padding: 2px 6px;
    }
    ul { margin: 18px 0 0; padding: 0; list-style: none; display: grid; gap: 10px; }
    li { display: flex; gap: 10px; align-items: center; font-size: 15px; color: #dfe2f2; }
    .dot {
      width: 10px; height: 10px; border-radius: 50%;
      background: #34d399; flex: none;
    }
    .dot.pending { background: #64748b; }
    .dot.movie { background: #f59e0b; }
    .bar {
      margin-top: 24px; height: 8px; border-radius: 999px;
      background: rgba(148, 163, 184, 0.25); overflow: hidden;
    }
    .bar > span { display: block; height: 100%; background: linear-gradient(90deg, #7c5cff, #22d3ee); }
    .step { margin-top: 14px; font-size: 14px; color: #94a3b8; }
  `;
}

function stillWelcome() {
  return `<!doctype html><meta charset="utf-8"><style>${shell()}</style>
  <div class="card">
    <div class="kicker">Astroshots demo</div>
    <h1>Your watch path works</h1>
    <p>These files were written by <code>astroshot demo</code>. If you can see
    this frame in the Astroshots tray, the app, your watched folder, and the
    on-disk contract are connected.</p>
    <ul>
      <li><span class="dot"></span>Still capture — this frame</li>
      <li><span class="dot movie"></span>Journey movie — poster + video</li>
      <li><span class="dot"></span>manifest.json — titles and metadata</li>
    </ul>
  </div>`;
}

function stillNextSteps() {
  return `<!doctype html><meta charset="utf-8"><style>${shell()}</style>
  <div class="card">
    <div class="kicker">Next steps</div>
    <h1>Capture something real</h1>
    <ul>
      <li><span class="dot"></span><code>astroshot doctor</code> — check setup</li>
      <li><span class="dot"></span><code>astroshot init react</code> — fixture</li>
      <li><span class="dot pending"></span><code>astroshot install-browser</code> — Chromium for react/ink/pty</li>
      <li><span class="dot movie"></span><code>astroshot movie which-source</code> — pick a movie source</li>
    </ul>
    <p class="step">Send feedback or mark this frame Seen in Astroshots; the app
    writes review.json next to these files.</p>
  </div>`;
}

function movieFrame(index) {
  const progress = Math.round(((index + 1) / FRAME_COUNT) * 100);
  const steps = [
    "Recording a journey…",
    "Poster frame captured",
    "Encoding WebM…",
    "Streaming into Astroshots",
  ];
  const step = steps[Math.min(steps.length - 1, Math.floor((index / FRAME_COUNT) * steps.length))];
  return `<!doctype html><meta charset="utf-8"><style>${shell()}</style>
  <div class="card">
    <div class="kicker">Astroshots demo · movie</div>
    <h1>Journey movie</h1>
    <p>A movie is a poster PNG plus a sibling video. Astroshots shows the badge,
    duration, and chapters, and plays it in the tray.</p>
    <div class="bar"><span style="width:${progress}%"></span></div>
    <div class="step">${step} — ${progress}%</div>
  </div>`;
}

/**
 * Read the encoded duration back from the file.
 *
 * `encodeFrames` returns the *intended* duration (frames / fps). The Playwright
 * recordVideo fallback drifts from that, and `manifest.duration_ms` is what the
 * Astroshots tray displays, so the shipped value must be measured.
 */
async function measureVideoDurationMs(page, videoPath) {
  const base64 = fs.readFileSync(videoPath).toString("base64");
  await page.setContent(
    `<!doctype html><meta charset="utf-8"><video id="probe" src="data:video/webm;base64,${base64}"></video>`,
  );
  return page.evaluate(async () => {
    const video = document.getElementById("probe");
    if (Number.isFinite(video.duration) && video.duration > 0) {
      return Math.round(video.duration * 1000);
    }
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("metadata timeout")), 10000);
      video.addEventListener("loadedmetadata", () => {
        clearTimeout(timer);
        resolve();
      });
      video.addEventListener("error", () => {
        clearTimeout(timer);
        reject(new Error("video failed to load"));
      });
    });
    return Math.round(video.duration * 1000);
  });
}

async function main() {
  fs.rmSync(fixturesDir, { recursive: true, force: true });
  fs.mkdirSync(fixturesDir, { recursive: true });
  const framesDir = fs.mkdtempSync(path.join(packageRoot, ".demo-frames-"));

  const browser = await chromium.launch();
  try {
    const page = await browser.newPage({
      viewport: { width: WIDTH, height: HEIGHT },
      deviceScaleFactor: 2,
    });

    await page.setContent(stillWelcome());
    await page.screenshot({ path: path.join(fixturesDir, "welcome.png") });

    await page.setContent(stillNextSteps());
    await page.screenshot({ path: path.join(fixturesDir, "next-steps.png") });

    const framePaths = [];
    for (let index = 0; index < FRAME_COUNT; index += 1) {
      await page.setContent(movieFrame(index));
      const frame = path.join(
        framesDir,
        `${String(index + 1).padStart(4, "0")}.png`,
      );
      await page.screenshot({ path: frame });
      framePaths.push(frame);
      if (index === POSTER_FRAME) {
        fs.copyFileSync(frame, path.join(fixturesDir, "journey.png"));
      }
    }

    const videoPath = path.join(fixturesDir, "journey.webm");
    await encodeFrames({
      framePaths,
      outPath: videoPath,
      fps: FPS,
      size: { width: WIDTH, height: HEIGHT },
    });
    const durationMs = await measureVideoDurationMs(page, videoPath);
    if (!(durationMs > 0)) {
      throw new Error(`could not measure the encoded duration of ${videoPath}`);
    }
    fs.writeFileSync(
      path.join(fixturesDir, "fixtures.json"),
      `${JSON.stringify(
        {
          version: 1,
          generated_by: "npm run build:demo-fixtures --workspace @archastro/astroshot",
          viewport: `${WIDTH}x${HEIGHT}`,
          shots: [
            {
              asset: "welcome.png",
              slug: "welcome",
              title: "Watch path works",
              description:
                "astroshot demo wrote this still into .astroshot/ with zero prerequisites.",
            },
            {
              asset: "next-steps.png",
              slug: "next-steps",
              title: "Next steps",
              description:
                "The commands that capture real React, Ink, PTY, and movie states.",
            },
            {
              asset: "journey.png",
              video: "journey.webm",
              slug: "journey",
              title: "Journey movie",
              description:
                "A poster PNG plus sibling WebM proves movie playback, duration, and chapters.",
              duration_ms: durationMs,
              source: "frames",
              // Chapters are fractions of the measured duration so every
              // t_ms stays inside the video the tray actually plays.
              chapters: [
                { slug: "recording", t_ms: 0 },
                {
                  slug: "poster",
                  t_ms: Math.round((durationMs * (POSTER_FRAME + 1)) / FRAME_COUNT),
                },
                { slug: "streaming", t_ms: Math.round((durationMs * 3) / 4) },
              ],
            },
          ],
        },
        null,
        2,
      )}\n`,
    );
  } finally {
    await browser.close();
    fs.rmSync(framesDir, { recursive: true, force: true });
  }

  for (const entry of fs.readdirSync(fixturesDir).sort()) {
    const stat = fs.statSync(path.join(fixturesDir, entry));
    console.log(`${entry.padEnd(18)} ${String(stat.size).padStart(8)} bytes`);
  }
}

await main();
