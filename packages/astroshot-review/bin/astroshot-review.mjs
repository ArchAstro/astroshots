#!/usr/bin/env node
import { main } from "../dist/cli.js";

main(process.argv.slice(2)).then(
  (code) => {
    process.exitCode = code;
    // main() has torn the tray down, so the event loop should drain and the
    // process exit on its own. If a stray handle is still referenced (a
    // socket, a pipe), don't sit invisibly in the shell forever: exit. The
    // timer is unref'd, so it never keeps the loop alive itself.
    setTimeout(() => process.exit(code), 1000).unref();
  },
  (error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  },
);
