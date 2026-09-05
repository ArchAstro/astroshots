/**
 * Drives the built tray inside a real pseudoterminal that emulates a
 * kitty-graphics terminal, and asserts pictures land where the rows are and
 * review.json changes exactly as the macOS app would write it.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { KittyGraphicsTracker } from "@archastro/tui-shot";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { createHeadlessTerminal, terminalPlainText } from "../../tui-shot/dist/terminal-html.js";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const demoFixtures = path.resolve(packageRoot, "../astroshot/fixtures/demo");
const executable = path.join(packageRoot, "bin", "astroshot-review.mjs");

function seedRoot(root: string) {
  const feature = path.join(root, "demo-app", ".astroshot", "checkout");
  fs.mkdirSync(feature, { recursive: true });
  fs.copyFileSync(path.join(demoFixtures, "welcome.png"), path.join(feature, "0001-welcome.png"));
  fs.copyFileSync(path.join(demoFixtures, "next-steps.png"), path.join(feature, "0002-next-steps.png"));
  fs.copyFileSync(path.join(demoFixtures, "journey.png"), path.join(feature, "0003-journey.png"));
  fs.copyFileSync(path.join(demoFixtures, "journey.webm"), path.join(feature, "0003-journey.webm"));
  fs.writeFileSync(
    path.join(feature, "manifest.json"),
    JSON.stringify({
      version: 1,
      feature: "checkout",
      run_id: "checkout-e2e",
      status: "pass",
      shots: [
        { id: "0001", file: "0001-welcome.png", slug: "welcome", title: "Welcome", description: "Landing state.", captured_at: "2026-09-05T14:10:00Z" },
        { id: "0002", file: "0002-next-steps.png", slug: "next-steps", title: "Next steps", description: "Confirmation.", captured_at: "2026-09-05T14:12:00Z" },
        { id: "0003", kind: "movie", file: "0003-journey.png", video: "0003-journey.webm", slug: "journey", title: "Journey", duration_ms: 4240, source: "frames", captured_at: "2026-09-05T14:20:00Z", chapters: [{ slug: "poster", t_ms: 2685 }] },
      ],
    }),
  );
  return feature;
}

interface Session {
  write(data: string): void;
  screen(): string;
  tracker: KittyGraphicsTracker;
  output(): string;
  waitFor(predicate: () => boolean, label: string, timeoutMs?: number): Promise<void>;
  close(): Promise<number | null>;
}

async function launch(root: string, cacheDir: string, cols = 140, rows = 40): Promise<Session> {
  const { spawn } = await import("node-pty");
  const terminal = createHeadlessTerminal(cols, rows);
  const child = spawn(process.execPath, [executable, "--root", root, "--no-index"], {
    name: "xterm-kitty",
    cols,
    rows,
    cwd: root,
    env: { ...process.env, TERM: "xterm-kitty", ASTROSHOT_REVIEW_CACHE_DIR: cacheDir, ASTROSHOT_REVIEW_FFMPEG: "" },
  });
  let exited: number | null = null;
  const reply = (data: string) => {
    if (exited === null) child.write(data);
  };
  const tracker = new KittyGraphicsTracker({ terminal, cols, rows, cellWidth: 9, cellHeight: 20, reply });
  terminal.onData(reply);
  let writes = Promise.resolve();
  let raw = "";
  child.onData((data) => {
    raw += data;
    writes = writes.then(() => tracker.write(data));
  });
  const exitPromise = new Promise<number | null>((resolve) => {
    child.onExit(({ exitCode }) => {
      exited = exitCode;
      resolve(exitCode);
    });
  });
  const session: Session = {
    write: (data) => child.write(data),
    screen: () => terminalPlainText(terminal, rows),
    tracker,
    output: () => raw,
    async waitFor(predicate, label, timeoutMs = 20_000) {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        await writes;
        if (predicate()) return;
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      throw new Error(`Timed out waiting for ${label}. Screen:\n${session.screen()}`);
    },
    async close() {
      // Leave any takeover or detail page first; `q` quits only from a list.
      for (const key of ["\x1b", "\x1b", "q"]) {
        if (exited !== null) break;
        child.write(key);
        await new Promise((resolve) => setTimeout(resolve, 150));
      }
      const code = await Promise.race([exitPromise, new Promise<null>((resolve) => setTimeout(() => resolve(null), 8000))]);
      if (exited === null) child.kill();
      await writes;
      terminal.dispose();
      return code;
    },
  };
  return session;
}

let root: string;
let cacheDir: string;

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-review-e2e-"));
  cacheDir = path.join(root, ".cache");
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe("astroshot review in a kitty-capable PTY", () => {
  it("streams shots with thumbnails, marks seen, and records feedback like the app", async () => {
    const feature = seedRoot(root);
    const session = await launch(root, cacheDir);
    try {
      await session.waitFor(() => session.screen().includes("Unseen (3)"), "the seeded stream");
      expect(session.output()).toContain("\x1b[?1049h");
      await session.waitFor(() => session.tracker.overlays().length >= 4, "thumbnails and the detail preview");
      const overlays = session.tracker.overlays();
      // Thumbnails sit below the header/tab/filter rows, to the right of the marker column.
      for (const overlay of overlays) {
        expect(overlay.row).toBeGreaterThanOrEqual(3);
        expect(overlay.col).toBeGreaterThanOrEqual(1);
        expect(overlay.cols).toBeGreaterThan(0);
        expect(overlay.rows).toBeGreaterThan(0);
      }
      const screen = session.screen();
      expect(screen).toContain("checkout · Journey");
      expect(screen).toContain("Movie · 4.2s");
      expect(screen).toContain("● Unseen");

      // Newest first: the movie leads. Feedback goes into review.json as a comment-only entry.
      session.write("c");
      await session.waitFor(() => session.screen().includes("Share feedback"), "the composer");
      session.write("Needs more contrast\r");
      const reviewPath = path.join(feature, "review.json");
      await session.waitFor(
        () => fs.existsSync(reviewPath) && session.screen().includes("Reviewer") && !session.screen().includes("Share feedback"),
        "the comment to land",
      );
      let review = JSON.parse(fs.readFileSync(reviewPath, "utf8"));
      expect(review.version).toBe(1);
      expect(review.run_id).toBe("checkout-e2e");
      expect(review.reviews["0003-journey.png"].comments[0].body).toBe("Needs more contrast");
      expect(review.reviews["0003-journey.png"].decision).toBeUndefined();

      // Seen removes the row from Unseen and records the hash.
      session.write("s");
      await session.waitFor(() => session.screen().includes("Unseen (2)"), "the seen count to drop");
      await session.waitFor(() => JSON.parse(fs.readFileSync(reviewPath, "utf8")).reviews["0003-journey.png"].decision === "seen", "the seen decision on disk");
      review = JSON.parse(fs.readFileSync(reviewPath, "utf8"));
      expect(review.reviews["0003-journey.png"].decision).toBe("seen");
      expect(review.reviews["0003-journey.png"].image_sha256).toMatch(/^[0-9a-f]{64}$/);
      expect(review.reviews["0003-journey.png"].comments.length).toBe(1);

      // History shows it again as Seen.
      session.write("u");
      await session.waitFor(() => session.screen().includes("History (1)"), "the history filter");
      expect(session.screen()).toContain("● Seen");

      // Full-screen review pages run siblings oldest → newest, seen ones included.
      session.write("u");
      await session.waitFor(() => session.screen().includes("Unseen (2)"), "back to unseen");
      session.write("f");
      await session.waitFor(() => session.screen().includes("Full-screen review"), "the takeover");
      expect(session.screen()).toMatch(/2 \/ 3/);
      expect(session.screen()).toContain("Next steps");
      session.write("\x1b[D");
      await session.waitFor(() => /1 \/ 3/.test(session.screen()), "the older sibling");
      expect(session.screen()).toContain("Welcome");
      session.write("\x1b[C");
      session.write("\x1b[C");
      await session.waitFor(() => /3 \/ 3/.test(session.screen()), "the newest sibling");
      expect(session.screen()).toContain("Journey");
    } finally {
      const code = await session.close();
      expect(code).toBe(0);
      expect(session.output()).toContain("a=d,d=A");
      expect(session.output()).toContain("\x1b[?1049l");
    }
  }, 60_000);

  it("ingests a new capture while running and shows friction logs", async () => {
    const feature = seedRoot(root);
    const frictionRun = path.join(root, "demo-app", ".astroshot", "friction-logs", "onboarding", "runs", "20260811T153000Z");
    fs.mkdirSync(frictionRun, { recursive: true });
    fs.writeFileSync(path.join(root, "demo-app", ".astroshot", "friction-logs", "onboarding", "prompt.md"), "# Onboarding\n");
    fs.copyFileSync(path.join(demoFixtures, "welcome.png"), path.join(frictionRun, "0001-land.png"));
    fs.writeFileSync(
      path.join(frictionRun, "log.jsonl"),
      `${JSON.stringify({ step: 1, id: "land", title: "Land on home", transcript: "I land on the home page.", screenshots: ["0001-land.png"], good: ["Fast"], improve: ["Copy is vague"] })}\n`,
    );
    const session = await launch(root, cacheDir);
    try {
      await session.waitFor(() => session.screen().includes("Unseen (3)"), "the seeded stream");
      fs.copyFileSync(path.join(demoFixtures, "next-steps.png"), path.join(feature, "0004-arrived.png"));
      await session.waitFor(() => session.screen().includes("Unseen (4)") && session.screen().includes("checkout · Arrived"), "the live arrival");
      expect(session.screen()).toContain("1 new");

      session.write("2");
      await session.waitFor(() => session.screen().includes("Onboarding"), "the friction list");
      expect(session.screen()).toContain("1 step · 1 improve");
      session.write("\r");
      await session.waitFor(() => session.screen().includes("Improve rollup · 1"), "the log detail");
      session.write("\r");
      await session.waitFor(() => session.screen().includes("I land on the home page."), "the step detail");
      expect(session.screen()).toContain("Copy is vague");
      await session.waitFor(() => session.tracker.overlays().length >= 1, "the step screenshot");
      // Seen lives on the log, like the app's list row and detail header.
      session.write("\x1b");
      await session.waitFor(() => session.screen().includes("Improve rollup · 1"), "back to the log detail");
      session.write("s");
      await session.waitFor(() => fs.existsSync(path.join(frictionRun, "review.json")), "the friction review sidecar");
      const review = JSON.parse(fs.readFileSync(path.join(frictionRun, "review.json"), "utf8"));
      expect(review.run_id).toBe("20260811T153000Z");
      expect(review.reviews["log.jsonl"].decision).toBe("seen");
    } finally {
      await session.close();
    }
  }, 60_000);
});
