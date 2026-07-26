#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const cli = path.join(root, "dist", "cli.js");
const require = createRequire(import.meta.url);
const tsx = require.resolve("tsx/cli");
const result = spawnSync(process.execPath, [tsx, cli, ...process.argv.slice(2)], {
  stdio: "inherit",
});

if (result.error) {
  console.error(`tui-shot could not start: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
