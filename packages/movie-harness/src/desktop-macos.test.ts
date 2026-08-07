import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { describe, expect, it } from "vitest";

import {
  checkScreenRecordingAccess,
  formatScreenRecordingDeniedHelp,
  listDesktopWindows,
  matchDesktopWindow,
  recordDesktopWindowMovie,
} from "./sources/desktop-macos.js";

const isMac = process.platform === "darwin";

describe.skipIf(!isMac)("desktop.window macOS", () => {
  it("reports Screen Recording TCC preflight as JSON-shaped status", () => {
    const report = checkScreenRecordingAccess({ request: false });
    expect(typeof report.granted).toBe("boolean");
    expect(report.enableApp.length).toBeGreaterThan(0);
    expect(formatScreenRecordingDeniedHelp(report)).toContain(
      "Screen Recording",
    );
    expect(formatScreenRecordingDeniedHelp(report)).toContain(
      "open-screen-settings",
    );
    expect(formatScreenRecordingDeniedHelp(report)).toContain(report.enableApp);
  });

  it("lists windows with ids", () => {
    const windows = listDesktopWindows();
    expect(windows.length).toBeGreaterThan(0);
    expect(windows[0]?.id).toBeTypeOf("number");
    expect(windows[0]?.width).toBeGreaterThan(0);
  });

  it("matches by window id and records a short movie", async () => {
    const windows = listDesktopWindows().filter(
      (w) => w.width >= 200 && w.height >= 200 && w.onScreen,
    );
    expect(windows.length).toBeGreaterThan(0);
    const target = windows[0]!;
    const matched = matchDesktopWindow(windows, { windowId: target.id });
    expect(matched.id).toBe(target.id);

    const root = fs.mkdtempSync(path.join(os.tmpdir(), "desktop-movie-"));
    try {
      const artifact = await recordDesktopWindowMovie({
        feature: "desktop-smoke",
        slug: "window",
        root,
        match: { windowId: target.id },
        durationMs: 400,
        fps: 5,
        status: "pass",
      });
      expect(fs.existsSync(artifact.posterPath)).toBe(true);
      expect(fs.existsSync(artifact.videoPath)).toBe(true);
      expect(artifact.source).toBe("desktop.window");
      const manifest = JSON.parse(
        fs.readFileSync(
          path.join(root, ".astroshot", "desktop-smoke", "manifest.json"),
          "utf8",
        ),
      );
      expect(manifest.shots[0].source).toBe("desktop.window");
      expect(manifest.shots[0].kind).toBe("movie");
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  }, 120_000);
});
