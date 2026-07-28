import { spawn } from "node:child_process";

const [, , token, command, ...args] = process.argv;

if (!token || !command) {
  process.stderr.write("Astroshot PTY status wrapper requires a token and command.\n");
  process.exit(2);
}

const child = spawn(command, args, {
  cwd: process.cwd(),
  env: process.env,
  stdio: "inherit",
  windowsHide: true,
});

let reported = false;

// Terminal-generated signals are delivered to the PTY process group. Keep the
// bridge alive so the target's exit callback can report its authoritative code.
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"] as const) {
  process.on(signal, () => {});
}

function reportExit(code: number): void {
  if (reported) return;
  reported = true;
  const marker = `\x1b]777;astroshot-exit-${token}=${code}\x07`;
  process.stdout.write(marker, () => process.exit(0));
}

child.once("error", (error) => {
  process.stderr.write(`Unable to launch PTY program: ${error.message}\n`);
  reportExit(127);
});

child.once("exit", (code) => {
  reportExit(code ?? 1);
});
