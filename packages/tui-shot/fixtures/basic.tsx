import { Box, Text } from "ink";
import React from "react";

import type { TuiShotFixture } from "../src/types.js";

export default {
  cols: 42,
  rows: 8,
  scale: 1,
  expectText: ["✦ tui-shot", "Real Ink rendered through xterm."],
  component: (
    <Box
      borderStyle="round"
      borderColor="#b9a8ff"
      paddingX={1}
      flexDirection="column"
    >
      <Text color="#b9a8ff" bold>
        ✦ tui-shot
      </Text>
      <Text>Real Ink rendered through xterm.</Text>
    </Box>
  ),
} satisfies TuiShotFixture;
