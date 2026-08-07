#!/usr/bin/env node
/**
 * Pack the unified public CLI and its two rendering engines, install their
 * tarballs into a clean consumer, and exercise the one executable users run.
 */
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "astroshots-pack-"));
const archiveDir = path.join(tempRoot, "archives");
const consumerDir = path.join(tempRoot, "consumer");
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

fs.mkdirSync(archiveDir, { recursive: true });
fs.mkdirSync(consumerDir, { recursive: true });

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

try {
  // Engines must build before pack so dist/ is present in the tarball.
  run("npm", ["run", "build", "--workspace", "@archastro/react-shot"]);
  run("npm", ["run", "build", "--workspace", "@archastro/tui-shot"]);
  run("npm", ["run", "build", "--workspace", "@archastro/movie-harness"]);

  const reactTarball = pack("packages/react-shot");
  const tuiTarball = pack("packages/tui-shot");
  const movieTarball = pack("packages/movie-harness");
  const astroshotTarball = pack("packages/astroshot");

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

  for (const packageName of ["react-shot", "tui-shot"]) {
    const packageRoot = path.join(
      consumerDir,
      "node_modules",
      "@archastro",
      packageName,
    );
    for (const requiredPath of [
      "package.json",
      `bin/${packageName}.mjs`,
      "dist/index.js",
      "dist/index.d.ts",
    ]) {
      if (!fs.existsSync(path.join(packageRoot, requiredPath))) {
        throw new Error(
          `@archastro/${packageName} tarball is missing ${requiredPath}`,
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

  console.log(
    "verify-packages: tarballs install cleanly and expose one astroshot command",
  );
} finally {
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
