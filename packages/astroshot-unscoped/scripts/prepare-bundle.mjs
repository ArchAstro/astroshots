#!/usr/bin/env node
/**
 * `prepack` guard for the unscoped `astroshot` wrapper.
 *
 * The wrapper ships @archastro/astroshot and its three engines through
 * bundleDependencies. Each engine declares `files: [..., "dist"]` with a `tsc`
 * build step, and `dist/` is gitignored. npm pack bundles a dependency whose
 * dist/ is absent with NO error and NO warning, producing a tarball that
 * explodes at runtime, and because the engine is bundled, nothing ever
 * re-fetches a good copy.
 *
 * Therefore this script, run from `prepack`:
 *   1. asserts the bundle declaration and dependency union are correct,
 *   2. builds every engine,
 *   3. materializes packages/astroshot-unscoped/node_modules/@archastro/* (npm
 *      hoists workspace links to the repo root, and `npm pack` only bundles
 *      dependencies found in the package's OWN node_modules),
 *   4. verifies each bundled package's built output exists before packing.
 *
 * scripts/verify-packages.mjs re-asserts (4) against the packed tarball itself,
 * which is the only proof that survives packing.
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import {
  BUNDLED_ENGINES,
  BUNDLED_PACKAGES,
  assertBundleDeclaration,
  assertBundledDependencyUnion,
  repoRoot,
  wrapperRoot,
} from "./bundle-contract.mjs";

assertBundleDeclaration();
assertBundledDependencyUnion();

for (const engine of BUNDLED_ENGINES) {
  console.log(`prepare-bundle: building ${engine.name}`);
  execFileSync("npm", ["run", "build", "--workspace", engine.name], {
    cwd: repoRoot,
    stdio: "inherit",
  });
}

// npm pack only bundles dependencies present in THIS package's node_modules,
// but npm hoists workspace links to the repo root. Recreate the local farm as
// symlinks into packages/; `npm pack` dereferences them, so real files (not
// broken links) land in the tarball.
const bundleRoot = path.join(wrapperRoot, "node_modules", "@archastro");
fs.rmSync(bundleRoot, { recursive: true, force: true });
fs.mkdirSync(bundleRoot, { recursive: true });
for (const bundled of BUNDLED_PACKAGES) {
  const target = path.join(repoRoot, bundled.directory);
  const link = path.join(bundleRoot, path.basename(bundled.name));
  fs.symlinkSync(path.relative(path.dirname(link), target), link, "dir");
}

// Only files that exist right now get bundled, so assert the built output is
// there before npm packs it.
for (const bundled of BUNDLED_PACKAGES) {
  for (const required of bundled.builtFiles) {
    const bundledPath = path.join(bundleRoot, path.basename(bundled.name), required);
    if (!fs.existsSync(bundledPath)) {
      throw new Error(
        `prepare-bundle: ${bundled.name} is missing ${required}; packing now ` +
          "would silently ship an unbuilt engine",
      );
    }
  }
}

console.log(
  `prepare-bundle: ${BUNDLED_PACKAGES.length} @archastro packages built and ` +
    "linked for bundling",
);
