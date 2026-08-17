#!/usr/bin/env node
/**
 * Pack the unified public CLI and its engines (react, tui, movie-harness),
 * install their tarballs into a clean consumer, and exercise the executable.
 *
 * Also packs the UNSCOPED `astroshot` wrapper and proves, under a hostile
 * @archastro scope registry, that a plain `npm install astroshot` still works.
 * That is the property this repository ships the wrapper for; see
 * docs/UNSCOPED-CLI-DESIGN.md.
 */
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
  BUNDLED_PACKAGES,
  assertBundleDeclaration,
  assertBundledDependencyUnion,
} from "../packages/astroshot-unscoped/scripts/bundle-contract.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "astroshots-pack-"));
const archiveDir = path.join(tempRoot, "archives");
const consumerDir = path.join(tempRoot, "consumer");
const hijackDir = path.join(tempRoot, "hijacked-consumer");
const astroshotManifest = JSON.parse(
  fs.readFileSync(
    path.join(repoRoot, "packages", "astroshot", "package.json"),
    "utf8",
  ),
);
const packageVersion = astroshotManifest.version;

for (const engineName of [
  "@archastro/react-shot",
  "@archastro/tui-shot",
  "@archastro/movie-harness",
]) {
  if (astroshotManifest.dependencies[engineName] !== packageVersion) {
    throw new Error(
      `@archastro/astroshot ${packageVersion} must depend on ${engineName} ${packageVersion}`,
    );
  }
}

// The unscoped wrapper must bundle every @archastro package at the release
// version and declare every unscoped runtime dependency those packages need.
assertBundleDeclaration();
assertBundledDependencyUnion();

fs.mkdirSync(archiveDir, { recursive: true });
fs.mkdirSync(consumerDir, { recursive: true });
fs.mkdirSync(hijackDir, { recursive: true });

function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: options.cwd ?? repoRoot,
    encoding: options.encoding,
    env: { ...process.env, ...options.env },
    stdio: options.encoding ? ["ignore", "pipe", "inherit"] : "inherit",
  });
}

function pack(packageDirectory) {
  const before = new Set(fs.readdirSync(archiveDir));
  run("npm", [
    "pack",
    path.join(repoRoot, packageDirectory),
    "--pack-destination",
    archiveDir,
  ]);
  const created = fs
    .readdirSync(archiveDir)
    .filter((file) => file.endsWith(".tgz") && !before.has(file));
  if (created.length !== 1) {
    throw new Error(
      `expected one tarball for ${packageDirectory}, found ${created.length}`,
    );
  }
  return path.join(archiveDir, created[0]);
}

/**
 * Prove the unscoped `astroshot` package installs and runs on a machine whose
 * npm config points the @archastro scope at an unreachable registry.
 *
 * This is the exact developer situation that made `npx @archastro/astroshot`
 * fail with E404: ~/.npmrc says
 *   @archastro:registry=https://npm.pkg.github.com
 * so npm never asks npmjs for the scoped package at all.
 *
 * The proof must be hostile, not merely clean:
 *   - a project .npmrc points @archastro at http://127.0.0.1:9/ (a port that
 *     refuses connections), so any attempt to FETCH a scoped package fails
 *     loudly instead of silently succeeding on a well-configured machine;
 *   - the ambient user config is neutralized with --userconfig pointing at an
 *     empty file, so this cannot pass just because the shell happens to be
 *     configured correctly;
 *   - the default registry stays npmjs so the wrapper's own unscoped runtime
 *     dependencies still resolve.
 *
 * If the wrapper ever regresses to fetching @archastro packages at install time
 * (a plain dependency instead of bundleDependencies), this install fails with
 * ECONNREFUSED against 127.0.0.1:9 and names the scoped package.
 */
function verifyUnscopedWrapper(unscopedTarball) {
  // 1. The tarball itself must contain the built output of every bundled
  //    @archastro package. npm pack bundles a dependency whose dist/ is missing
  //    with no error and no warning, and because it is bundled nothing ever
  //    re-fetches a good copy, so this assertion is the only thing between a
  //    forgotten build and a CLI that explodes on the user's first run.
  const listing = run("tar", ["tzf", unscopedTarball], { encoding: "utf8" })
    .split("\n")
    .map((line) => line.replace(/^package\//, "").replace(/\/$/, ""))
    .filter(Boolean);
  const packed = new Set(listing);
  for (const bundled of BUNDLED_PACKAGES) {
    for (const requiredPath of bundled.builtFiles) {
      const entry = `node_modules/${bundled.name}/${requiredPath}`;
      if (!packed.has(entry)) {
        throw new Error(
          `the unscoped astroshot tarball is missing ${entry}. ` +
            `${bundled.name} was bundled without its build output, so the ` +
            "published CLI would fail at runtime with no way to recover.",
        );
      }
    }
  }
  // Nothing scoped may be left as a bare dependency to fetch.
  const packedManifest = JSON.parse(
    run("tar", ["xzfO", unscopedTarball, "package/package.json"], {
      encoding: "utf8",
    }),
  );
  const bundleList = new Set(packedManifest.bundleDependencies ?? []);
  for (const name of Object.keys(packedManifest.dependencies ?? {})) {
    if (name.startsWith("@archastro/") && !bundleList.has(name)) {
      throw new Error(
        `the unscoped astroshot tarball depends on ${name} without bundling ` +
          "it; a hijacked @archastro scope registry would break the install",
      );
    }
  }

  // 2. Install it with the @archastro scope pointed at a dead registry.
  const userConfig = path.join(hijackDir, "neutralized-user.npmrc");
  fs.writeFileSync(userConfig, "");
  fs.writeFileSync(
    path.join(hijackDir, ".npmrc"),
    // 127.0.0.1:9 is the discard port: connections are refused immediately.
    "@archastro:registry=http://127.0.0.1:9/\n" +
      "registry=https://registry.npmjs.org/\n",
  );
  fs.writeFileSync(
    path.join(hijackDir, "package.json"),
    JSON.stringify(
      {
        name: "astroshot-hijacked-scope-consumer",
        version: "1.0.0",
        private: true,
        // Ink fixtures are ESM; the engines resolve them relative to this root.
        type: "module",
      },
      null,
      2,
    ) + "\n",
  );

  const install = spawnSync(
    "npm",
    [
      "install",
      "--no-audit",
      "--no-fund",
      "--userconfig",
      userConfig,
      unscopedTarball,
    ],
    { cwd: hijackDir, encoding: "utf8", env: process.env },
  );
  if (install.status !== 0) {
    throw new Error(
      "installing the unscoped astroshot package failed while the @archastro " +
        "scope pointed at an unreachable registry. The wrapper must bundle " +
        "every @archastro package instead of fetching it at install time.\n" +
        `${install.stdout}\n${install.stderr}`,
    );
  }

  // 3. Run the plain unscoped entry point the README now documents.
  const executable = path.join(hijackDir, "node_modules", ".bin", "astroshot");
  if (!fs.existsSync(executable)) {
    throw new Error("the unscoped astroshot package did not install its bin");
  }
  const version = spawnSync(executable, ["--version"], {
    cwd: hijackDir,
    encoding: "utf8",
    env: process.env,
  });
  if (version.status !== 0) {
    throw new Error(
      `unscoped astroshot --version failed: ${version.stdout}\n${version.stderr}`,
    );
  }
  if (version.stdout.trim() !== packageVersion) {
    throw new Error(
      `unscoped astroshot reported ${version.stdout.trim()}, expected ${packageVersion}`,
    );
  }
  for (const modeArguments of [
    ["--help"],
    ["react", "--help"],
    ["ink", "--help"],
    ["pty", "--help"],
    ["movie", "--help"],
    ["movie", "which-source", "ratatui tui"],
  ]) {
    const result = spawnSync(executable, modeArguments, {
      cwd: hijackDir,
      encoding: "utf8",
      env: process.env,
    });
    if (result.status !== 0) {
      throw new Error(
        `unscoped astroshot ${modeArguments.join(" ")} failed under a ` +
          `hijacked @archastro scope: ${result.stdout}\n${result.stderr}`,
      );
    }
  }

  // 4. The engines really are on disk inside the install, not just resolvable
  //    because the ambient machine already had them.
  for (const bundled of BUNDLED_PACKAGES) {
    for (const requiredPath of bundled.builtFiles) {
      const installed = path.join(
        hijackDir,
        "node_modules",
        "astroshot",
        "node_modules",
        bundled.name,
        requiredPath,
      );
      if (!fs.existsSync(installed)) {
        throw new Error(
          `the installed unscoped astroshot package is missing ${bundled.name}/${requiredPath}`,
        );
      }
    }
  }

  // 5. In the CI capture job, take a real screenshot through the bundled
  //    engines. Help output alone would not catch a bundled engine whose
  //    unscoped runtime dependencies (playwright, vite, tsx, yaml, …) the
  //    wrapper failed to declare, because those are only loaded on capture.
  if (process.env.ASTROSHOTS_VERIFY_PACKAGES_CAPTURE === "1") {
    const peers = spawnSync(
      "npm",
      [
        "install",
        "--no-audit",
        "--no-fund",
        "--userconfig",
        userConfig,
        "react@19",
        "react-dom@19",
        "ink@7",
      ],
      { cwd: hijackDir, encoding: "utf8", env: process.env },
    );
    if (peers.status !== 0) {
      throw new Error(
        `installing peers into the hijacked consumer failed: ${peers.stdout}\n${peers.stderr}`,
      );
    }

    for (const [mode, fixture, output] of [
      ["react", "react-fixture.tsx", "react-proof.png"],
      ["ink", "ink-fixture.tsx", "ink-proof.png"],
    ]) {
      const init = spawnSync(executable, ["init", mode, fixture], {
        cwd: hijackDir,
        encoding: "utf8",
        env: process.env,
      });
      if (init.status !== 0) {
        throw new Error(
          `unscoped astroshot init ${mode} failed: ${init.stdout}\n${init.stderr}`,
        );
      }
      const capture = spawnSync(
        executable,
        [mode, `./${fixture}`, "-o", `./${output}`],
        { cwd: hijackDir, encoding: "utf8", env: process.env },
      );
      if (capture.status !== 0) {
        throw new Error(
          `unscoped astroshot ${mode} capture failed under a hijacked ` +
            `@archastro scope. A bundled engine's unscoped runtime dependency ` +
            `is probably missing from the wrapper's dependencies: ` +
            `${capture.stdout}\n${capture.stderr}`,
        );
      }
      const imagePath = path.join(hijackDir, output);
      if (!fs.existsSync(imagePath) || fs.statSync(imagePath).size === 0) {
        throw new Error(
          `the hijacked consumer did not generate ${output} via the plain ` +
            "unscoped entry point",
        );
      }
      console.log(
        `verify-packages: hijacked consumer captured ${output} ` +
          `(${fs.statSync(imagePath).size} bytes) through plain ` +
          `\`astroshot ${mode}\``,
      );
    }
  }

  console.log(
    "verify-packages: plain `astroshot` installs and runs with @archastro " +
      "pointed at an unreachable registry",
  );
}

try {
  // Engines must build before pack so dist/ is present in the tarball.
  run("npm", ["run", "build", "--workspace", "@archastro/react-shot"]);
  run("npm", ["run", "build", "--workspace", "@archastro/tui-shot"]);
  run("npm", ["run", "build", "--workspace", "@archastro/movie-harness"]);

  const reactTarball = pack("packages/react-shot");
  const tuiTarball = pack("packages/tui-shot");
  const movieTarball = pack("packages/movie-harness");
  const astroshotTarball = pack("packages/astroshot");
  // Packing the wrapper runs its prepack: build every engine, materialize the
  // local @archastro farm, and refuse to pack an unbuilt engine.
  const unscopedTarball = pack("packages/astroshot-unscoped");

  fs.writeFileSync(
    path.join(consumerDir, "package.json"),
    JSON.stringify(
      {
        name: "astroshots-clean-consumer",
        version: "1.0.0",
        private: true,
        type: "module",
      },
      null,
      2,
    ) + "\n",
  );

  run(
    "npm",
    [
      "install",
      "--no-audit",
      "--no-fund",
      reactTarball,
      tuiTarball,
      movieTarball,
      astroshotTarball,
      "react@19",
      "react-dom@19",
      "ink@7",
    ],
    { cwd: consumerDir },
  );

  run("npx", ["--no-install", "astroshot", "--help"], {
    cwd: consumerDir,
  });
  run("npx", ["--no-install", "astroshot", "react", "--help"], {
    cwd: consumerDir,
  });
  run("npx", ["--no-install", "astroshot", "ink", "--help"], {
    cwd: consumerDir,
  });
  run("npx", ["--no-install", "astroshot", "pty", "--help"], {
    cwd: consumerDir,
  });
  run("npx", ["--no-install", "astroshot", "movie", "--help"], {
    cwd: consumerDir,
  });
  run("npx", ["--no-install", "astroshot", "movie", "which-source", "ratatui tui"], {
    cwd: consumerDir,
  });
  run(
    "node",
    [
      "--input-type=module",
      "-e",
      `
        const react = await import("@archastro/astroshot/react");
        const ink = await import("@archastro/astroshot/ink");
        const pty = await import("@archastro/astroshot/pty");
        if (typeof react.takeShot !== "function") throw new Error("missing React API");
        if (typeof ink.takeTuiShot !== "function") throw new Error("missing Ink API");
        if (typeof pty.takePtyShot !== "function") throw new Error("missing PTY API");
      `,
    ],
    { cwd: consumerDir },
  );

  const unifiedRoot = path.join(
    consumerDir,
    "node_modules",
    "@archastro",
    "astroshot",
  );
  for (const requiredPath of [
    "package.json",
    "bin/astroshot.mjs",
    "react.d.ts",
    "react.js",
    "ink.d.ts",
    "ink.js",
    "pty.d.ts",
    "pty.js",
    "tui.d.ts",
    "tui.js",
  ]) {
    if (!fs.existsSync(path.join(unifiedRoot, requiredPath))) {
      throw new Error(`@archastro/astroshot tarball is missing ${requiredPath}`);
    }
  }

  for (const packageName of ["react-shot", "tui-shot", "movie-harness"]) {
    const packageRoot = path.join(
      consumerDir,
      "node_modules",
      "@archastro",
      packageName,
    );
    const binName =
      packageName === "movie-harness" ? "astroshot-movie.mjs" : `${packageName}.mjs`;
    for (const requiredPath of [
      "package.json",
      `bin/${binName}`,
      "dist/index.js",
      "dist/index.d.ts",
    ]) {
      if (!fs.existsSync(path.join(packageRoot, requiredPath))) {
        throw new Error(
          `@archastro/${packageName} tarball is missing ${requiredPath}`,
        );
      }
    }
    if (packageName === "movie-harness") {
      const swiftHelper = path.join(
        packageRoot,
        "native/macos/WindowTools.swift",
      );
      if (!fs.existsSync(swiftHelper)) {
        throw new Error(
          "@archastro/movie-harness tarball is missing native/macos/WindowTools.swift",
        );
      }
    }

    const installedPackage = JSON.parse(
      fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
    );
    if (installedPackage.version !== packageVersion) {
      throw new Error(
        `@archastro/${packageName} ${installedPackage.version} does not match unified CLI ${packageVersion}`,
      );
    }
  }

  if (process.env.ASTROSHOTS_VERIFY_PACKAGES_CAPTURE === "1") {
    // Prove clean consumers can generate the public React and Ink contracts
    // and immediately capture both without reaching into an engine package.
    run("npx", ["--no-install", "astroshot", "init", "react", "react-fixture.tsx"], {
      cwd: consumerDir,
    });
    run("npx", ["--no-install", "astroshot", "init", "ink", "ink-fixture.tsx"], {
      cwd: consumerDir,
    });
    run("npx", ["--no-install", "astroshot", "init", "pty", "generated-pty.yaml"], {
      cwd: consumerDir,
    });

    fs.writeFileSync(
      path.join(consumerDir, "pty-program.mjs"),
      `process.stdin.setEncoding("utf8");
process.stdin.setRawMode(true);
function render(status = "Choose") {
  process.stdout.write("\\u001b[?1049h\\u001b[2J\\u001b[HPTY package proof\\r\\n" + status);
}
process.stdin.on("data", data => {
  if (data.includes("\\r")) render("Ready");
});
process.on("SIGHUP", () => process.exit(0));
render();
process.stdin.resume();
`,
    );
    fs.writeFileSync(
      path.join(consumerDir, "pty-fixture.yaml"),
      `version: 1
command: node
args: [pty-program.mjs]
cols: 36
rows: 5
scale: 1
actions:
  - waitFor: Choose
  - key: enter
  - waitFor: Ready
expectText: [PTY package proof, Ready]
`,
    );

    run(
      "npx",
      [
        "--no-install",
        "astroshot",
        "react",
        "./react-fixture.tsx",
        "-o",
        "./react-proof.png",
      ],
      { cwd: consumerDir },
    );
    run(
      "npx",
      [
        "--no-install",
        "astroshot",
        "ink",
        "./ink-fixture.tsx",
        "-o",
        "./ink-proof.png",
      ],
      { cwd: consumerDir },
    );
    run(
      "npx",
      [
        "--no-install",
        "astroshot",
        "pty",
        "./pty-fixture.yaml",
        "-o",
        "./pty-proof.png",
      ],
      { cwd: consumerDir },
    );

    for (const image of ["react-proof.png", "ink-proof.png", "pty-proof.png"]) {
      const imagePath = path.join(consumerDir, image);
      if (!fs.existsSync(imagePath) || fs.statSync(imagePath).size === 0) {
        throw new Error(`clean consumer did not generate ${image}`);
      }
    }

    // Simulate an unsupported optional native addon. React and Ink must remain
    // usable, while PTY reports the promised setup error instead of crashing
    // during package import.
    fs.rmSync(path.join(consumerDir, "node_modules", "node-pty"), {
      recursive: true,
      force: true,
    });
    run(
      "npx",
      [
        "--no-install",
        "astroshot",
        "react",
        "./react-fixture.tsx",
        "-o",
        "./react-without-pty.png",
      ],
      { cwd: consumerDir },
    );
    run(
      "npx",
      [
        "--no-install",
        "astroshot",
        "ink",
        "./ink-fixture.tsx",
        "-o",
        "./ink-without-pty.png",
      ],
      { cwd: consumerDir },
    );
    const unavailablePty = spawnSync(
      "npx",
      [
        "--no-install",
        "astroshot",
        "pty",
        "./pty-fixture.yaml",
        "-o",
        "./unavailable-pty.png",
      ],
      { cwd: consumerDir, encoding: "utf8", env: process.env },
    );
    if (unavailablePty.status === 0) {
      throw new Error("PTY capture unexpectedly succeeded without node-pty");
    }
    if (!unavailablePty.stderr.includes("optional node-pty native addon")) {
      throw new Error(
        `PTY capture did not report its optional dependency: ${unavailablePty.stderr}`,
      );
    }
  }

  verifyUnscopedWrapper(unscopedTarball);

  console.log(
    "verify-packages: tarballs install cleanly and expose one astroshot command",
  );
} finally {
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
