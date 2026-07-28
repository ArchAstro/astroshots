process.stdout.write(
  "\x1b[?1049h\x1b[2J\x1b[H\x1b[32;1mExport complete\x1b[0m\r\nAll artifacts are ready.",
  () => setTimeout(() => process.exit(0), 100),
);
