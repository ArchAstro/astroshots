import { createElement } from "react";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

import { render, Text } from "ink";
import { describe, expect, it } from "vitest";

import { renderInkFrame } from "./render-ink.js";
import { ansiFrameToHtml } from "./terminal-html.js";
import type { TuiShotFixture } from "./types.js";

const testRequire = createRequire(import.meta.url);
const inkRequire = createRequire(testRequire.resolve("ink"));
const chalk = (
  (await import(pathToFileURL(inkRequire.resolve("chalk")).href)) as {
    default: { level: number };
  }
).default;

describe("ansiFrameToHtml", () => {
  it("preserves terminal colors and escapes fixture content", async () => {
    const html = await ansiFrameToHtml(
      "\u001b[38;2;124;92;255mPurple <frame>\u001b[0m",
      {
        cols: 24,
        rows: 2,
        foreground: "#ffffff",
        background: "#090a12",
      },
    );
    expect(html).toContain("color:#7c5cff");
    expect(html).toContain("Purple &lt;frame&gt;");
  });

  it("preserves a real Ink component's truecolor styling", async () => {
    const fixture: TuiShotFixture = {
      component: createElement(Text, { color: "#7c5cff" }, "Brand purple"),
    };
    const ansi = renderInkFrame(fixture, 24, 2, { render, chalk });
    const html = await ansiFrameToHtml(ansi, {
      cols: 24,
      rows: 2,
      foreground: "#ffffff",
      background: "#090a12",
    });
    expect(html).toContain("color:#7c5cff");
    expect(html).toContain("Brand purple");
  });

  it("keeps the first row when every terminal row fills the exact width", async () => {
    const html = await ansiFrameToHtml(
      ["╭────────╮", "│ frame  │", "│ footer │", "╰────────╯"].join("\n"),
      {
        cols: 10,
        rows: 4,
        foreground: "#ffffff",
        background: "#090a12",
      },
    );
    expect(html).toContain("╭────────╮");
    expect(html).toContain("╰────────╯");
    expect(html.match(/class="tui-row"/g)).toHaveLength(4);
  });
});
