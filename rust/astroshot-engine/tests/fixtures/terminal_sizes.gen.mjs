// Generates terminal_sizes.json, the expected sizes for tests/tui_shot_sizes.rs.
//
// Each row is one real TS capture: `captureTerminalHtml` from
// packages/tui-shot/src/shot.ts lays the `[data-tui-shot]` box out in
// Chromium and screenshots it; `png` is the size read from the PNG header and
// `css` is the box size the TS computed (`Math.ceil(...)`, same expressions as
// shot.ts and movie-harness terminal-paint.ts `terminalDocument`).
//
// Regenerate (needs Playwright's Chromium):
//   (cd packages/tui-shot && npm run build)
//   node rust/astroshot/tests/fixtures/terminal_sizes.gen.mjs
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../../..");
const { captureTerminalHtml, closeSharedBrowser } = await import(
  path.join(repo, "packages/tui-shot/dist/shot.js")
);

// [cols, rows, fontSize, lineHeight, padding, scale]
const cases = [];
const tui = (cols, rows, scale) => cases.push([cols, rows, 15, 1.32, 22, scale]);
const movie = (cols, rows, scale) => cases.push([cols, rows, 14, 1.35, 16, scale]);
for (let rows = 1; rows <= 100; rows++) {
  tui(100, rows, 2); // tui-shot defaults
  tui(52, rows, 1); // interactive-pty.yaml grid and scale
  movie(80, rows, 2); // movie PTY defaults
  cases.push([64, rows, 16, 1.35, 16, 2]); // movie truecolor demo font
}
for (let cols = 1; cols <= 300; cols += 1) {
  tui(cols, 3, 1);
  movie(cols, 3, 1);
}
// Fractional device scale and fractional font metrics.
for (const scale of [0.3, 0.5, 1.1, 1.15, 1.25, 1.5, 1.7, 1.75, 2.5, 3.3, 4]) {
  tui(51, 11, scale);
  tui(33, 7, scale);
  movie(33, 7, scale);
  cases.push([33, 7, 13.5, 1.4, 10.5, scale]);
  cases.push([40, 9, 15, 1.3, 0, scale]);
}

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "terminal-sizes-"));
const outPath = path.join(dir, "size.png");
const table = [];
for (const [cols, rows, fontSize, lineHeight, padding, scale] of cases) {
  await captureTerminalHtml({
    terminalRows: "",
    outPath,
    cols,
    rows,
    scale,
    background: "#090a12",
    foreground: "#e8e8f2",
    fontFamily: "monospace",
    fontSize,
    lineHeight,
    padding,
    borderRadius: 12,
  });
  const png = fs.readFileSync(outPath);
  table.push({
    cols,
    rows,
    fontSize,
    lineHeight,
    padding,
    scale,
    css: [
      Math.ceil(cols * fontSize * 0.62 + padding * 2),
      Math.ceil(rows * fontSize * lineHeight + padding * 2),
    ],
    png: [png.readUInt32BE(16), png.readUInt32BE(20)],
  });
}
await closeSharedBrowser();
fs.rmSync(dir, { recursive: true, force: true });
fs.writeFileSync(
  path.join(here, "terminal_sizes.json"),
  `[\n${table.map((row) => `  ${JSON.stringify(row)}`).join(",\n")}\n]\n`,
);
console.log(`wrote ${table.length} rows`);
