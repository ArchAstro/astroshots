import { describe, expect, it } from "vitest";

import {
  formatSourceCatalog,
  recommendSource,
  SOURCE_CATALOG,
  SOURCE_DECISION_TABLE,
} from "./source-help.js";

describe("source selection help", () => {
  it("decision table mentions every implemented source", () => {
    for (const source of ["browser", "pty", "desktop.window", "frames"]) {
      expect(SOURCE_DECISION_TABLE).toContain(source);
    }
    expect(SOURCE_DECISION_TABLE).toMatch(/NEVER screenshot Terminal/i);
  });

  it("catalog has required flags and examples for agents", () => {
    for (const entry of Object.values(SOURCE_CATALOG)) {
      expect(entry.summary.length).toBeGreaterThan(10);
      expect(entry.useWhen.length).toBeGreaterThan(0);
      expect(entry.neverWhen.length).toBeGreaterThan(0);
      expect(entry.example).toContain("astroshot movie");
    }
    expect(formatSourceCatalog()).toContain("--source desktop.window");
  });

  it("recommends pty for TUI intent", () => {
    expect(recommendSource("record ratatui truecolor dashboard").source).toBe(
      "pty",
    );
  });

  it("recommends desktop.window for native app intent", () => {
    expect(
      recommendSource("SwiftUI onboarding window bundle id").source,
    ).toBe("desktop.window");
    expect(recommendSource("native mac app").source).toBe("desktop.window");
  });

  it("recommends desktop.window for menu-bar / tray / Astroshots intents", () => {
    expect(
      recommendSource("record the Astroshots menu-bar tray").source,
    ).toBe("desktop.window");
    expect(recommendSource("capture Astroshots tray").source).toBe(
      "desktop.window",
    );
    expect(recommendSource("status-item popover of my LSUIElement app").source)
      .toBe("desktop.window");
  });

  it("recommends browser for web intent", () => {
    expect(recommendSource("agent-browser SPA signup flow").source).toBe(
      "browser",
    );
  });
});
