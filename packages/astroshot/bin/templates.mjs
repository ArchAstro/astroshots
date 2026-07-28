import fs from "node:fs";
import path from "node:path";

const REACT_TEMPLATE = `import type { ReactShotFixture } from "@archastro/astroshot/react";

function ExampleCard() {
  return (
    <article
      data-astroshot
      style={{
        width: 420,
        padding: 24,
        borderRadius: 16,
        background: "#f8fafc",
        color: "#0f172a",
        fontFamily: "system-ui, sans-serif",
        boxShadow: "0 20px 50px rgba(15, 23, 42, 0.16)",
      }}
    >
      <h1 style={{ margin: "0 0 8px", fontSize: 24 }}>React fixture</h1>
      <p style={{ margin: 0 }}>Replace this component with the state you want to capture.</p>
    </article>
  );
}

export default {
  width: 720,
  height: 420,
  background: "#e2e8f0",
  selector: "[data-astroshot]",
  waitFor: "text=React fixture",
  component: <ExampleCard />,
} satisfies ReactShotFixture;
`;

const INK_TEMPLATE = `import React from "react";
import { Box, Text } from "ink";
import type { InkShotFixture } from "@archastro/astroshot/ink";

export default {
  cols: 64,
  rows: 12,
  expectText: ["Ink fixture", "Ready to capture"],
  component: (
    <Box borderStyle="round" borderColor="cyan" paddingX={1} flexDirection="column">
      <Text bold color="cyan">Ink fixture</Text>
      <Text>Ready to capture</Text>
    </Box>
  ),
} satisfies InkShotFixture;
`;

const PTY_TEMPLATE = `version: 1
command: ./target/debug/my-tui
args: []
cwd: .
cols: 100
rows: 30
timeoutMs: 15000
actions:
  - waitFor: "Choose an option"
  - key: down
  - key: enter
  - waitFor: "Ready"
expectText:
  - "Ready"
`;

const PTY_JSON_TEMPLATE = `${JSON.stringify(
  {
    version: 1,
    command: "./target/debug/my-tui",
    args: [],
    cwd: ".",
    cols: 100,
    rows: 30,
    timeoutMs: 15000,
    actions: [
      { waitFor: "Choose an option" },
      { key: "down" },
      { key: "enter" },
      { waitFor: "Ready" },
    ],
    expectText: ["Ready"],
  },
  null,
  2,
)}\n`;

const TEMPLATES = {
  react: {
    defaultPath: "react.shot.tsx",
    label: "React",
    source: REACT_TEMPLATE,
  },
  ink: {
    defaultPath: "ink.shot.tsx",
    label: "Ink",
    source: INK_TEMPLATE,
  },
  pty: {
    defaultPath: "pty.shot.yaml",
    label: "PTY",
    source: PTY_TEMPLATE,
  },
};

export function canonicalTemplateMode(mode) {
  return mode === "tui" ? "ink" : mode;
}

export function writeFixtureTemplate({
  mode,
  outputPath,
  force = false,
  cwd = process.cwd(),
}) {
  const canonicalMode = canonicalTemplateMode(mode);
  const template = TEMPLATES[canonicalMode];
  if (!template) {
    throw new Error("init requires one of: react, ink, pty");
  }

  const requestedPath = outputPath ?? template.defaultPath;
  const absolutePath = path.resolve(cwd, requestedPath);
  const expectedExtension = canonicalMode === "pty" ? /\.(?:ya?ml|json)$/i : /\.tsx$/i;
  if (!expectedExtension.test(absolutePath)) {
    throw new Error(
      canonicalMode === "pty"
        ? "PTY fixture templates must use .yaml, .yml, or .json"
        : `${template.label} fixture templates must use .tsx`,
    );
  }

  const parentDirectory = path.dirname(absolutePath);
  fs.mkdirSync(parentDirectory, { recursive: true });
  const source =
    canonicalMode === "pty" && path.extname(absolutePath).toLowerCase() === ".json"
      ? PTY_JSON_TEMPLATE
      : template.source;
  if (!force) {
    try {
      fs.writeFileSync(absolutePath, source, {
        encoding: "utf8",
        flag: "wx",
      });
    } catch (error) {
      if (error?.code === "EEXIST") {
        throw new Error(`Refusing to overwrite ${absolutePath}; pass --force to replace it`);
      }
      throw error;
    }
  } else {
    let existing;
    try {
      existing = fs.lstatSync(absolutePath);
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
    if (existing?.isSymbolicLink()) {
      throw new Error(`Refusing to replace symbolic link ${absolutePath}`);
    }
    if (existing?.isDirectory()) {
      throw new Error(`Refusing to replace directory ${absolutePath}`);
    }
    const temporaryPath = path.join(
      parentDirectory,
      `.${path.basename(absolutePath)}.${process.pid}.${Date.now()}.tmp`,
    );
    try {
      fs.writeFileSync(temporaryPath, source, {
        encoding: "utf8",
        flag: "wx",
      });
      fs.renameSync(temporaryPath, absolutePath);
    } finally {
      fs.rmSync(temporaryPath, { force: true });
    }
  }

  return {
    absolutePath,
    label: template.label,
    mode: canonicalMode,
  };
}
