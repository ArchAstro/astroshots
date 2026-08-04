#!/usr/bin/env node

/**
 * Promote CHANGELOG.md's "## [Unreleased]" notes into a dated release section
 * (Keep a Changelog). Same discipline as Scopey's prepare-release.sh.
 *
 * Usage:
 *   node scripts/roll-changelog.mjs <version> --track macos|npm [--date YYYY-MM-DD]
 *
 * Headers written as:
 *   ## [0.1.15] (macos) - 2026-08-04
 *   ## [0.1.1] (npm) - 2026-08-04
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

function usage(message) {
  if (message) console.error(message);
  console.error(
    "Usage: node scripts/roll-changelog.mjs <version> --track macos|npm [--date YYYY-MM-DD]",
  );
  process.exit(message ? 1 : 0);
}

const args = process.argv.slice(2);
let version = null;
let track = null;
let releaseDate = new Date().toISOString().slice(0, 10);

for (let i = 0; i < args.length; i += 1) {
  const arg = args[i];
  if (arg === "--track") {
    track = args[++i];
  } else if (arg === "--date") {
    releaseDate = args[++i];
  } else if (arg === "--help" || arg === "-h") {
    usage();
  } else if (!arg.startsWith("-") && version === null) {
    version = arg.startsWith("v") ? arg.slice(1) : arg;
  } else {
    usage(`Unknown argument: ${arg}`);
  }
}

if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  usage("A valid semver version is required.");
}
if (track !== "macos" && track !== "npm") {
  usage("--track must be macos or npm.");
}
if (!/^\d{4}-\d{2}-\d{2}$/.test(releaseDate)) {
  usage("--date must be YYYY-MM-DD.");
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const changelogPath = path.join(root, "CHANGELOG.md");
if (!fs.existsSync(changelogPath)) {
  console.error(`Missing ${changelogPath}`);
  process.exit(1);
}

const text = fs.readFileSync(changelogPath, "utf8");
const unreleasedHeader = "## [Unreleased]";
if (!text.includes(unreleasedHeader)) {
  console.error("CHANGELOG.md has no ## [Unreleased] section.");
  process.exit(1);
}

const releaseHeader = `## [${version}] (${track}) - ${releaseDate}`;
if (text.includes(`## [${version}] (${track})`)) {
  console.error(`CHANGELOG.md already has a ${track} section for ${version}.`);
  process.exit(1);
}

// Insert a fresh empty Unreleased section and convert the previous Unreleased
// block header into the dated release header (content stays in place).
const next = text.replace(
  unreleasedHeader,
  `${unreleasedHeader}\n\n${releaseHeader}`,
);

if (next === text) {
  console.error("Failed to roll CHANGELOG.md Unreleased section.");
  process.exit(1);
}

fs.writeFileSync(changelogPath, next);
console.log(`Rolled CHANGELOG.md: ${releaseHeader}`);
