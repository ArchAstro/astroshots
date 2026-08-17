/**
 * Read-only access to the Astroshots macOS app's real preferences.
 *
 * Two traps make the obvious implementations wrong:
 *
 * 1. Whole-domain plist → JSON conversion can never work. The domain contains
 *    AppKit's `NSOSPLastRootDirectory` open-panel bookmark (CFData), and JSON
 *    has no representation for it, so `plutil -convert json` aborts with
 *    "Invalid object in plist for JSON format" on effectively every real user's
 *    machine. Extract one key at a time instead.
 * 2. `~/Library/Preferences/<domain>.plist` is a lazily flushed cache of
 *    cfprefsd state, so reading the file can report yesterday's configuration.
 *    Go through cfprefsd (`defaults export`) first and treat the file as a
 *    fallback only.
 *
 * Nothing here writes: `astroshot doctor` must not mutate app state.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

export const ASTROSHOTS_DOMAIN = "ai.archastro.Astroshots";

const MISSING_KEY_PATTERN =
  /No value at that key path|invalid key path|Invalid object in plist/i;

// Resolve Apple's tools by absolute path. They always live in /usr/bin, and a
// minimal or empty PATH must never be mistaken for "this user has no watch
// roots" — that is the exact false negative doctor exists to eliminate.
const DEFAULTS_BIN = "/usr/bin/defaults";
const PLUTIL_BIN = "/usr/bin/plutil";

/** Whether the tools this module shells out to are actually present. */
export function preferenceToolsAvailable({ platform = process.platform } = {}) {
  if (platform !== "darwin") return { available: false, missing: [] };
  const missing = [DEFAULTS_BIN, PLUTIL_BIN].filter(
    (binary) => !fs.existsSync(binary),
  );
  return { available: missing.length === 0, missing };
}

/**
 * Snapshot the preference domain as an XML plist.
 *
 * `defaults export` goes through cfprefsd, so it observes what the app itself
 * sees. The on-disk plist is only used when cfprefsd is unavailable.
 */
export function readPreferenceDomain(
  domain = ASTROSHOTS_DOMAIN,
  { home = os.homedir(), platform = process.platform } = {},
) {
  if (platform !== "darwin") {
    return { available: false, source: null, reason: "not-macos" };
  }

  const exported = spawnSync(DEFAULTS_BIN, ["export", domain, "-"], {
    maxBuffer: 16 * 1024 * 1024,
  });
  if (!exported.error && exported.status === 0 && exported.stdout?.length) {
    return { available: true, source: "cfprefsd", plist: exported.stdout };
  }

  const plistPath = path.join(
    home,
    "Library",
    "Preferences",
    `${domain}.plist`,
  );
  try {
    const bytes = fs.readFileSync(plistPath);
    return {
      available: true,
      source: "plist-file",
      plist: bytes,
      plistPath,
      staleRisk: true,
    };
  } catch {
    // `defaults` failing to launch is a tool failure, not evidence that the
    // domain is absent.
    if (exported.error) {
      return {
        available: false,
        source: null,
        reason: "tool-unavailable",
        error: exported.error.message,
      };
    }
    return {
      available: false,
      source: null,
      reason:
        exported.status === 1 && !exported.stdout?.length
          ? "domain-not-found"
          : "unreadable",
    };
  }
}

/**
 * Extract a single preference key from a plist snapshot.
 *
 * Three outcomes, deliberately distinct:
 *   - `{ present: true, raw }`            the key exists
 *   - `{ present: false }`                the key is genuinely absent
 *   - `{ present: false, failed: true }`  the tool could not answer
 *
 * A missing key exits non-zero with the same "Invalid object" wording as a real
 * failure, so absence is matched explicitly. Everything else — `plutil`
 * missing, a spawn error, or an unrecognized non-zero exit — is a tool failure.
 * Collapsing the third case into absence would make a correctly configured user
 * read as "setup never completed".
 */
export function extractPreferenceKey(plist, key, format = "raw") {
  const result = spawnSync(
    PLUTIL_BIN,
    ["-extract", key, format, "-o", "-", "-"],
    { input: plist, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
  );
  if (result.error) {
    return { present: false, failed: true, error: result.error.message };
  }
  if (result.status !== 0) {
    if (MISSING_KEY_PATTERN.test(result.stderr ?? "")) {
      return { present: false };
    }
    return {
      present: false,
      failed: true,
      error: (result.stderr ?? "").trim() || `plutil exited ${result.status}`,
    };
  }
  return { present: true, raw: result.stdout };
}

function extractStringArray(plist, key) {
  const extracted = extractPreferenceKey(plist, key, "json");
  if (!extracted.present) {
    return { present: false, failed: extracted.failed, error: extracted.error };
  }
  try {
    const parsed = JSON.parse(extracted.raw);
    if (!Array.isArray(parsed)) return { present: false };
    return {
      present: true,
      value: parsed.filter((entry) => typeof entry === "string"),
    };
  } catch (error) {
    // The key exists but did not decode: unreadable, not absent.
    return {
      present: false,
      failed: true,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function extractString(plist, key) {
  const extracted = extractPreferenceKey(plist, key, "raw");
  if (!extracted.present) {
    return { present: false, failed: extracted.failed, error: extracted.error };
  }
  const value = extracted.raw.replace(/\n$/, "");
  return { present: true, value };
}

function extractBoolean(plist, key) {
  const extracted = extractString(plist, key);
  if (!extracted.present) {
    return { present: false, failed: extracted.failed };
  }
  return { present: true, value: /^(true|1|yes)$/i.test(extracted.value) };
}

/**
 * Mirror of `Preferences.normalizeWatchRootPaths` in the Swift app: expand
 * tildes, standardize, resolve symlinks, drop duplicates, and drop roots
 * already covered recursively by an earlier root.
 */
export function normalizeWatchRootPaths(paths, { home = os.homedir() } = {}) {
  const result = [];
  for (const rawPath of paths) {
    if (typeof rawPath !== "string" || rawPath.length === 0) continue;
    const candidate = normalizePath(rawPath, { home });
    if (result.some((root) => pathIsInside(root, candidate))) continue;
    for (let index = result.length - 1; index >= 0; index -= 1) {
      if (pathIsInside(candidate, result[index])) result.splice(index, 1);
    }
    result.push(candidate);
  }
  return result;
}

/** Expand `~`, make absolute, and resolve symlinks as far as the path exists. */
export function normalizePath(rawPath, { home = os.homedir() } = {}) {
  let expanded = rawPath;
  if (expanded === "~") expanded = home;
  else if (expanded.startsWith("~/")) expanded = path.join(home, expanded.slice(2));
  const absolute = path.resolve(expanded);
  try {
    return fs.realpathSync.native(absolute);
  } catch {
    // Resolve the deepest existing ancestor so a missing leaf still normalizes.
    const parts = absolute.split(path.sep);
    for (let depth = parts.length - 1; depth > 1; depth -= 1) {
      const ancestor = parts.slice(0, depth).join(path.sep);
      try {
        const real = fs.realpathSync.native(ancestor);
        return path.join(real, ...parts.slice(depth));
      } catch {
        continue;
      }
    }
    return absolute;
  }
}

/**
 * Containment on path-component boundaries, so `/Users/x/proj-two` is not
 * treated as living inside `/Users/x/proj`.
 */
export function pathIsInside(root, candidate) {
  if (root === candidate) return true;
  const prefix = root.endsWith(path.sep) ? root : `${root}${path.sep}`;
  return candidate.startsWith(prefix);
}

/**
 * The app's live watch configuration.
 *
 * `watchRoots` (string array) is authoritative; the legacy singular
 * `watchRoot` is still honored when `watchRoots` was never written so upgrades
 * keep the folder the user already chose.
 */
export function readWatchConfiguration({
  domain = ASTROSHOTS_DOMAIN,
  home = os.homedir(),
  platform = process.platform,
  // Seam for tests: lets a suite supply a plist snapshot (including a corrupt
  // one) without breaking the host's /usr/bin.
  readDomain = readPreferenceDomain,
} = {}) {
  const snapshot = readDomain(domain, { home, platform });
  if (!snapshot.available) {
    return {
      available: false,
      reason: snapshot.reason,
      source: null,
      roots: [],
      hasCompletedFirstRunSetup: false,
      usedLegacyKey: false,
    };
  }

  const modern = extractStringArray(snapshot.plist, "watchRoots");
  const legacy = modern.present
    ? { present: false }
    : extractString(snapshot.plist, "watchRoot");
  const firstRun = extractBoolean(snapshot.plist, "hasCompletedFirstRunSetup");

  // If the extraction tool could not answer, we know nothing about this user's
  // setup. Reporting "no watch roots" here would tell a correctly configured
  // user to redo first-run setup, so surface the unreadable state instead.
  const failure = [modern, legacy, firstRun].find((result) => result.failed);
  if (failure) {
    return {
      available: false,
      reason: "tool-unavailable",
      error: failure.error,
      source: snapshot.source,
      roots: [],
      hasCompletedFirstRunSetup: false,
      usedLegacyKey: false,
    };
  }

  const stored = modern.present
    ? modern.value
    : legacy.present && legacy.value
      ? [legacy.value]
      : [];

  return {
    available: true,
    source: snapshot.source,
    staleRisk: Boolean(snapshot.staleRisk),
    plistPath: snapshot.plistPath,
    roots: normalizeWatchRootPaths(stored, { home }),
    storedRoots: stored,
    usedLegacyKey: !modern.present && legacy.present,
    hasCompletedFirstRunSetup: firstRun.present ? firstRun.value : false,
    hasConfiguredWatchRoots: modern.present || legacy.present,
  };
}

/**
 * Classify a project directory against the app's watch roots.
 *
 * Each outcome needs a different remediation, so they stay distinct instead of
 * collapsing into "not watched":
 *   - `setup-incomplete`  first-run folder setup never finished
 *   - `outside-roots`     setup finished, but this project is not covered
 *   - `inside-root`       covered by `matchedRoot`
 *   - `unknown`           the configuration could not be read at all — never
 *                         claim anything about the user's setup here
 *   - `unsupported`       not macOS
 */
export function evaluateWatchCoverage(
  projectPath,
  configuration,
  { home = os.homedir() } = {},
) {
  if (!configuration.available) {
    return {
      state: configuration.reason === "not-macos" ? "unsupported" : "unknown",
      reason: configuration.reason,
      error: configuration.error,
      roots: [],
    };
  }
  const normalizedProject = normalizePath(projectPath, { home });
  const matchedRoot =
    configuration.roots.find((root) => pathIsInside(root, normalizedProject)) ??
    null;
  if (matchedRoot) {
    return {
      state: "inside-root",
      matchedRoot,
      projectPath: normalizedProject,
      roots: configuration.roots,
    };
  }
  if (!configuration.hasCompletedFirstRunSetup || configuration.roots.length === 0) {
    return {
      state: "setup-incomplete",
      projectPath: normalizedProject,
      roots: configuration.roots,
    };
  }
  return {
    state: "outside-roots",
    projectPath: normalizedProject,
    roots: configuration.roots,
  };
}
