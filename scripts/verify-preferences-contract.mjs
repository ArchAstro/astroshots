#!/usr/bin/env node
/**
 * Guard the Astroshots preferences read contract.
 *
 * Two silent-false-negative bugs this prevents:
 *   1. Reading the wrong preferences domain. The real domain is
 *      `ai.archastro.Astroshots`; the plausible wrong guess keeps the
 *      organization but swaps the `ai.` prefix, and `defaults read` on a
 *      missing domain looks like "not configured" instead of an error. Any
 *      tracked file naming a wrong-prefix organization domain fails here.
 *   2. Documenting the bundle identifier as an unchecked literal. The
 *      identifier is derived here from macos/project.yml and every other
 *      occurrence is asserted against it.
 *
 * It also keeps docs/PREFERENCES.md honest about the two-key watch-root
 * precedence that Preferences.swift actually implements.
 *
 * Escape hatch: a line containing the marker `domain-guard: allow` may hold a
 * wrong-prefix domain (docs need to name the wrong guess). Every allowance is
 * printed so it stays visible in review.
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ALLOW_MARKER = "domain-guard: allow";
const PROJECT_YML = "macos/project.yml";
const PREFERENCES_SWIFT = "macos/Astroshots/App/Preferences.swift";
const CONTRACT_DOC = "docs/PREFERENCES.md";
const BINARY_EXTENSIONS = new Set([
  ".png",
  ".jpg",
  ".jpeg",
  ".gif",
  ".pdf",
  ".mp4",
  ".mov",
  ".zip",
  ".icns",
  ".ico",
  ".woff",
  ".woff2",
  ".ttf",
  ".otf",
]);

const failures = [];
const allowances = [];

function fail(message) {
  failures.push(message);
}

function read(relativePath) {
  return fs.readFileSync(path.join(repoRoot, relativePath), "utf8");
}

/** Single source of truth: the app's own project definition. */
function resolveBundleIdentifier() {
  const projectYml = read(PROJECT_YML);
  const prefix = projectYml.match(/^\s*bundleIdPrefix:\s*(\S+)\s*$/m)?.[1];
  const identifier = projectYml.match(
    /^\s*PRODUCT_BUNDLE_IDENTIFIER:\s*(\S+)\s*$/m,
  )?.[1];

  if (!prefix) throw new Error(`${PROJECT_YML} does not declare bundleIdPrefix`);
  if (!identifier) {
    throw new Error(`${PROJECT_YML} does not declare PRODUCT_BUNDLE_IDENTIFIER`);
  }
  if (identifier !== `${prefix}.Astroshots`) {
    fail(
      `${PROJECT_YML}: PRODUCT_BUNDLE_IDENTIFIER ${identifier} does not match bundleIdPrefix ${prefix}`,
    );
  }

  const organizationPrefix = prefix.split(".").slice(0, -1).join(".");
  const organization = prefix.split(".").at(-1);
  if (!organization) {
    throw new Error(`${PROJECT_YML}: bundleIdPrefix ${prefix} has no organization component`);
  }
  return { prefix, identifier, organization, organizationPrefix };
}

function trackedFiles() {
  return execFileSync("git", ["ls-files", "-z"], {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  })
    .split("\0")
    .filter(Boolean);
}

/**
 * Every tracked mention of `<owner>.<organization>` must use the prefix that
 * macos/project.yml declares.
 */
function checkDomainPrefixes({ organization, organizationPrefix, prefix }) {
  const pattern = new RegExp(
    String.raw`\b([A-Za-z0-9_]+)\.${organization}(?:\.[A-Za-z0-9_]+)*\b`,
    "gi",
  );
  for (const relativePath of trackedFiles()) {
    if (BINARY_EXTENSIONS.has(path.extname(relativePath).toLowerCase())) continue;
    const absolute = path.join(repoRoot, relativePath);
    let text;
    try {
      text = fs.readFileSync(absolute, "utf8");
    } catch {
      continue;
    }
    if (text.includes("\0")) continue;

    text.split("\n").forEach((line, index) => {
      for (const match of line.matchAll(pattern)) {
        // Compare only the component immediately left of the organization, so
        // `-ai.archastro` inside a shell default keeps passing.
        const owner = match[1];
        if (owner.toLowerCase() === organizationPrefix.toLowerCase()) continue;
        const location = `${relativePath}:${index + 1}`;
        if (line.includes(ALLOW_MARKER)) {
          allowances.push(`${location}: ${match[0]}`);
          continue;
        }
        fail(
          `${location}: wrong preferences/bundle domain "${match[0]}" — the canonical prefix is "${prefix}" (see ${CONTRACT_DOC})`,
        );
      }
    });
  }
}

/** The identifier must be asserted against project.yml, never re-typed blind. */
function checkIdentifierUsages({ identifier }) {
  const generatedProject = "macos/Astroshots.xcodeproj/project.pbxproj";
  if (!read(generatedProject).includes(`PRODUCT_BUNDLE_IDENTIFIER = ${identifier};`)) {
    fail(
      `${generatedProject} does not carry ${identifier}; re-run xcodegen after editing ${PROJECT_YML}`,
    );
  }

  const doc = read(CONTRACT_DOC);
  if (!doc.includes(identifier)) {
    fail(`${CONTRACT_DOC} must state the canonical domain ${identifier}`);
  }
  if (!doc.includes(PROJECT_YML)) {
    fail(`${CONTRACT_DOC} must point at ${PROJECT_YML} as the identifier's source of truth`);
  }
}

/** Keep the documented read order tied to the implementation. */
function checkWatchRootContract() {
  const swift = read(PREFERENCES_SWIFT);
  for (const key of [
    "hasCompletedFirstRunSetup",
    "normalizeWatchRootPaths",
    'static let watchRoots = "watchRoots"',
    'static let watchRoot = "watchRoot"',
  ]) {
    if (!swift.includes(key)) {
      fail(`${PREFERENCES_SWIFT} no longer defines ${key}; ${CONTRACT_DOC} is now wrong`);
    }
  }

  // The getter is everything between `get {` and the matching `set {`.
  const getter = swift.match(
    /var watchRootPaths: \[String\] \{\s*\n\s*get \{([\s\S]*?)\n\s*set \{/,
  )?.[1];
  if (!getter) {
    fail(`${PREFERENCES_SWIFT}: could not locate the watchRootPaths getter`);
  } else {
    const plural = getter.indexOf("Key.watchRoots");
    const singular = getter.search(/Key\.watchRoot\b(?!s)/);
    if (plural < 0 || singular < 0) {
      fail(
        `${PREFERENCES_SWIFT}: watchRootPaths must read both Key.watchRoots and Key.watchRoot`,
      );
    } else if (plural > singular) {
      fail(
        `${PREFERENCES_SWIFT}: watchRootPaths must read Key.watchRoots before legacy Key.watchRoot`,
      );
    }
  }

  const doc = read(CONTRACT_DOC);
  for (const claim of [
    "watchRoots",
    "watchRoot",
    "hasCompletedFirstRunSetup",
    "normalizeWatchRootPaths",
  ]) {
    if (!doc.includes(claim)) fail(`${CONTRACT_DOC} must document ${claim}`);
  }
}

/** A tool author must be able to find the contract from the obvious entry points. */
function checkDiscoverability() {
  const linkers = [
    ["README.md", "docs/PREFERENCES.md"],
    ["macos/README.md", "../docs/PREFERENCES.md"],
    [PREFERENCES_SWIFT, "docs/PREFERENCES.md"],
  ];
  for (const [file, link] of linkers) {
    if (!read(file).includes(link)) {
      fail(`${file} must link the preferences contract (${link})`);
    }
  }
}

function main() {
  const bundle = resolveBundleIdentifier();
  checkDomainPrefixes(bundle);
  checkIdentifierUsages(bundle);
  checkWatchRootContract();
  checkDiscoverability();

  for (const allowance of allowances) {
    console.log(`verify-preferences-contract: allowed wrong-domain example at ${allowance}`);
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`verify-preferences-contract: ${failure}`);
    }
    process.exit(1);
  }

  console.log(
    `verify-preferences-contract: canonical domain ${bundle.identifier} (from ${PROJECT_YML}) and the watch-root read contract hold`,
  );
}

main();
