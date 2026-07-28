process.on("SIGINT", () => {
  process.stdout.write("\r\nInterrupted cleanly", () => process.exit(42));
});

process.stdout.write("Ready for interrupt\r\n");
setInterval(() => {}, 1_000);
