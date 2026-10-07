// Reports what `globalThis.React` is when this module loads, which is before
// the helper installs its bridge for this render. The report travels back as
// `expectText`, which the helper returns untouched.
import { Text } from "ink";
import { createElement } from "react";

const descriptor = Object.getOwnPropertyDescriptor(globalThis, "React");
const report = JSON.stringify({
  present: descriptor !== undefined,
  source: (descriptor?.value as { source?: string } | undefined)?.source ?? null,
  writable: descriptor?.writable ?? null,
  configurable: descriptor?.configurable ?? null,
});

export default {
  cols: 20,
  rows: 3,
  expectText: [report],
  component: createElement(Text, null, "probe"),
};
