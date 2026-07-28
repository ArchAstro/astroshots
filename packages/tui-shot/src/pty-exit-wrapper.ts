import { spawn } from "node:child_process";
import fs from "node:fs";

const [, , token, statusPath, command, ...args] = process.argv;

if (!token || !statusPath || !command) {
  process.stderr.write(
    "Astroshot PTY status wrapper requires a token, status path, and command.\n",
  );
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
  const temporaryStatusPath = `${statusPath}.${process.pid}.tmp`;
  try {
    fs.writeFileSync(temporaryStatusPath, String(code), { mode: 0o600 });
    fs.renameSync(temporaryStatusPath, statusPath);
  } catch (error) {
    process.stderr.write(
      `Unable to write PTY program exit status: ${
        error instanceof Error ? error.message : String(error)
      }\n`,
    );
  }

  const marker = `\x1b]777;astroshot-exit-${token}=${code}\x07`;
  const writeMarker = () => {
    process.stdout.write(marker, () => process.exit(0));
  };
  const markerDelay = Number(
    process.env.ASTROSHOT_TEST_DELAY_PTY_EXIT_MARKER_MS ?? 0,
  );
  if (Number.isFinite(markerDelay) && markerDelay > 0) {
    setTimeout(writeMarker, markerDelay);
  } else {
    writeMarker();
  }
}

child.once("error", (error) => {
  process.stderr.write(`Unable to launch PTY program: ${error.message}\n`);
  reportExit(127);
});

child.once("exit", (code) => {
  reportExit(code ?? 1);
});
