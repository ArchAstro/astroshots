#!/usr/bin/env node
/**
 * Unscoped `astroshot` entry point.
 *
 * This package exists so `npx astroshot ...` works with no registry flags. The
 * @archastro packages ship *inside* this tarball through bundleDependencies, so
 * a developer whose ~/.npmrc maps @archastro to a private registry can still
 * install and run the CLI. See docs/UNSCOPED-CLI-DESIGN.md.
 *
 * The only job of this file is to hand every argument to the real CLI and
 * faithfully return its stdout, stderr, exit code, and terminating signal.
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const packageRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const CLI_RELATIVE_PATH = path.join(
  "@archastro",
  "astroshot",
  "bin",
  "astroshot.mjs",
);

/**
 * Look for the bundled CLI next to this package first (the published layout
 * puts it in ./node_modules), then walk up through parent node_modules
 * directories so the workspace checkout resolves its symlinked sibling.
 */
function resolveCli() {
  const searched = [];
  let directory = packageRoot;
  for (;;) {
    const candidate = path.join(directory, "node_modules", CLI_RELATIVE_PATH);
    searched.push(candidate);
    if (fs.existsSync(candidate)) return candidate;
    const parent = path.dirname(directory);
    if (parent === directory) break;
    directory = parent;
  }
  return { searched };
}

const cli = resolveCli();
if (typeof cli !== "string") {
  console.error(
    "astroshot: the bundled @archastro/astroshot CLI is missing from this " +
      "installation. Reinstall with `npm install astroshot`.\nLooked in:\n" +
      cli.searched.map((entry) => `  ${entry}`).join("\n"),
  );
  process.exit(1);
}

// stdio: "inherit" hands the real file descriptors to the CLI, so stdout and
// stderr stay unbuffered, interleaved, and TTY-aware exactly as if the user had
// run @archastro/astroshot directly.
const result = spawnSync(process.execPath, [cli, ...process.argv.slice(2)], {
  stdio: "inherit",
});

if (result.error) {
  console.error(`astroshot could not start the CLI: ${result.error.message}`);
  process.exit(1);
}
// Re-raise the child's signal so `kill`/Ctrl-C semantics survive the hop.
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
