process.stdout.write(
  "\x1b[?1049h\x1b[2J\x1b[H\x1b[31;1mFatal terminal state\x1b[0m",
);
setTimeout(() => process.exit(7), 20);
