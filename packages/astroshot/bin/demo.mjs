/**
 * `astroshot demo` — seed a real .astroshot/<feature>/ set with zero
 * prerequisites.
 *
 * The payload is bundled PNG/WebM bytes under fixtures/demo/, so this command
 * needs no managed Chromium, no ffmpeg, and no user-supplied assets. It only
 * writes files, so it also works while the macOS app is closed; the caller is
 * told where to look.
 */
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import {
  evaluateWatchCoverage,
  readWatchConfiguration,
} from "./mac-preferences.mjs";

const FIXTURES_DIR = fileURLToPath(new URL("../fixtures/demo/", import.meta.url));
export const DEFAULT_DEMO_FEATURE = "astroshot-demo";

export function demoHelp() {
  return `astroshot demo — write a complete .astroshot/ example set (no prerequisites)

Usage:
  astroshot demo [options]

Options:
  --feature <name>   Feature directory under .astroshot/ (default: ${DEFAULT_DEMO_FEATURE})
  --root <dir>       Worktree root (default: git root, else cwd)
  --json             Print the written paths as JSON
  --dry-run          Print planned paths without writing
  --clean            Accepted; does not delete yet
  -h, --help         Show this help

Writes two stills, one movie poster + video pair, and manifest.json using
bundled fixtures. No Chromium download, no ffmpeg, no assets of your own.
Re-running starts a fresh run and appends new numbered frames.

After it runs, open the Astroshots menu-bar icon → Shots. If nothing appears,
run "astroshot doctor".`;
}

function resolveRoot(explicitRoot) {
  if (explicitRoot) return path.resolve(explicitRoot);
  try {
    return execFileSync("git", ["rev-parse", "--show-toplevel"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return process.cwd();
  }
}

function assertKebabCase(value, label) {
  if (!/^[a-z0-9][a-z0-9-]*$/.test(value)) {
    throw new Error(
      `${label} must be kebab-case [a-z0-9-]+, got ${JSON.stringify(value)}`,
    );
  }
}

function nextSequence(featureDirectory) {
  let max = 0;
  const entries = fs.existsSync(featureDirectory)
    ? fs.readdirSync(featureDirectory)
    : [];
  for (const name of entries) {
    const match = /^(\d{4})-/.exec(name);
    if (match) max = Math.max(max, Number(match[1]));
  }
  return max + 1;
}

function writeAtomic(filePath, contents) {
  const temporary = path.join(
    path.dirname(filePath),
    `.${path.basename(filePath)}.tmp.${process.pid}.${Date.now()}`,
  );
  fs.writeFileSync(temporary, contents);
  fs.renameSync(temporary, filePath);
}

function copyAtomic(source, destination) {
  const temporary = path.join(
    path.dirname(destination),
    `.${path.basename(destination)}.tmp.${process.pid}.${Date.now()}`,
  );
  fs.copyFileSync(source, temporary);
  fs.renameSync(temporary, destination);
}

export function loadDemoFixtures(fixturesDirectory = FIXTURES_DIR) {
  const indexPath = path.join(fixturesDirectory, "fixtures.json");
  if (!fs.existsSync(indexPath)) {
    throw new Error(
      `bundled demo fixtures are missing at ${fixturesDirectory}. Reinstall @archastro/astroshot.`,
    );
  }
  const index = JSON.parse(fs.readFileSync(indexPath, "utf8"));
  if (!Array.isArray(index.shots) || index.shots.length === 0) {
    throw new Error(`bundled demo fixtures are empty: ${indexPath}`);
  }
  for (const shot of index.shots) {
    for (const asset of [shot.asset, shot.video].filter(Boolean)) {
      const assetPath = path.join(fixturesDirectory, asset);
      if (!fs.existsSync(assetPath)) {
        throw new Error(`bundled demo asset is missing: ${assetPath}`);
      }
    }
  }
  return index;
}

function publicDemoResult(plan) {
  return {
    root: plan.root,
    feature: plan.feature,
    featureDirectory: plan.featureDirectory,
    manifestPath: plan.manifestPath,
    runId: plan.runId,
    shots: plan.shots,
    files: plan.files,
  };
}

/**
 * Read-only plan of the next demo write: same root/feature resolve, kebab-case
 * check, fixtures, next sequence, and file names (including manifest.json).
 * Does not mkdir or write.
 */
export function planDemo({
  root,
  feature = DEFAULT_DEMO_FEATURE,
  fixturesDirectory = FIXTURES_DIR,
  now = new Date(),
} = {}) {
  assertKebabCase(feature, "feature");
  const resolvedRoot = resolveRoot(root);
  const featureDirectory = path.join(resolvedRoot, ".astroshot", feature);
  const index = loadDemoFixtures(fixturesDirectory);

  const startSequence = nextSequence(featureDirectory);
  const stamp = now.toISOString().replace(/[-:]/g, "").replace(/\.\d+Z$/, "Z");
  // The first sequence of this run keeps the id unique when two runs land in
  // the same second from the same process.
  const runId = `${feature}-${stamp}-${process.pid}-${String(startSequence).padStart(4, "0")}`;
  const capturedAt = now.toISOString();

  const shots = [];
  const files = [];
  const copies = [];
  index.shots.forEach((fixture, offset) => {
    const sequence = String(startSequence + offset).padStart(4, "0");
    const posterName = `${sequence}-${fixture.slug}.png`;
    const posterPath = path.join(featureDirectory, posterName);
    copies.push({
      source: path.join(fixturesDirectory, fixture.asset),
      destination: posterPath,
    });
    files.push(posterPath);

    const shot = {
      id: sequence,
      file: posterName,
      slug: fixture.slug,
      title: fixture.title,
      description: fixture.description,
      captured_at: capturedAt,
      viewport: index.viewport,
    };

    if (fixture.video) {
      const videoExtension = path.extname(fixture.video) || ".webm";
      const videoName = `${sequence}-${fixture.slug}${videoExtension}`;
      const videoPath = path.join(featureDirectory, videoName);
      copies.push({
        source: path.join(fixturesDirectory, fixture.video),
        destination: videoPath,
      });
      files.push(videoPath);
      shot.kind = "movie";
      shot.video = videoName;
      shot.duration_ms = fixture.duration_ms;
      shot.source = fixture.source ?? "frames";
      if (Array.isArray(fixture.chapters)) shot.chapters = fixture.chapters;
    }

    shots.push(shot);
  });

  const manifestPath = path.join(featureDirectory, "manifest.json");
  const manifest = {
    version: 1,
    feature,
    run_id: runId,
    status: "pass",
    description:
      "Synthetic proof set written by `astroshot demo` — stills plus one journey movie.",
    shots,
  };
  files.push(manifestPath);

  return {
    root: resolvedRoot,
    feature,
    featureDirectory,
    manifestPath,
    manifest,
    runId,
    shots,
    files,
    copies,
  };
}

/**
 * Write the demo set and return the paths, without printing anything.
 *
 * Each invocation is its own run: a new `run_id` with a fresh shot list, while
 * earlier numbered frames stay on disk as prior-run evidence. That matches the
 * documented lifecycle used by astroshot-capture and the movie harness.
 */
export function writeDemo(options = {}) {
  const plan = planDemo(options);
  fs.mkdirSync(plan.featureDirectory, { recursive: true });
  for (const { source, destination } of plan.copies) {
    copyAtomic(source, destination);
  }
  writeAtomic(plan.manifestPath, `${JSON.stringify(plan.manifest, null, 2)}\n`);
  return publicDemoResult(plan);
}

function coverageAdvice(root) {
  const configuration = readWatchConfiguration();
  const coverage = evaluateWatchCoverage(root, configuration);
  switch (coverage.state) {
    case "inside-root":
      return [
        `Watched by Astroshots via ${coverage.matchedRoot}.`,
        "Open the Astroshots menu-bar icon → Shots to see these frames.",
      ];
    case "setup-incomplete":
      return [
        "Astroshots has no watched folders yet, so it will not pick these up.",
        "Fix: open Astroshots and choose a folder, or menu-bar icon → gear → Add folders…",
        "Then re-check with: astroshot doctor",
      ];
    case "outside-roots":
      return [
        `This project is outside every watched folder (${coverage.roots.join(", ")}).`,
        "Fix: Astroshots menu-bar icon → gear → Add folders… and add a parent of this project.",
        "Then re-check with: astroshot doctor",
      ];
    case "unsupported":
      return [
        "The Astroshots app is macOS-only; the files above still follow the .astroshot contract.",
      ];
    default:
      // Unknown — do not claim the user has no watched folders.
      return [
        "Could not read Astroshots' watched folders, so whether this project is",
        "watched is unknown (the files above were still written correctly).",
        "Check with: astroshot doctor",
      ];
  }
}

export function runDemo(argv, { log = console.log } = {}) {
  const options = {
    feature: DEFAULT_DEMO_FEATURE,
    root: undefined,
    json: false,
    dryRun: false,
    clean: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "-h" || token === "--help" || token === "help") {
      log(demoHelp());
      return 0;
    }
    if (token === "--json") {
      options.json = true;
      continue;
    }
    if (token === "--dry-run") {
      options.dryRun = true;
      continue;
    }
    if (token === "--clean") {
      options.clean = true;
      continue;
    }
    if (token === "--feature" || token === "--root") {
      const value = argv[index + 1];
      if (!value || value.startsWith("-")) {
        throw new Error(`${token} requires a value`);
      }
      options[token === "--feature" ? "feature" : "root"] = value;
      index += 1;
      continue;
    }
    throw new Error(`Unknown demo argument: ${token}`);
  }

  const result = options.dryRun
    ? publicDemoResult(planDemo(options))
    : writeDemo(options);
  if (options.json) {
    log(JSON.stringify({ ...result, advice: coverageAdvice(result.root) }, null, 2));
    return 0;
  }

  log(`astroshot demo → ${result.featureDirectory}`);
  for (const file of result.files) {
    log(`  ${path.relative(result.root, file)}`);
  }
  if (options.dryRun) return 0;

  const movies = result.shots.filter((shot) => shot.kind === "movie").length;
  const stills = result.shots.length - movies;
  log("");
  log(
    `Wrote ${stills} still${stills === 1 ? "" : "s"}, ${movies} movie${movies === 1 ? "" : "s"} (poster + video), and manifest.json.`,
  );
  for (const line of coverageAdvice(result.root)) log(line);
  return 0;
}
