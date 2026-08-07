import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { encodeSolidPng } from "./png.js";
import { MovieSession } from "./session.js";
import {
  loadFrameSession,
  markFrameSession,
  pushFrameToSession,
  startFrameSession,
  stopFrameSession,
} from "./sources/frames-store.js";
import { recordTruecolorDemoMovie } from "./sources/pty.js";
import { terminalToHtml, createHeadlessTerminal, writeTerminal } from "./terminal-paint.js";

const temps: string[] = [];

afterEach(() => {
  for (const dir of temps.splice(0)) {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

function tempRoot(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "movie-harness-test-"));
  temps.push(dir);
  return dir;
}

describe("MovieSession frames path", () => {
  it("encodes frames, writes poster+video, and updates manifest", async () => {
    const root = tempRoot();
    const session = MovieSession.create({
      feature: "demo-journey",
      slug: "flow",
      root,
      size: { width: 160, height: 90 },
      fps: 8,
      source: "frames",
      title: "Flow",
      description: "Synthetic color frames",
    });

    const colors: Array<[number, number, number]> = [
      [124, 92, 255],
      [80, 220, 160],
      [240, 114, 122],
      [90, 200, 250],
    ];
    for (const rgb of colors) {
      session.pushFrame(encodeSolidPng(160, 90, rgb));
    }
    session.mark("mid", "halfway");

    const artifact = await session.stop({ status: "pass" });

    expect(fs.existsSync(artifact.posterPath)).toBe(true);
    expect(fs.existsSync(artifact.videoPath)).toBe(true);
    expect(path.extname(artifact.videoPath)).toBe(".webm");
    expect(artifact.sequence).toBe("0001");
    expect(artifact.chapters).toHaveLength(1);

    const manifest = JSON.parse(
      fs.readFileSync(
        path.join(root, ".astroshot", "demo-journey", "manifest.json"),
        "utf8",
      ),
    );
    expect(manifest.feature).toBe("demo-journey");
    expect(manifest.status).toBe("pass");
    expect(manifest.shots).toHaveLength(1);
    expect(manifest.shots[0].kind).toBe("movie");
    expect(manifest.shots[0].video).toBe("0001-flow.webm");
    expect(manifest.shots[0].file).toBe("0001-flow.png");
    expect(manifest.shots[0].chapters[0].slug).toBe("mid");
  }, 120_000);
});

describe("persisted frames CLI store", () => {
  it("survives start / push / mark / stop across loads", async () => {
    const root = tempRoot();
    const started = startFrameSession({
      feature: "cli-frames",
      slug: "walk",
      root,
      size: { width: 120, height: 80 },
      fps: 6,
    });
    expect(started.frameCount).toBe(0);

    const frame = path.join(root, "a.png");
    fs.writeFileSync(frame, encodeSolidPng(120, 80, [124, 92, 255]));
    pushFrameToSession(started, frame);
    pushFrameToSession(
      loadFrameSession(root, "cli-frames", started.id),
      frame,
    );
    markFrameSession(
      loadFrameSession(root, "cli-frames", started.id),
      "step-a",
    );

    const artifact = await stopFrameSession(
      loadFrameSession(root, "cli-frames", started.id),
      { status: "pass" },
    );
    expect(artifact.sequence).toBe("0001");
    expect(fs.existsSync(artifact.videoPath)).toBe(true);
    expect(artifact.chapters[0]?.slug).toBe("step-a");
  }, 120_000);
});

describe("truecolor paint path", () => {
  it("preserves SGR truecolor in HTML", async () => {
    const terminal = createHeadlessTerminal(24, 2);
    try {
      await writeTerminal(
        terminal,
        "\u001b[38;2;124;92;255mPurple frame\u001b[0m",
      );
      const html = terminalToHtml(terminal, {
        cols: 24,
        rows: 2,
        foreground: "#ffffff",
        background: "#090a12",
      });
      expect(html).toContain("color:#7c5cff");
      expect(html).toContain("Purple frame");
    } finally {
      terminal.dispose();
    }
  });

  it("records a truecolor pty-demo movie into .astroshot", async () => {
    const root = tempRoot();
    const artifact = await recordTruecolorDemoMovie({
      feature: "pty-color",
      slug: "brand",
      root,
      fps: 8,
      status: "pass",
    });
    expect(fs.existsSync(artifact.posterPath)).toBe(true);
    expect(fs.existsSync(artifact.videoPath)).toBe(true);
    expect(artifact.source).toBe("pty");

    const manifest = JSON.parse(
      fs.readFileSync(
        path.join(root, ".astroshot", "pty-color", "manifest.json"),
        "utf8",
      ),
    );
    expect(manifest.shots[0].source).toBe("pty");
    expect(manifest.shots[0].kind).toBe("movie");
  }, 120_000);
});
