/**
 * `astroshot doctor` — report every way Astroshots setup silently fails.
 *
 * Read-only by contract: it never installs, launches, or writes preferences.
 * Each check prints pass/fail plus the exact remediation command, and required
 * failures make the process exit non-zero.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { loadDemoFixtures } from "./demo.mjs";
import {
  ASTROSHOTS_DOMAIN,
  evaluateWatchCoverage,
  preferenceToolsAvailable,
  readWatchConfiguration,
} from "./mac-preferences.mjs";

const APP_PROCESS_NAME = "Astroshots";

export function doctorHelp() {
  return `astroshot doctor — diagnose Astroshots capture setup (read-only)

Usage:
  astroshot doctor [options]

Options:
  --root <dir>     Project directory to test for watch coverage (default: cwd)
  --json           Machine-readable report
  --skip-screen    Skip the macOS Screen Recording probe (avoids a Swift run)
  -h, --help       Show this help

Reports Node version, watched-folder coverage, whether the Astroshots app is
installed and running, the managed Chromium runtime, and macOS Screen Recording
state for desktop.window. Exits non-zero when a required check fails.
doctor never installs anything and never changes app state.`;
}

function readEngines() {
  const packageJSON = JSON.parse(
    fs.readFileSync(new URL("../package.json", import.meta.url), "utf8"),
  );
  return packageJSON.engines?.node ?? ">=22.14.0";
}

function parseVersion(value) {
  const match = /(\d+)\.(\d+)\.(\d+)/.exec(value);
  if (!match) return null;
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function compareVersions(a, b) {
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) return a[index] < b[index] ? -1 : 1;
  }
  return 0;
}

function checkNode(range) {
  const minimum = parseVersion(range);
  const current = parseVersion(process.versions.node);
  if (!minimum || !current) {
    return {
      id: "node",
      title: "Node.js version",
      required: true,
      status: "warn",
      detail: `could not compare ${process.versions.node} with "${range}"`,
      remediation: "Install Node.js 22.14.0 or newer: https://nodejs.org/en/download",
    };
  }
  const ok = compareVersions(current, minimum) >= 0;
  return {
    id: "node",
    title: "Node.js version",
    required: true,
    status: ok ? "pass" : "fail",
    detail: ok
      ? `${process.versions.node} satisfies ${range}`
      : `${process.versions.node} is older than required ${range}`,
    remediation: "nvm install 22.14.0 && nvm use 22.14.0",
  };
}

function checkDemoFixtures() {
  try {
    const index = loadDemoFixtures();
    const movies = index.shots.filter((shot) => shot.video).length;
    return {
      id: "demo-fixtures",
      title: "Bundled demo payload",
      required: true,
      status: "pass",
      detail: `${index.shots.length} fixtures (${movies} movie) ready for "astroshot demo"`,
      remediation: "npm install --global @archastro/astroshot",
    };
  } catch (error) {
    return {
      id: "demo-fixtures",
      title: "Bundled demo payload",
      required: true,
      status: "fail",
      detail: error instanceof Error ? error.message : String(error),
      remediation: "npm install --global @archastro/astroshot",
    };
  }
}

export function checkWatchCoverage(
  projectPath,
  platform,
  // Seam for tests: the tool-failure path is otherwise only reachable by
  // breaking the host's /usr/bin.
  { readConfiguration = readWatchConfiguration } = {},
) {
  const base = { id: "watch-roots", title: "Watched folder covers this project" };
  if (platform !== "darwin") {
    return {
      ...base,
      required: false,
      status: "skip",
      detail: "the Astroshots app is macOS-only; .astroshot files are still written",
      remediation: "Run captures on macOS to review them in the Astroshots tray",
    };
  }

  const configuration = readConfiguration();
  const coverage = evaluateWatchCoverage(projectPath, configuration);
  const source =
    configuration.source === "plist-file"
      ? ` (read from ${configuration.plistPath}; cfprefsd unavailable)`
      : "";
  const legacy = configuration.usedLegacyKey ? " via legacy watchRoot key" : "";

  switch (coverage.state) {
    case "inside-root":
      return {
        ...base,
        required: true,
        status: "pass",
        detail: `${coverage.projectPath} is inside ${coverage.matchedRoot}${legacy}${source}`,
        remediation: "Astroshots menu-bar icon → gear → Add folders…",
        data: coverage,
      };
    case "setup-incomplete":
      return {
        ...base,
        required: true,
        status: "fail",
        detail:
          "first-launch folder setup has not completed, so Astroshots watches nothing yet",
        remediation:
          "open -a Astroshots   # then choose the folder that holds your projects",
        data: coverage,
      };
    case "outside-roots":
      return {
        ...base,
        required: true,
        status: "fail",
        detail: `${coverage.projectPath} is outside every watched folder (${coverage.roots.join(", ")})`,
        remediation:
          "Astroshots menu-bar icon → gear → Add folders… → add a parent folder of this project",
        data: coverage,
      };
    default: {
      // The configuration could not be read. Say exactly that: claiming
      // "setup incomplete" here would tell a correctly configured user to redo
      // work they already did.
      const tools = preferenceToolsAvailable({ platform });
      const toolProblem =
        coverage.reason === "tool-unavailable" || !tools.available;
      const missing = tools.missing.length
        ? ` (missing ${tools.missing.join(", ")})`
        : "";
      return {
        ...base,
        required: true,
        status: "warn",
        detail: toolProblem
          ? `watch coverage is UNKNOWN, not unconfigured: could not read ${ASTROSHOTS_DOMAIN} because the macOS preference tools are unavailable${missing}`
          : `watch coverage is UNKNOWN: could not read ${ASTROSHOTS_DOMAIN} preferences (${coverage.reason ?? "unknown"})`,
        remediation: toolProblem
          ? "Re-run with /usr/bin on PATH — doctor needs /usr/bin/defaults and /usr/bin/plutil"
          : "Install and launch Astroshots once: https://github.com/ArchAstro/astroshots/releases",
        data: coverage,
      };
    }
  }
}

function findInstalledApp({ home = os.homedir() } = {}) {
  const candidates = [
    "/Applications/Astroshots.app",
    path.join(home, "Applications", "Astroshots.app"),
  ];
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate;
  }
  const found = spawnSync(
    "/usr/bin/mdfind",
    ["-0", `kMDItemCFBundleIdentifier == '${ASTROSHOTS_DOMAIN}'`],
    { encoding: "utf8" },
  );
  if (found.status === 0 && found.stdout) {
    const hit = found.stdout
      .split("\0")
      .map((entry) => entry.trim())
      .find((entry) => entry.endsWith(".app") && fs.existsSync(entry));
    if (hit) return hit;
  }
  return null;
}

function appVersion(appPath) {
  const plist = path.join(appPath, "Contents", "Info.plist");
  if (!fs.existsSync(plist)) return null;
  const result = spawnSync(
    "/usr/bin/plutil",
    ["-extract", "CFBundleShortVersionString", "raw", "-o", "-", plist],
    { encoding: "utf8" },
  );
  if (result.status !== 0) return null;
  return result.stdout.trim() || null;
}

function checkApp(platform) {
  const base = { id: "app", title: "Astroshots app installed" };
  if (platform !== "darwin") {
    return {
      ...base,
      required: false,
      status: "skip",
      detail: "macOS-only review app",
      remediation: "Review captured files directly on this platform",
    };
  }
  const appPath = findInstalledApp();
  if (!appPath) {
    return {
      ...base,
      required: true,
      status: "fail",
      detail: "no Astroshots.app found in /Applications or ~/Applications",
      remediation:
        "Download the latest DMG: https://github.com/ArchAstro/astroshots/releases",
    };
  }
  const version = appVersion(appPath);
  return {
    ...base,
    required: true,
    status: "pass",
    detail: `${appPath}${version ? ` (${version})` : ""}`,
    remediation: "open -a Astroshots",
  };
}

function checkAppRunning(platform) {
  const base = { id: "app-running", title: "Astroshots app running" };
  if (platform !== "darwin") {
    return {
      ...base,
      required: false,
      status: "skip",
      detail: "macOS-only review app",
      remediation: "Review captured files directly on this platform",
    };
  }
  const result = spawnSync("/usr/bin/pgrep", ["-x", APP_PROCESS_NAME], {
    encoding: "utf8",
  });
  const running = result.status === 0 && Boolean(result.stdout.trim());
  return {
    ...base,
    required: false,
    status: running ? "pass" : "warn",
    detail: running
      ? `pid ${result.stdout.trim().split(/\s+/).join(", ")} (menu-bar only, no Dock icon)`
      : "not running, so new captures will not stream or flash an overlay",
    remediation: "open -a Astroshots",
  };
}

function resolvePlaywright() {
  // playwright belongs to the engine packages, so resolve it from the engine
  // rather than assuming a hoisted install next to this CLI. It is CommonJS,
  // so require it: ESM interop does not expose the `chromium` named export.
  const engineEntry = fileURLToPath(
    import.meta.resolve("@archastro/react-shot"),
  );
  return createRequire(engineEntry)("playwright");
}

function checkChromium() {
  const base = {
    id: "chromium",
    title: "Managed Chromium runtime",
    required: false,
    remediation: "astroshot install-browser   # add --with-deps on Linux CI",
  };
  let executablePath;
  try {
    executablePath = resolvePlaywright().chromium.executablePath();
  } catch (error) {
    return {
      ...base,
      status: "warn",
      detail: `could not resolve Playwright (${error instanceof Error ? error.message.split("\n")[0] : error}); react/ink/pty and browser movies need it`,
    };
  }
  const present = Boolean(executablePath) && fs.existsSync(executablePath);
  return {
    ...base,
    status: present ? "pass" : "warn",
    detail: present
      ? executablePath
      : `not installed at ${executablePath} — needed for react/ink/pty stills and browser movies (not for "astroshot demo")`,
  };
}

function checkScreenRecording(platform, { skip = false } = {}) {
  const base = {
    id: "screen-recording",
    title: "Screen Recording permission (movie --source desktop.window)",
    required: false,
  };
  if (platform !== "darwin") {
    return {
      ...base,
      status: "skip",
      detail: "desktop.window is macOS-only; use --source browser, pty, or frames",
      remediation: "astroshot movie which-source \"<intent>\"",
    };
  }
  if (skip) {
    return {
      ...base,
      status: "skip",
      detail: "skipped with --skip-screen",
      remediation: "astroshot movie check-screen-access",
    };
  }

  const engineEntry = fileURLToPath(
    import.meta.resolve("@archastro/movie-harness"),
  );
  const harnessBin = path.join(
    path.dirname(path.dirname(engineEntry)),
    "bin",
    "astroshot-movie.mjs",
  );
  const result = spawnSync(
    process.execPath,
    [harnessBin, "check-screen-access"],
    { encoding: "utf8" },
  );
  let report = null;
  try {
    report = JSON.parse(result.stdout ?? "");
  } catch {
    report = null;
  }
  if (!report) {
    return {
      ...base,
      status: "warn",
      detail: `could not probe Screen Recording (${(result.stderr ?? "").trim().split("\n")[0] || `status ${result.status}`})`,
      remediation: "xcode-select --install && astroshot movie check-screen-access",
    };
  }
  return {
    ...base,
    status: report.granted ? "pass" : "warn",
    detail: report.granted
      ? `granted for ${report.enableApp}`
      : `denied — enable "${report.enableApp}" then quit and reopen it`,
    remediation: report.granted
      ? "astroshot movie check-screen-access"
      : "astroshot movie open-screen-settings",
  };
}

const SYMBOLS = { pass: "PASS", fail: "FAIL", warn: "WARN", skip: "SKIP" };

export function collectDoctorChecks({
  projectPath = process.cwd(),
  platform = process.platform,
  skipScreen = false,
} = {}) {
  return [
    checkNode(readEngines()),
    checkDemoFixtures(),
    checkApp(platform),
    checkAppRunning(platform),
    checkWatchCoverage(projectPath, platform),
    checkChromium(),
    checkScreenRecording(platform, { skip: skipScreen }),
  ];
}

export function summarizeDoctor(checks) {
  const failures = checks.filter(
    (check) => check.required && check.status === "fail",
  );
  const warnings = checks.filter((check) => check.status === "warn");
  return {
    ok: failures.length === 0,
    failures: failures.map((check) => check.id),
    warnings: warnings.map((check) => check.id),
  };
}

export function runDoctor(argv, { log = console.log } = {}) {
  const options = { root: undefined, json: false, skipScreen: false };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "-h" || token === "--help" || token === "help") {
      log(doctorHelp());
      return 0;
    }
    if (token === "--json") {
      options.json = true;
      continue;
    }
    if (token === "--skip-screen") {
      options.skipScreen = true;
      continue;
    }
    if (token === "--root") {
      const value = argv[index + 1];
      if (!value || value.startsWith("-")) throw new Error("--root requires a value");
      options.root = value;
      index += 1;
      continue;
    }
    throw new Error(`Unknown doctor argument: ${token}`);
  }

  const projectPath = resolveProjectPath(options.root);
  const checks = collectDoctorChecks({
    projectPath,
    skipScreen: options.skipScreen,
  });
  const summary = summarizeDoctor(checks);

  if (options.json) {
    log(JSON.stringify({ project: projectPath, ...summary, checks }, null, 2));
    return summary.ok ? 0 : 1;
  }

  log(`astroshot doctor — ${projectPath}`);
  log("");
  for (const check of checks) {
    const scope = check.required ? "required" : "optional";
    log(`${SYMBOLS[check.status]}  ${check.title} [${scope}]`);
    log(`      ${check.detail}`);
    if (check.status === "fail" || check.status === "warn") {
      log(`      fix: ${check.remediation}`);
    }
  }
  log("");
  if (summary.ok) {
    log(
      summary.warnings.length
        ? `All required checks pass (${summary.warnings.length} optional warning${summary.warnings.length === 1 ? "" : "s"} above).`
        : "All checks pass. Try: astroshot demo",
    );
    return 0;
  }
  log(
    `${summary.failures.length} required check${summary.failures.length === 1 ? "" : "s"} failed: ${summary.failures.join(", ")}`,
  );
  log("Run the fix line under each failure, then re-run: astroshot doctor");
  return 1;
}

function resolveProjectPath(explicitRoot) {
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
