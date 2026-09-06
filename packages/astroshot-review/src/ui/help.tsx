import { Box, Text } from "ink";

import { theme } from "./theme.js";

const SECTIONS: Array<{ title: string; keys: Array<[string, string]> }> = [
  {
    title: "Everywhere",
    keys: [
      ["1 / 2", "Shots · Friction Logs"],
      ["tab", "next tab"],
      [",", "settings"],
      ["r", "rescan"],
      ["?", "this help"],
      ["q", "quit"],
    ],
  },
  {
    title: "Stream",
    keys: [
      ["↑↓ j k", "move"],
      ["⏎", "open detail"],
      ["f", "full-screen review"],
      ["u", "Unseen ⇄ History"],
      ["m", "Movies only"],
      ["s", "mark Seen"],
      ["S", "mark all visible Seen"],
      ["A", "mark this worktree Seen"],
      ["c", "send feedback"],
      ["z", "collapse worktree"],
      ["o", "reveal in Finder"],
      ["O", "open movie in player"],
      ["y", "copy image"],
    ],
  },
  {
    title: "Detail / Review",
    keys: [
      ["← →", "older · newer"],
      ["+ / -", "zoom in / out"],
      ["0", "reset zoom"],
      ["p", "play in tray"],
      ["space", "play / pause"],
      [", .", "seek −5s / +5s"],
      ["[ ]", "previous / next chapter"],
      ["esc", "back / close"],
    ],
  },
  {
    title: "Friction Logs",
    keys: [
      ["⏎", "open log · open step"],
      ["[ ]", "switch run · switch image"],
      ["p", "toggle prompt"],
      ["← →", "previous / next step"],
    ],
  },
];

export function HelpOverlay({ width, height }: { width: number; height: number }) {
  const columns = width >= 100 ? 2 : 1;
  const columnWidth = Math.floor((width - 4) / columns);
  return (
    <Box flexShrink={0} flexDirection="column" width={width} height={height} paddingX={2} paddingY={1}>
      <Text bold>Keyboard</Text>
      <Text color={theme.muted}>Press ? or esc to close</Text>
      <Box flexDirection="row" flexWrap="wrap" marginTop={1}>
        {SECTIONS.map((section) => (
          <Box key={section.title} flexDirection="column" width={columnWidth} marginBottom={1}>
            <Text color={theme.brand} bold>
              {section.title}
            </Text>
            {section.keys.map(([key, label]) => (
              <Text key={key}>
                <Text color={theme.blue}>{key.padEnd(10)}</Text>
                <Text color={theme.text}>{label}</Text>
              </Text>
            ))}
          </Box>
        ))}
      </Box>
    </Box>
  );
}
