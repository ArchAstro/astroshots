import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  DEFAULT_DEMO_FEATURE,
  loadDemoFixtures,
  writeDemo,
} from "../bin/demo.mjs";
import {
  checkWatchCoverage,
  collectDoctorChecks,
  summarizeDoctor,
} from "../bin/doctor.mjs";
import {
  evaluateWatchCoverage,
  extractPreferenceKey,
  normalizeWatchRootPaths,
  pathIsInside,
  preferenceToolsAvailable,
  readWatchConfiguration,
} from "../bin/mac-preferences.mjs";

const packageRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const executable = path.join(packageRoot, "bin", "astroshot.mjs");
const PNG_MAGIC = "89504e470d0a1a0a";

function runIn(cwd, ...arguments_) {
  return spawnSync(process.execPath, [executable, ...arguments_], {
    cwd,
    encoding: "utf8",
  });
}

function temporaryProject() {
  const directory = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-demo-")),
  );
  spawnSync("git", ["init", "-q", "."], { cwd: directory });
  return directory;
}

function magic(filePath, bytes) {
  const handle = fs.openSync(filePath, "r");
  try {
    const buffer = Buffer.alloc(bytes);
    fs.readSync(handle, buffer, 0, bytes, 0);
    return buffer.toString("hex");
  } finally {
    fs.closeSync(handle);
  }
}

test("demo writes stills, a movie pair, and a contract-valid manifest", () => {
  const project = temporaryProject();
  try {
    const result = runIn(project, "demo");
    assert.equal(result.status, 0, result.stderr);

    const featureDirectory = path.join(
      project,
      ".astroshot",
      DEFAULT_DEMO_FEATURE,
    );
    const manifest = JSON.parse(
      fs.readFileSync(path.join(featureDirectory, "manifest.json"), "utf8"),
    );

    assert.equal(manifest.version, 1);
    assert.equal(manifest.feature, DEFAULT_DEMO_FEATURE);
    assert.match(manifest.run_id, /^astroshot-demo-\d{8}T\d{6}Z-\d+-\d{4}$/);
    assert.ok(["running", "pass", "fail", "idle"].includes(manifest.status));
    assert.ok(Array.isArray(manifest.shots) && manifest.shots.length >= 2);

    for (const shot of manifest.shots) {
      assert.equal(shot.file, path.basename(shot.file));
      assert.match(shot.file, /^\d{4}-[a-z0-9-]+\.png$/);
      assert.equal(shot.id, shot.file.slice(0, 4));
      assert.equal(shot.slug, shot.file.slice(5).replace(/\.png$/, ""));
      assert.ok(shot.title && shot.description);
      assert.ok(!Number.isNaN(Date.parse(shot.captured_at)));
      const image = path.join(featureDirectory, shot.file);
      assert.ok(fs.existsSync(image), `missing ${shot.file}`);
      assert.equal(magic(image, 8), PNG_MAGIC, `${shot.file} is not a PNG`);
    }

    const stills = manifest.shots.filter((shot) => !shot.video);
    const movies = manifest.shots.filter((shot) => shot.video);
    assert.ok(stills.length >= 1, "demo must include at least one still");
    assert.equal(movies.length, 1, "demo must include exactly one movie");

    const [movie] = movies;
    assert.equal(movie.kind, "movie");
    assert.equal(movie.video, path.basename(movie.video));
    assert.match(movie.video, /\.(webm|mp4|mov)$/);
    // The poster keeps the still-image contract so review stays hash-keyed.
    assert.equal(
      path.parse(movie.video).name,
      path.parse(movie.file).name,
      "movie video must be the poster's sibling",
    );
    const video = path.join(featureDirectory, movie.video);
    assert.ok(fs.existsSync(video));
    assert.ok(fs.statSync(video).size > 0);
    assert.equal(typeof movie.duration_ms, "number");
    assert.ok(movie.duration_ms > 0);
    assert.ok(["browser", "pty", "desktop.window", "frames"].includes(movie.source));
    for (const chapter of movie.chapters ?? []) {
      assert.match(chapter.slug, /^[a-z0-9-]+$/);
      assert.equal(typeof chapter.t_ms, "number");
    }

    // No temp artifacts leak from the atomic writes.
    const leftovers = fs
      .readdirSync(featureDirectory)
      .filter((entry) => entry.startsWith("."));
    assert.deepEqual(leftovers, []);

    assert.match(result.stdout, /0001-welcome\.png/);
    assert.match(result.stdout, /manifest\.json/);
  } finally {
    fs.rmSync(project, { recursive: true, force: true });
  }
});

test("demo needs no Chromium, no ffmpeg, and no user assets", () => {
  const project = temporaryProject();
  const emptyBrowsers = fs.mkdtempSync(path.join(os.tmpdir(), "astroshot-nobrowser-"));
  try {
    const result = spawnSync(process.execPath, [executable, "demo", "--json"], {
      cwd: project,
      encoding: "utf8",
      env: {
        ...process.env,
        PLAYWRIGHT_BROWSERS_PATH: emptyBrowsers,
        // Empty PATH removes ffmpeg, git, and every other helper binary.
        PATH: "",
      },
    });
    assert.equal(result.status, 0, result.stderr);
    const report = JSON.parse(result.stdout);
    assert.equal(report.root, project);
    assert.ok(report.files.length >= 4);
    for (const file of report.files) assert.ok(fs.existsSync(file));
  } finally {
    fs.rmSync(project, { recursive: true, force: true });
    fs.rmSync(emptyBrowsers, { recursive: true, force: true });
  }
});

test("demo re-runs as a fresh run without clobbering earlier frames", () => {
  const project = temporaryProject();
  try {
    const first = writeDemo({ root: project });
    const second = writeDemo({ root: project });
    assert.notEqual(first.runId, second.runId);
    assert.equal(second.shots[0].id, "0004");
    for (const file of [...first.files, ...second.files]) {
      assert.ok(fs.existsSync(file));
    }
    const manifest = JSON.parse(fs.readFileSync(second.manifestPath, "utf8"));
    assert.equal(manifest.run_id, second.runId);
    assert.equal(manifest.shots.length, second.shots.length);
  } finally {
    fs.rmSync(project, { recursive: true, force: true });
  }
});

test("demo honors --feature and rejects unsupported input", () => {
  const project = temporaryProject();
  try {
    const named = runIn(project, "demo", "--feature", "quickstart-proof");
    assert.equal(named.status, 0, named.stderr);
    assert.ok(
      fs.existsSync(path.join(project, ".astroshot", "quickstart-proof", "manifest.json")),
    );

    const bad = runIn(project, "demo", "--feature", "Not Kebab");
    assert.equal(bad.status, 1);
    assert.match(bad.stderr, /kebab-case/);

    const unknown = runIn(project, "demo", "--wat");
    assert.equal(unknown.status, 1);
    assert.match(unknown.stderr, /Unknown demo argument/);

    const missingValue = runIn(project, "demo", "--feature");
    assert.equal(missingValue.status, 1);
    assert.match(missingValue.stderr, /requires a value/);
  } finally {
    fs.rmSync(project, { recursive: true, force: true });
  }
});

test("bundled demo fixtures are shipped and self-consistent", () => {
  const index = loadDemoFixtures();
  assert.equal(index.version, 1);
  assert.ok(index.shots.some((shot) => shot.video));
  assert.ok(index.shots.some((shot) => !shot.video));
  const packageJSON = JSON.parse(
    fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
  );
  assert.ok(
    packageJSON.files.includes("fixtures"),
    "fixtures/ must ship in the npm tarball",
  );
});

test("doctor reports every check with pass/fail and a remediation line", () => {
  const checks = collectDoctorChecks({ skipScreen: true });
  const ids = checks.map((check) => check.id);
  for (const id of [
    "node",
    "watch-roots",
    "app",
    "app-running",
    "chromium",
    "screen-recording",
  ]) {
    assert.ok(ids.includes(id), `doctor is missing the ${id} check`);
  }
  for (const check of checks) {
    assert.ok(["pass", "fail", "warn", "skip"].includes(check.status), check.id);
    assert.equal(typeof check.title, "string");
    assert.ok(check.detail.length > 0, `${check.id} has no detail`);
    assert.ok(check.remediation.length > 0, `${check.id} has no remediation`);
    assert.equal(typeof check.required, "boolean");
  }
});

test("doctor exit status follows required checks only", () => {
  assert.equal(
    summarizeDoctor([
      { id: "node", required: true, status: "pass" },
      { id: "chromium", required: false, status: "warn" },
    ]).ok,
    true,
  );
  const failed = summarizeDoctor([
    { id: "node", required: true, status: "pass" },
    { id: "watch-roots", required: true, status: "fail" },
    { id: "chromium", required: false, status: "fail" },
  ]);
  assert.equal(failed.ok, false);
  assert.deepEqual(failed.failures, ["watch-roots"]);
});

test("doctor prints a fix line for each failure and never mutates state", () => {
  const project = temporaryProject();
  try {
    const result = runIn(project, "doctor", "--skip-screen");
    assert.ok([0, 1].includes(result.status), result.stderr);
    assert.match(result.stdout, /astroshot doctor —/);
    assert.match(result.stdout, /Node\.js version \[required\]/);
    // Watch coverage is only enforceable where the review app can run; off
    // macOS doctor reports it as an optional skip, so assert the tag the
    // platform actually contracts for instead of assuming macOS.
    const watchTag = process.platform === "darwin" ? "required" : "optional";
    assert.match(
      result.stdout,
      new RegExp(`Watched folder covers this project \\[${watchTag}\\]`),
    );
    assert.match(result.stdout, /Managed Chromium runtime \[optional\]/);
    const failureLines = result.stdout
      .split("\n")
      .filter((line) => line.startsWith("FAIL") || line.startsWith("WARN"));
    const fixLines = result.stdout
      .split("\n")
      .filter((line) => line.trim().startsWith("fix:"));
    assert.equal(fixLines.length, failureLines.length);
    if (result.status === 1) {
      assert.match(result.stdout, /required check(s)? failed/);
    }
    // doctor is a reporter: it must not create .astroshot or touch the project.
    assert.equal(fs.existsSync(path.join(project, ".astroshot")), false);
  } finally {
    fs.rmSync(project, { recursive: true, force: true });
  }
});

test("doctor --json is machine readable and reports the project it checked", () => {
  const project = temporaryProject();
  try {
    const result = runIn(project, "doctor", "--json", "--skip-screen");
    const report = JSON.parse(result.stdout);
    assert.equal(report.project, project);
    assert.equal(typeof report.ok, "boolean");
    assert.equal(report.ok, result.status === 0);
    assert.ok(Array.isArray(report.checks));
    assert.ok(Array.isArray(report.failures));
  } finally {
    fs.rmSync(project, { recursive: true, force: true });
  }
});

test("watch-root normalization matches the app's rules", () => {
  const home = "/Users/tester";
  assert.deepEqual(
    normalizeWatchRootPaths(["~/proj", "/Users/tester/proj", "/Users/tester/proj/inner"], {
      home,
    }),
    ["/Users/tester/proj"],
    "duplicates and covered children collapse into the parent root",
  );
  assert.deepEqual(
    normalizeWatchRootPaths(["/Users/tester/proj/inner", "/Users/tester/proj"], { home }),
    ["/Users/tester/proj"],
    "a later parent replaces an already-covered child",
  );
  assert.deepEqual(normalizeWatchRootPaths(["", null, 3], { home }), []);

  // Component-boundary matching: proj-two is not inside proj.
  assert.equal(pathIsInside("/Users/tester/proj", "/Users/tester/proj/app"), true);
  assert.equal(pathIsInside("/Users/tester/proj", "/Users/tester/proj"), true);
  assert.equal(pathIsInside("/Users/tester/proj", "/Users/tester/proj-two"), false);
  assert.equal(pathIsInside("/", "/Users/tester"), true);
});

test("watch coverage keeps setup-incomplete distinct from outside-roots", () => {
  const home = "/Users/tester";
  const unavailable = { available: false, reason: "domain-not-found", roots: [] };
  assert.equal(
    evaluateWatchCoverage("/Users/tester/proj", unavailable, { home }).state,
    "unknown",
  );
  assert.equal(
    evaluateWatchCoverage(
      "/Users/tester/proj",
      { available: false, reason: "not-macos", roots: [] },
      { home },
    ).state,
    "unsupported",
  );

  const neverSetUp = {
    available: true,
    roots: [],
    hasCompletedFirstRunSetup: false,
  };
  assert.equal(
    evaluateWatchCoverage("/Users/tester/proj", neverSetUp, { home }).state,
    "setup-incomplete",
  );

  const configured = {
    available: true,
    roots: ["/Users/tester/watched"],
    hasCompletedFirstRunSetup: true,
  };
  assert.equal(
    evaluateWatchCoverage("/Users/tester/elsewhere", configured, { home }).state,
    "outside-roots",
  );
  const inside = evaluateWatchCoverage(
    "/Users/tester/watched/app",
    configured,
    { home },
  );
  assert.equal(inside.state, "inside-root");
  assert.equal(inside.matchedRoot, "/Users/tester/watched");
  assert.equal(
    evaluateWatchCoverage("/Users/tester/watched-two", configured, { home }).state,
    "outside-roots",
    "sibling paths sharing a prefix must not read as watched",
  );
});

test("a missing preference key is absence, not a failure", (t) => {
  if (process.platform !== "darwin") return t.skip("macOS plutil only");
  const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>watchRoots</key>
  <array><string>/Users/tester/watched</string></array>
  <key>watchRoot</key>
  <string>/Users/tester/watched</string>
  <key>hasCompletedFirstRunSetup</key>
  <true/>
  <key>NSOSPLastRootDirectory</key>
  <data>YWJjZA==</data>
</dict>
</plist>
`;
  const roots = extractPreferenceKey(plist, "watchRoots", "json");
  assert.equal(roots.present, true);
  assert.deepEqual(JSON.parse(roots.raw), ["/Users/tester/watched"]);

  // Scalars need raw, and CFData in the domain must not break per-key reads.
  assert.equal(
    extractPreferenceKey(plist, "watchRoot", "raw").raw.trim(),
    "/Users/tester/watched",
  );
  assert.equal(
    extractPreferenceKey(plist, "hasCompletedFirstRunSetup", "raw").raw.trim(),
    "true",
  );

  const absent = extractPreferenceKey(plist, "watchRootsNope", "json");
  assert.equal(absent.present, false);
  assert.equal(absent.error, undefined, "absence must not surface as an error");
});

test("an unreadable plist is a tool failure, not an absent key", (t) => {
  if (process.platform !== "darwin") return t.skip("macOS plutil only");
  // A key that exists but cannot be decoded, and a snapshot plutil cannot read
  // at all, must both be distinguishable from genuine absence.
  const corrupt = extractPreferenceKey("this is not a plist", "watchRoots", "json");
  assert.equal(corrupt.present, false);
  assert.equal(corrupt.failed, true, "a parse failure must be flagged as failed");
  assert.ok(corrupt.error);

  const empty = extractPreferenceKey("", "watchRoots", "json");
  assert.equal(empty.present, false);
  assert.equal(empty.failed, true);
});

test("doctor never claims setup-incomplete when the tools are unreadable", () => {
  if (process.platform !== "darwin") return;
  // Regression: when `defaults`/`plutil` cannot answer (empty or minimal PATH),
  // every key read as missing and doctor told correctly configured users to
  // redo first-run setup. Unknown must stay unknown.
  const check = checkWatchCoverage("/Users/tester/proj", "darwin", {
    readConfiguration: () => ({
      available: false,
      reason: "tool-unavailable",
      error: "spawn /usr/bin/plutil ENOENT",
      source: null,
      roots: [],
      hasCompletedFirstRunSetup: false,
      usedLegacyKey: false,
    }),
  });

  assert.equal(check.id, "watch-roots");
  assert.notEqual(check.status, "fail", "an unreadable config must not fail the user");
  assert.equal(check.status, "warn");
  assert.match(check.detail, /UNKNOWN/);
  assert.doesNotMatch(
    check.detail,
    /setup has not completed|no watched folders|outside every watched folder/i,
    "doctor must not accuse the user of skipping setup it could not read",
  );
  assert.match(check.remediation, /\/usr\/bin/);
  assert.equal(check.data.state, "unknown");
});

test("doctor still reports the real three states when the config is readable", () => {
  if (process.platform !== "darwin") return;
  const base = {
    available: true,
    source: "cfprefsd",
    usedLegacyKey: false,
  };
  const neverSetUp = checkWatchCoverage("/Users/tester/proj", "darwin", {
    readConfiguration: () => ({
      ...base,
      roots: [],
      hasCompletedFirstRunSetup: false,
    }),
  });
  assert.equal(neverSetUp.status, "fail");
  assert.match(neverSetUp.detail, /first-launch folder setup/);

  const outside = checkWatchCoverage("/Users/tester/elsewhere", "darwin", {
    readConfiguration: () => ({
      ...base,
      roots: ["/Users/tester/watched"],
      hasCompletedFirstRunSetup: true,
    }),
  });
  assert.equal(outside.status, "fail");
  assert.match(outside.detail, /outside every watched folder/);

  const inside = checkWatchCoverage("/Users/tester/watched/app", "darwin", {
    readConfiguration: () => ({
      ...base,
      roots: ["/Users/tester/watched"],
      hasCompletedFirstRunSetup: true,
    }),
  });
  assert.equal(inside.status, "pass");
});

test("the macOS preference tools are resolved by absolute path", () => {
  if (process.platform !== "darwin") return;
  const tools = preferenceToolsAvailable({ platform: "darwin" });
  assert.equal(tools.available, true, "/usr/bin/defaults and /usr/bin/plutil must exist");
  assert.deepEqual(tools.missing, []);
  // An empty PATH must not change the answer: absolute paths are used.
  const probe = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "-e",
      `import { readWatchConfiguration } from ${JSON.stringify(
        path.join(packageRoot, "bin", "mac-preferences.mjs"),
      )};
       const config = readWatchConfiguration();
       process.stdout.write(JSON.stringify({
         available: config.available,
         reason: config.reason ?? null,
       }));`,
    ],
    { encoding: "utf8", env: { ...process.env, PATH: "" } },
  );
  assert.equal(probe.status, 0, probe.stderr);
  const result = JSON.parse(probe.stdout);
  assert.equal(
    result.available,
    true,
    "an empty PATH must not make the app configuration unreadable",
  );
});

test("an unreadable snapshot never becomes an empty watch configuration", (t) => {
  if (process.platform !== "darwin") return t.skip("macOS plutil only");
  // Exercises the REAL extraction path: a snapshot plutil cannot parse must
  // surface as unreadable, not as "this user has zero watch roots". This is the
  // layer where the false "setup-incomplete" verdict was produced.
  const configuration = readWatchConfiguration({
    home: "/Users/tester",
    platform: "darwin",
    readDomain: () => ({
      available: true,
      source: "plist-file",
      plist: Buffer.from("this is not a plist"),
      plistPath: "/Users/tester/Library/Preferences/broken.plist",
      staleRisk: true,
    }),
  });

  assert.equal(
    configuration.available,
    false,
    "an unparseable snapshot must not report a readable, empty configuration",
  );
  assert.equal(configuration.reason, "tool-unavailable");
  assert.deepEqual(configuration.roots, []);

  const coverage = evaluateWatchCoverage("/Users/tester/proj", configuration, {
    home: "/Users/tester",
  });
  assert.equal(
    coverage.state,
    "unknown",
    "unreadable configuration must classify as unknown, never setup-incomplete",
  );

  const check = checkWatchCoverage("/Users/tester/proj", "darwin", {
    readConfiguration: () => configuration,
  });
  assert.equal(check.status, "warn");
  assert.doesNotMatch(check.detail, /setup has not completed/i);
});
