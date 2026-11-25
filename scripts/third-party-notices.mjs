#!/usr/bin/env node
/**
 * Generate the third-party license notices that ship with every release
 * artifact.
 *
 *   node scripts/third-party-notices.mjs            regenerate committed files
 *   node scripts/third-party-notices.mjs --check    fail if any committed file drifted
 *   node scripts/third-party-notices.mjs installed <node_modules> <out> <title>
 *   node scripts/third-party-notices.mjs swift <Package.resolved> <checkouts> <out>
 *
 * Committed outputs (regenerated and drift-checked):
 *   - packages/<name>/THIRD_PARTY_NOTICES.md: production dependency tree of
 *     each published npm package, resolved from package-lock.json.
 *   - rust/THIRD_PARTY_NOTICES.md: Rust crates (cargo-about, rust/about.toml),
 *     the fonts compiled into astroshot-engine, and the npm packages of the
 *     Node helper embedded in it (rust/node-helper/package-lock.json).
 *
 * Build-time outputs (the macOS app resolves these trees while it builds):
 *   - `installed`: an installed node_modules tree (the app's Tools payload).
 *   - `swift`: Swift packages resolved by Xcode (the app binary).
 *
 * License texts are read from the installed package directories, so run
 * `npm ci` (root) and `npm ci --ignore-scripts` (rust/node-helper) first.
 * Platform-specific optional packages (lockfile `os`/`cpu`) are listed but
 * their files are never read, so the output is the same on every OS.
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** cargo-about output differs between releases; CI installs this version. */
export const CARGO_ABOUT_VERSION = "0.9.3";

/** Published npm packages and the workspace directory of each. */
const NPM_PACKAGES = [
  "packages/react-shot",
  "packages/tui-shot",
  "packages/movie-harness",
  "packages/astroshot-review",
  "packages/astroshot",
  "packages/astroshot-unscoped",
];

const LICENSE_FILE = /^(licen[cs]e|copying|notice)([-._].*)?$/i;

function isFirstParty(name) {
  return name === "astroshot" || name.startsWith("@archastro/");
}

function repositoryUrl(manifest) {
  const repo = manifest.repository;
  const raw = typeof repo === "string" ? repo : repo?.url;
  if (!raw) return manifest.homepage ?? "";
  return raw
    .replace(/^git\+/, "")
    .replace(/^git:\/\//, "https://")
    .replace(/^ssh:\/\/git@/, "https://")
    .replace(/^git@github\.com:/, "https://github.com/")
    .replace(/^github:/, "https://github.com/")
    .replace(/^([\w.-]+\/[\w.-]+)$/, "https://github.com/$1")
    .replace(/\.git$/, "");
}

function licenseString(license) {
  if (!license) return "UNKNOWN";
  if (typeof license === "string") return license;
  return license.type ?? "UNKNOWN";
}

function readLicenseFiles(directory) {
  if (!fs.existsSync(directory)) return null;
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && LICENSE_FILE.test(entry.name))
    .map((entry) => entry.name)
    .sort()
    .map((name) => ({
      file: name,
      text: fs
        .readFileSync(path.join(directory, name), "utf8")
        .replace(/\r\n?/g, "\n")
        .trim(),
    }))
    .filter((entry) => entry.text.length > 0);
}

/** Resolve `name` from lockfile path `from` the way Node walks node_modules. */
function resolveLock(packages, from, name) {
  let base = from;
  for (;;) {
    const candidate = `${base ? `${base}/` : ""}node_modules/${name}`;
    if (packages[candidate]) return candidate;
    if (!base) return null;
    const index = base.lastIndexOf("node_modules/");
    base = index < 0 ? "" : base.slice(0, index).replace(/\/$/, "");
  }
}

/**
 * Production closure of `rootPath` in a package-lock.json: dependencies and
 * optionalDependencies, transitively. Workspace links are followed so a
 * bundled first-party package contributes its own dependencies.
 */
function lockfileEntries(lockfilePath, rootPath) {
  const lockDir = path.dirname(lockfilePath);
  const { packages } = JSON.parse(fs.readFileSync(lockfilePath, "utf8"));
  const seen = new Set();
  const stack = [rootPath];
  const entries = [];
  while (stack.length) {
    let key = stack.pop();
    let entry = packages[key];
    if (entry?.link) {
      key = entry.resolved;
      entry = packages[key];
    }
    if (!entry || seen.has(key)) continue;
    seen.add(key);
    const dependencies = {
      ...entry.dependencies,
      ...entry.optionalDependencies,
    };
    for (const name of Object.keys(dependencies)) {
      const resolved = resolveLock(packages, key, name);
      if (resolved) stack.push(resolved);
      else if (!entry.optionalDependencies?.[name]) {
        throw new Error(`${lockfilePath}: ${key} depends on ${name}, which the lockfile does not contain`);
      }
    }
    if (key === rootPath || !key.includes("node_modules/")) continue;
    const name = entry.name ?? key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
    if (isFirstParty(name)) continue;
    const platform = Boolean(entry.os || entry.cpu || entry.libc);
    const directory = path.join(lockDir, key);
    let manifest = {};
    let texts = [];
    if (!platform) {
      const manifestPath = path.join(directory, "package.json");
      if (!fs.existsSync(manifestPath)) {
        throw new Error(
          `${name}@${entry.version} is not installed at ${path.relative(repoRoot, directory)}; ` +
            `run npm ci in ${path.relative(repoRoot, lockDir) || "."} first`,
        );
      }
      manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
      texts = readLicenseFiles(directory);
    }
    entries.push({
      name,
      version: entry.version,
      license: licenseString(entry.license ?? manifest.license),
      repository: repositoryUrl(manifest),
      platform,
      texts,
    });
  }
  return dedupe(entries);
}

/** Every package in an installed node_modules tree (nested trees included). */
function installedEntries(nodeModules) {
  const entries = [];
  const visit = (directory) => {
    if (!fs.existsSync(directory)) return;
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
      const child = path.join(directory, entry.name);
      if (entry.name.startsWith("@")) {
        visit(child);
        continue;
      }
      const manifestPath = path.join(child, "package.json");
      if (fs.existsSync(manifestPath)) {
        const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
        if (manifest.name && !isFirstParty(manifest.name)) {
          entries.push({
            name: manifest.name,
            version: manifest.version,
            license: licenseString(manifest.license),
            repository: repositoryUrl(manifest),
            platform: false,
            texts: readLicenseFiles(child),
          });
        }
      }
      visit(path.join(child, "node_modules"));
    }
  };
  visit(nodeModules);
  return dedupe(entries);
}

function dedupe(entries) {
  const byKey = new Map();
  for (const entry of entries) byKey.set(`${entry.name}@${entry.version}`, entry);
  return [...byKey.values()].sort(
    (a, b) => compare(a.name, b.name) || compare(a.version, b.version),
  );
}

function compare(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

function fence(text) {
  const longest = Math.max(2, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  return "`".repeat(longest + 1);
}

function cell(value) {
  return String(value || "-").replace(/\|/g, "\\|").replace(/\n/g, " ");
}

/** Markdown: a package table followed by each distinct license text once. */
function renderSection({ heading, intro, kind, entries }) {
  const lines = [`## ${heading}`, "", ...intro, ""];
  if (entries.length === 0) {
    lines.push(`This artifact bundles no third-party ${kind}s.`, "");
    return lines.join("\n");
  }
  lines.push(
    `| ${kind[0].toUpperCase()}${kind.slice(1)} | Version | License | Repository |`,
    "| --- | --- | --- | --- |",
  );
  for (const entry of entries) {
    lines.push(
      `| ${cell(entry.name)} | ${cell(entry.version)} | ${cell(entry.license)} | ${cell(entry.repository)} |`,
    );
  }
  lines.push("");

  const groups = new Map();
  const missing = [];
  const platform = [];
  for (const entry of entries) {
    if (entry.platform) {
      platform.push(entry);
      continue;
    }
    if (!entry.texts?.length) {
      missing.push(entry);
      continue;
    }
    const text = entry.texts.map((t) => t.text).join("\n\n");
    const group = groups.get(text) ?? { license: entry.license, users: [] };
    group.users.push(entry);
    groups.set(text, group);
  }
  lines.push(`### License texts`, "");
  for (const [text, group] of groups) {
    const licenses = [...new Set(group.users.map((u) => u.license))].join(", ");
    lines.push(`#### ${licenses}`, "", "Used by:", "");
    for (const user of group.users) lines.push(`- ${user.name} ${user.version}`);
    const marker = fence(text);
    lines.push("", `${marker}text`, text, marker, "");
  }
  if (platform.length) {
    lines.push(
      "### Platform-specific packages",
      "",
      "npm installs only the package matching the host OS and CPU. Each is a",
      "prebuilt binary of the project named in its scope and is distributed",
      "under the declared license in the table above; any license file it ships",
      "is inside the installed package.",
      "",
    );
    for (const entry of platform) lines.push(`- ${entry.name} ${entry.version} (${entry.license})`);
    lines.push("");
  }
  if (missing.length) {
    lines.push(
      "### Packages without a license file",
      "",
      "These packages ship no license file; they are distributed under the",
      "license declared in their package metadata, shown in the table above.",
      "",
    );
    for (const entry of missing) lines.push(`- ${entry.name} ${entry.version} (${entry.license})`);
    lines.push("");
  }
  return lines.join("\n");
}

function header(title, lines) {
  return [`# ${title}`, "", ...lines, ""].join("\n");
}

const GENERATED = [
  "Generated by `scripts/third-party-notices.mjs`. Do not edit by hand; run",
  "`npm run notices` after changing dependencies.",
];

function npmPackageNotices(directory) {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(repoRoot, directory, "package.json"), "utf8"),
  );
  const entries = lockfileEntries(path.join(repoRoot, "package-lock.json"), directory);
  return [
    header(`Third-party notices: ${manifest.name}`, [
      `${manifest.name} is MIT licensed (see \`LICENSE\` next to this file).`,
      "Installing it also installs the third-party npm packages below. Each is",
      "listed with its declared license, followed by the license texts the",
      "packages ship.",
      "",
      ...GENERATED,
    ]),
    renderSection({
      heading: "npm packages",
      intro: ["Production dependencies resolved from `package-lock.json`."],
      kind: "package",
      entries,
    }),
  ].join("\n");
}

function cargoAbout() {
  let version;
  try {
    version = execFileSync("cargo", ["about", "--version"], { encoding: "utf8" }).trim();
  } catch {
    throw new Error(
      `cargo-about is required: cargo install --locked cargo-about --version ${CARGO_ABOUT_VERSION} --features cli`,
    );
  }
  if (version !== `cargo-about ${CARGO_ABOUT_VERSION}`) {
    throw new Error(`expected cargo-about ${CARGO_ABOUT_VERSION}, found ${version}`);
  }
  return execFileSync(
    "cargo",
    ["about", "generate", "--locked", "--fail", "about.hbs"],
    { cwd: path.join(repoRoot, "rust"), encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  ).trimEnd();
}

function rustNotices() {
  const fontDirectory = path.join(repoRoot, "rust", "astroshot-engine", "assets", "fonts");
  const fonts = fs
    .readdirSync(fontDirectory)
    .filter((name) => name.endsWith(".ttf"))
    .sort();
  const ofl = fs.readFileSync(path.join(fontDirectory, "OFL.txt"), "utf8").replace(/\r\n?/g, "\n").trim();
  const helper = lockfileEntries(path.join(repoRoot, "rust", "node-helper", "package-lock.json"), "");
  return [
    cargoAbout(),
    "",
    "## Fonts",
    "",
    "`astroshot-engine` compiles these fonts into the binary for terminal",
    "rendering (`rust/astroshot-engine/assets/fonts`):",
    "",
    ...fonts.map((font) => `- ${font} (JetBrains Mono, OFL-1.1)`),
    "",
    `${fence(ofl)}text`,
    ofl,
    fence(ofl),
    "",
    renderSection({
      heading: "Embedded Node helper packages",
      intro: [
        "`astroshot-engine` embeds a Node helper distribution built from",
        "`rust/node-helper/package-lock.json`. The archive also carries each",
        "package's own license files.",
      ],
      kind: "package",
      entries: helper,
    }),
  ].join("\n");
}

/**
 * License files of a source checkout: its own, plus those of code it vendors
 * in subdirectories (e.g. C++ libraries compiled into the package). Tests,
 * examples, docs and build output are skipped.
 */
function vendoredLicenseFiles(root, depth = 4) {
  const skip = /^(\.|tests?$|examples?$|docs?$|benchmarks?$|build$)/i;
  const found = [];
  const visit = (directory, level) => {
    for (const entry of readLicenseFiles(directory) ?? []) {
      const relative = path.relative(root, path.join(directory, entry.file));
      found.push({ file: relative, text: relative === entry.file ? entry.text : `${relative}:\n\n${entry.text}` });
    }
    if (level >= depth) return;
    for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((a, b) => compare(a.name, b.name))) {
      if (entry.isDirectory() && !skip.test(entry.name)) visit(path.join(directory, entry.name), level + 1);
    }
  };
  visit(root, 0);
  return found;
}

/** Swift package checkouts as resolved by Xcode for the app build. */
function swiftEntries(resolvedPath, checkouts) {
  const resolved = JSON.parse(fs.readFileSync(resolvedPath, "utf8"));
  const pins = resolved.pins ?? resolved.object?.pins ?? [];
  if (pins.length === 0) throw new Error(`${resolvedPath} has no package pins`);
  return dedupe(
    pins.map((pin) => {
      const location = (pin.location ?? pin.repositoryURL).replace(/\.git$/, "");
      const identity = pin.identity ?? path.basename(location).toLowerCase();
      const candidates = fs.existsSync(checkouts) ? fs.readdirSync(checkouts) : [];
      const checkout =
        candidates.find((name) => name.toLowerCase() === identity) ??
        candidates.find((name) => name.toLowerCase() === path.basename(location).toLowerCase());
      if (!checkout) throw new Error(`no checkout for Swift package ${identity} in ${checkouts}`);
      const texts = vendoredLicenseFiles(path.join(checkouts, checkout));
      if (!texts?.length) throw new Error(`Swift package ${identity} ships no license file`);
      const state = pin.state ?? {};
      return {
        name: identity,
        version: state.version ?? state.revision?.slice(0, 12) ?? "",
        license: detectLicense(texts.map((t) => t.text).join("\n")),
        repository: location,
        platform: false,
        texts,
      };
    }),
  );
}

function detectLicense(text) {
  const found = [];
  if (/Apache License\s+Version 2\.0/i.test(text)) found.push("Apache-2.0");
  if (/Permission is hereby granted, free of charge/i.test(text)) found.push("MIT");
  if (/Redistribution and use in source and binary forms/i.test(text)) found.push("BSD");
  if (/GNU GENERAL PUBLIC LICENSE\s+Version 2/i.test(text)) found.push("GPL-2.0");
  return found.length ? found.join(", ") : "see license text";
}

function swiftNotices(resolvedPath, checkouts) {
  return [
    header("Third-party notices: Astroshots for macOS", [
      "Astroshots is MIT licensed (see `LICENSE` next to this file). The app",
      "links the Swift packages below. The bundled command line tools in",
      "`Tools/` list their own npm packages in `Tools/THIRD_PARTY_NOTICES.md`,",
      "and the bundled Node.js runtime's license is `Tools/node/LICENSE`.",
      "",
      "Generated at build time by `scripts/third-party-notices.mjs` from the",
      "Swift packages Xcode resolved for this build.",
    ]),
    renderSection({
      heading: "Swift packages",
      intro: [
        "The license column lists the licenses detected in each package's own",
        "and vendored license files. zstd is dual licensed (BSD or GPL-2.0) and",
        "is used under its BSD license.",
      ],
      kind: "package",
      entries: swiftEntries(resolvedPath, checkouts),
    }),
  ].join("\n");
}

function installedNotices(nodeModules, title) {
  return [
    header(`Third-party notices: ${title}`, [
      "These tools are MIT licensed (see `LICENSE` next to this file). They run",
      "on the bundled Node.js runtime (license: `node/LICENSE`) and include the",
      "third-party npm packages below.",
      "",
      "Generated at build time by `scripts/third-party-notices.mjs` from the",
      "installed package tree.",
    ]),
    renderSection({
      heading: "npm packages",
      intro: ["Every package installed in the bundled tree."],
      kind: "package",
      entries: installedEntries(nodeModules),
    }),
  ].join("\n");
}

function committedOutputs() {
  const outputs = NPM_PACKAGES.map((directory) => [
    path.join(directory, "THIRD_PARTY_NOTICES.md"),
    () => npmPackageNotices(directory),
  ]);
  outputs.push(["rust/THIRD_PARTY_NOTICES.md", rustNotices]);
  return outputs;
}

function finish(text) {
  return `${text.trimEnd()}\n`;
}

function main(argv) {
  const [command, ...rest] = argv;
  if (command === "installed") {
    const [nodeModules, out, title] = rest;
    if (!nodeModules || !out || !title) throw new Error("usage: installed <node_modules> <out> <title>");
    fs.writeFileSync(out, finish(installedNotices(nodeModules, title)));
    return 0;
  }
  if (command === "swift") {
    const [resolvedPath, checkouts, out] = rest;
    if (!resolvedPath || !checkouts || !out) throw new Error("usage: swift <Package.resolved> <checkouts> <out>");
    fs.writeFileSync(out, finish(swiftNotices(resolvedPath, checkouts)));
    return 0;
  }
  const check = command === "--check";
  if (command && !check) throw new Error(`unknown argument: ${command}`);
  const drifted = [];
  for (const [relative, generate] of committedOutputs()) {
    const target = path.join(repoRoot, relative);
    const text = finish(generate());
    const current = fs.existsSync(target) ? fs.readFileSync(target, "utf8") : null;
    if (current === text) continue;
    if (check) drifted.push(relative);
    else {
      fs.writeFileSync(target, text);
      console.log(`third-party-notices: wrote ${relative}`);
    }
  }
  if (drifted.length) {
    console.error(
      `third-party-notices: out of date:\n${drifted.map((f) => `  ${f}`).join("\n")}\n` +
        "Run `npm run notices` and commit the result.",
    );
    return 1;
  }
  if (check) console.log("third-party-notices: all notices are current");
  return 0;
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url))) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    console.error(`third-party-notices: ${error.message}`);
    process.exitCode = 1;
  }
}
