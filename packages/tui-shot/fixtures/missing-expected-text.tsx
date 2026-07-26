import { Text } from "ink";

import type { TuiShotFixture } from "../src/types.js";

export default {
  cols: 20,
  rows: 3,
  expectText: ["Intended screen"],
  component: <Text>Unexpected screen</Text>,
} satisfies TuiShotFixture;
