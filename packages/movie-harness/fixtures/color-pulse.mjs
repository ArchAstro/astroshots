#!/usr/bin/env node
/** Minimal truecolor PTY program for movie fixtures. */
const colors = [
  [124, 92, 255],
  [80, 220, 160],
  [240, 114, 122],
  [90, 200, 250],
];

process.stdout.write("\x1b[?25l");
for (const [r, g, b] of colors) {
  process.stdout.write(
    `\x1b[2J\x1b[H\x1b[38;2;${r};${g};${b}m██ brand #${r.toString(16)}${g.toString(16)}${b.toString(16)}\x1b[0m\nready\n`,
  );
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 200);
}
process.stdout.write("\x1b[?25h");
process.exit(0);
