// Owns `globalThis.React` before a capture, like the sentinel in
// packages/tui-shot/src/shot.e2e.test.ts ("restores an existing global React
// bridge after a failed capture"). The component renders nothing printable,
// so the Ink render fails while the helper's own React bridge is installed.
import { Text } from "ink";
import { createElement } from "react";

Object.defineProperty(globalThis, "React", {
  configurable: true,
  value: { source: "test-owner" },
  writable: false,
});

export default {
  cols: 20,
  rows: 3,
  component: createElement(Text, null, " "),
};
