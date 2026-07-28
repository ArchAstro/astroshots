let selected = 0;

function render(status = "Choose an option") {
  const first = selected === 0 ? "\x1b[36;1m› First\x1b[0m" : "  First";
  const second = selected === 1 ? "\x1b[36;1m› Second\x1b[0m" : "  Second";
  process.stdout.write(
    `\x1b[?1049h\x1b[2J\x1b[H\x1b[35;1mAstroshot PTY fixture\x1b[0m\r\n\r\n${status}\r\n${first}\r\n${second}`,
  );
}

process.stdin.setEncoding("utf8");
process.stdin.setRawMode(true);
process.stdin.on("data", (data) => {
  if (data.includes("\x1b[B")) {
    selected = 1;
    render();
  }
  if (data.includes("\r")) {
    render(`Ready — selected ${selected === 1 ? "Second" : "First"}`);
  }
});
process.on("SIGHUP", () => process.exit(0));
process.on("SIGTERM", () => process.exit(0));
render();
process.stdin.resume();
