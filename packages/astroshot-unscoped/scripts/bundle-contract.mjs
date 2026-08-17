/**
 * The bundling contract for the unscoped `astroshot` wrapper.
 *
 * Shared by prepack (build-then-verify) and by the drift assertion that CI and
 * `npm test --workspace astroshot` run. See docs/UNSCOPED-CLI-DESIGN.md.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const wrapperRoot = path.dirname(
  path.dirname(fileURLToPath(import.meta.url)),
);
export const repoRoot = path.resolve(wrapperRoot, "..", "..");

/** Every @archastro package the wrapper bundles, plus the build output that
 * must exist before packing. `dist/` is gitignored and produced by `tsc`, and
 * npm pack bundles a dependency with a missing dist/ silently. */
export const BUNDLED_ENGINES = [
  {
    name: "@archastro/react-shot",
    directory: "packages/react-shot",
    builtFiles: ["dist/index.js", "dist/index.d.ts"],
  },
  {
    name: "@archastro/tui-shot",
    directory: "packages/tui-shot",
    builtFiles: ["dist/index.js", "dist/index.d.ts"],
  },
  {
    name: "@archastro/movie-harness",
    directory: "packages/movie-harness",
    builtFiles: ["dist/index.js", "dist/index.d.ts"],
  },
];

/** The unified CLI is bundled too, but it is plain .mjs with no build step. */
export const BUNDLED_CLI = {
  name: "@archastro/astroshot",
  directory: "packages/astroshot",
  builtFiles: ["bin/astroshot.mjs"],
};

export const BUNDLED_PACKAGES = [BUNDLED_CLI, ...BUNDLED_ENGINES];

export function readManifest(directory) {
  return JSON.parse(
    fs.readFileSync(path.join(repoRoot, directory, "package.json"), "utf8"),
  );
}

export function readWrapperManifest() {
  return JSON.parse(
    fs.readFileSync(path.join(wrapperRoot, "package.json"), "utf8"),
  );
}

function isScoped(name) {
  return name.startsWith("@archastro/");
}

/**
 * Minimal, dependency-free range check for the shapes this repo actually pins
 * (`1.2.3`, `^1.2.3`, `~1.2.3`, `>=1.2.3`). Returns true when the range cannot
 * be evaluated, so an unusual future range never produces a false failure — the
 * missing-dependency check above is the assertion that must never be skipped.
 */
function parseSimpleRange(range) {
  const match = /^(\^|~|>=|=)?\s*(\d+)\.(\d+)\.(\d+)$/.exec(String(range).trim());
  if (!match) return null;
  return {
    operator: match[1] ?? "=",
    version: [Number(match[2]), Number(match[3]), Number(match[4])],
  };
}

function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] < right[index] ? -1 : 1;
  }
  return 0;
}

function wrapperRangeCoversEngineRange(wrapperRange, engineRange) {
  const wrapperParsed = parseSimpleRange(wrapperRange);
  const engineParsed = parseSimpleRange(engineRange);
  if (!wrapperParsed || !engineParsed) return true; // unevaluable, do not fail
  const wrapperMinimum = wrapperParsed.version;
  const engineMinimum = engineParsed.version;
  if (compareVersions(wrapperMinimum, engineMinimum) < 0) return false;
  switch (engineParsed.operator) {
    case "^":
      return wrapperMinimum[0] === engineMinimum[0];
    case "~":
      return (
        wrapperMinimum[0] === engineMinimum[0] &&
        wrapperMinimum[1] === engineMinimum[1]
      );
    case ">=":
      return true;
    default:
      return compareVersions(wrapperMinimum, engineMinimum) === 0;
  }
}

/**
 * Assert the wrapper declares every UNSCOPED runtime dependency of every
 * bundled package.
 *
 * Why this matters: scoped packages are bundled into the tarball, so they are
 * never fetched at install time and a hijacked @archastro scope registry cannot
 * break the install. Their own unscoped dependencies, however, are NOT bundled —
 * npm resolves them from the default registry when the wrapper installs. They
 * are therefore only present if the wrapper declares them itself.
 *
 * Covers `dependencies` + `optionalDependencies`.
 *
 * peerDependencies are DELIBERATELY EXCLUDED: react-shot peers react/react-dom
 * and tui-shot peers ink/react. Those are unscoped (so unhijackable) and are
 * correctly peers because the consumer supplies the framework version their own
 * fixtures compile against. Absorbing them would silently pin a React version
 * for every consumer of `astroshot`.
 */
export function assertBundledDependencyUnion() {
  const wrapper = readWrapperManifest();
  const declared = new Map();
  for (const field of ["dependencies", "optionalDependencies"]) {
    for (const [name, range] of Object.entries(wrapper[field] ?? {})) {
      declared.set(name, { range, field });
    }
  }

  const failures = [];
  for (const bundled of BUNDLED_PACKAGES) {
    const manifest = readManifest(bundled.directory);
    for (const field of ["dependencies", "optionalDependencies"]) {
      for (const [name, range] of Object.entries(manifest[field] ?? {})) {
        if (isScoped(name)) continue; // bundled, never fetched at install time
        const declaredEntry = declared.get(name);
        if (!declaredEntry) {
          failures.push(
            `${bundled.name} requires unscoped ${field} "${name}": "${range}", ` +
              `but the unscoped astroshot wrapper does not declare "${name}". ` +
              `Add "${name}" to packages/astroshot-unscoped/package.json ` +
              `${field === "optionalDependencies" ? "optionalDependencies" : "dependencies"}.`,
          );
          continue;
        }
        if (
          field === "optionalDependencies" &&
          declaredEntry.field !== "optionalDependencies"
        ) {
          failures.push(
            `${bundled.name} declares "${name}" as an optionalDependency, but ` +
              `the unscoped astroshot wrapper declares it under ` +
              `"${declaredEntry.field}". Keep it optional so installs that ` +
              "cannot build the native addon still succeed.",
          );
        }
        if (!wrapperRangeCoversEngineRange(declaredEntry.range, range)) {
          failures.push(
            `${bundled.name} requires "${name}": "${range}", which the ` +
              `unscoped astroshot wrapper's "${name}": "${declaredEntry.range}" ` +
              "does not satisfy. Widen the wrapper's range in " +
              "packages/astroshot-unscoped/package.json.",
          );
        }
      }
    }

    // Peers must stay peers. Fail loudly if someone absorbs them, because that
    // would pin a framework version for every consumer.
    for (const name of Object.keys(manifest.peerDependencies ?? {})) {
      if (declared.has(name)) {
        failures.push(
          `${bundled.name} declares "${name}" as a peerDependency, but the ` +
            "unscoped astroshot wrapper declares it as its own dependency. " +
            "Peer dependencies must stay the consumer's choice; remove it " +
            "from packages/astroshot-unscoped/package.json.",
        );
      }
    }
  }

  if (failures.length) {
    throw new Error(
      "unscoped astroshot wrapper dependency drift:\n" +
        failures.map((failure) => `  - ${failure}`).join("\n"),
    );
  }
}

/** Every bundled package must be listed in bundleDependencies at the pinned
 * release version, or npm silently tries to FETCH it at install time — the
 * exact failure this package exists to prevent. */
export function assertBundleDeclaration() {
  const wrapper = readWrapperManifest();
  const bundle = new Set(wrapper.bundleDependencies ?? []);
  const failures = [];
  for (const bundled of BUNDLED_PACKAGES) {
    if (!bundle.has(bundled.name)) {
      failures.push(
        `${bundled.name} is not listed in the unscoped astroshot wrapper's ` +
          "bundleDependencies; npm would fetch it from the (hijackable) " +
          "@archastro scope registry at install time.",
      );
    }
    const range = wrapper.dependencies?.[bundled.name];
    const expected = readManifest(bundled.directory).version;
    if (range !== expected) {
      failures.push(
        `${bundled.name} is pinned to "${range}" by the unscoped astroshot ` +
          `wrapper but the workspace is at "${expected}".`,
      );
    }
  }
  if (wrapper.version !== readManifest(BUNDLED_CLI.directory).version) {
    failures.push(
      `astroshot ${wrapper.version} must share the release version of ` +
        `${BUNDLED_CLI.name} ${readManifest(BUNDLED_CLI.directory).version}.`,
    );
  }
  if (failures.length) {
    throw new Error(
      "unscoped astroshot wrapper bundle declaration is wrong:\n" +
        failures.map((failure) => `  - ${failure}`).join("\n"),
    );
  }
}
