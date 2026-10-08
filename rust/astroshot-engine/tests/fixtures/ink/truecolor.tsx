// A real Ink component styled with a truecolor chalk color, as in
// packages/tui-shot/src/terminal-html.test.ts ("preserves a real Ink
// component's truecolor styling"), plus a row of the characters the TS HTML
// path had to escape. The package.json beside this file makes tsx load these
// fixtures as ES modules, like the ones in packages/tui-shot.
//
// The color is asserted on the border. A fixture file gets its own copy of
// ink (and chalk) from tsx's `tsImport`, separate from the copy whose
// `render` draws the frame and whose chalk level is raised to truecolor, so
// `<Text color>` set in a fixture file is not colored. That is the same in
// the TS `tui-shot shot` and in the Node helper; borders are drawn by the
// renderer's copy and keep their color.
import { Box, Text } from "ink";
import { createElement } from "react";

export default {
  cols: 24,
  rows: 4,
  foreground: "#ffffff",
  background: "#090a12",
  component: createElement(
    Box,
    { borderStyle: "round", borderColor: "#7c5cff", flexDirection: "column" },
    createElement(Text, { color: "#7c5cff" }, "Brand purple"),
    createElement(Text, null, `<a> & "b" 'c'`),
  ),
};
