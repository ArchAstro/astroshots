/**
 * Source selection guidance for humans and coding agents.
 * Keep this the single source of truth for "which --source should I use?"
 */

export const SOURCE_DECISION_TABLE = `
Which --source should I use?
============================

Pick the FIRST row that matches the thing you need to record:

| You need to record…                              | Use --source        | Why |
|--------------------------------------------------|---------------------|-----|
| A web page / SPA / agent-browser session         | browser             | Headless Chromium; no Screen Recording TCC; deterministic viewport |
| An isolated React component (not a full app)     | browser (or still)  | Prefer stills via \`astroshot react\` unless motion matters |
| A TUI / CLI / Ink / Ratatui / truecolor terminal | pty                 | SGR→xterm truecolor path; NEVER screenshot Terminal.app |
| Color-critical terminal (brand purple, etc.)     | pty                 | Host terminal themes remapping 16 colors would lie |
| A native macOS app window (SwiftUI, Electron…)   | desktop.window      | Real window pixels via screencapture; needs Screen Recording |
| The whole monitor / multi-window desktop         | desktop.display     | (not implemented yet — use desktop.window or frames) |
| Frames from any other tool (Unity, remote, custom)| frames              | You push PNG/JPEG; harness only encodes + sinks |
| You already have PNG frames on disk              | frames              | Multi-process start/push-frame/stop |

Hard rules for agents
---------------------
1. Terminal/TUI color → always \`pty\` (or \`pty-demo\` for a truecolor smoke test).
   Do NOT use desktop.window on Terminal.app / iTerm / Ghostty for TUI review.
2. Web UI → \`browser\`, not desktop of a browser window (loses headless CI + viewport control).
3. Native Mac app chrome → \`desktop.window\` with --bundle-id or --window-id.
4. Unknown engine that can dump images → \`frames\`.
5. Always write into .astroshot/ via this harness (poster PNG + video) so Astroshots can stream posters today.

Permission / environment
------------------------
| Source          | Needs                  | CI-friendly? |
|-----------------|------------------------|--------------|
| browser         | Playwright Chromium    | yes (headless) |
| pty / pty-demo  | node-pty (optional)    | yes |
| frames          | nothing special        | yes |
| desktop.window  | macOS + Screen Recording TCC + WindowTools (Swift) | hard |

Quick commands
--------------
  # Web journey
  astroshot movie run --source browser --feature f --slug s --url https://…

  # Truecolor TUI fixture
  astroshot movie run --source pty --feature f --slug s --fixture ./flow.pty.yaml

  # Native app window (largest window of bundle, 3s)
  astroshot movie run --source desktop.window --feature f --slug s \\
    --bundle-id com.example.App --duration-ms 3000

  # List windows (macOS)
  astroshot movie list-windows

  # Push your own frames
  astroshot movie start --feature f --slug s
  astroshot movie push-frame --feature f --file ./frame.png
  astroshot movie stop --feature f --status pass
`.trim();

export type SourceKindHelp =
  | "browser"
  | "pty"
  | "pty-demo"
  | "desktop.window"
  | "desktop.display"
  | "desktop.region"
  | "frames";

export interface SourceAdvice {
  source: SourceKindHelp;
  summary: string;
  useWhen: string[];
  neverWhen: string[];
  requiredFlags: string[];
  example: string;
}

export const SOURCE_CATALOG: Record<SourceKindHelp, SourceAdvice> = {
  browser: {
    source: "browser",
    summary: "Headless Chromium viewport movie via Playwright recordVideo.",
    useWhen: [
      "Recording a web app, SPA, or agent-browser session",
      "You need a fixed viewport and no OS permissions",
      "CI / headless environments",
    ],
    neverWhen: [
      "Recording a TUI (use pty)",
      "Recording native app chrome (use desktop.window)",
    ],
    requiredFlags: ["--feature", "--slug"],
    example:
      "astroshot movie run --source browser --feature web --slug home --url https://example.com --settle-ms 500",
  },
  pty: {
    source: "pty",
    summary:
      "Truecolor terminal movie: node-pty → xterm SGR cells → Chromium frames.",
    useWhen: [
      "Ink, Ratatui, Bubble Tea, curses, or any CLI TUI",
      "Color accuracy matters (truecolor / 256-color)",
      "Deterministic fixture YAML/JSON journeys",
    ],
    neverWhen: [
      "Screenshotting Terminal.app/iTerm/Ghostty with desktop.window",
      "Web UIs",
    ],
    requiredFlags: ["--feature", "--slug", "--fixture"],
    example:
      "astroshot movie run --source pty --feature tui --slug flow --fixture ./flow.pty.yaml",
  },
  "pty-demo": {
    source: "pty-demo",
    summary: "Built-in truecolor SGR smoke test (no external program).",
    useWhen: [
      "Verifying the truecolor paint/encode path",
      "CI smoke without a real TUI binary",
    ],
    neverWhen: ["Production journey capture (use pty + fixture)"],
    requiredFlags: ["--feature", "--slug"],
    example:
      "astroshot movie run --source pty-demo --feature tui --slug brand",
  },
  "desktop.window": {
    source: "desktop.window",
    summary:
      "macOS native window pixels via CGWindowList + screencapture -l sampling.",
    useWhen: [
      "SwiftUI / AppKit / Electron / any real Mac window",
      "You need the actual app chrome and OS rendering",
    ],
    neverWhen: [
      "TUIs (use pty — terminal themes lie about color)",
      "Web-only journeys you can drive headlessly (use browser)",
      "Linux/Windows CI without a Mac (not supported yet)",
    ],
    requiredFlags: [
      "--feature",
      "--slug",
      "one of: --window-id | --bundle-id | --title-regex | --owner | --pid",
    ],
    example:
      "astroshot movie run --source desktop.window --feature app --slug onboard --bundle-id com.example.App --duration-ms 4000 --fps 10",
  },
  "desktop.display": {
    source: "desktop.display",
    summary: "Full display capture (planned).",
    useWhen: ["Whole-monitor demos"],
    neverWhen: ["Prefer desktop.window when a single app matters"],
    requiredFlags: ["(not implemented)"],
    example:
      "astroshot movie run --source desktop.window …  # until desktop.display ships",
  },
  "desktop.region": {
    source: "desktop.region",
    summary: "Display region crop (planned).",
    useWhen: ["Fixed rectangle on a display"],
    neverWhen: ["Prefer desktop.window when possible"],
    requiredFlags: ["(not implemented)"],
    example:
      "astroshot movie start --feature x --slug region  # push cropped frames for now",
  },
  frames: {
    source: "frames",
    summary: "Encode a PNG/JPEG sequence you already produce.",
    useWhen: [
      "Custom engines, remote desktops, game captures",
      "Multi-process producers that write images over time",
      "Synthetic / test patterns",
    ],
    neverWhen: [
      "You have a first-class source above that fits — use it instead",
    ],
    requiredFlags: ["--feature", "--slug", "then push-frame or --demo-frames"],
    example:
      "astroshot movie start --feature x --slug walk && astroshot movie push-frame --feature x --file f.png && astroshot movie stop --feature x",
  },
};

/** Short one-liner for errors when source is wrong/missing. */
export function sourceHintForError(kind?: string): string {
  if (kind && kind in SOURCE_CATALOG) {
    const entry = SOURCE_CATALOG[kind as SourceKindHelp];
    return `${entry.summary}\n  example: ${entry.example}`;
  }
  return (
    "See `astroshot movie which-source` or `astroshot movie --help` for the decision table."
  );
}

export function formatSourceCatalog(): string {
  const blocks = Object.values(SOURCE_CATALOG).map((entry) => {
    const use = entry.useWhen.map((line) => `    • ${line}`).join("\n");
    const never = entry.neverWhen.map((line) => `    • ${line}`).join("\n");
    return [
      `--source ${entry.source}`,
      `  ${entry.summary}`,
      `  Use when:`,
      use,
      `  Never when:`,
      never,
      `  Required: ${entry.requiredFlags.join(", ")}`,
      `  Example: ${entry.example}`,
    ].join("\n");
  });
  return `${SOURCE_DECISION_TABLE}\n\nSource catalog\n--------------\n\n${blocks.join("\n\n")}\n`;
}

/**
 * Lightweight advisor for agents: pass free-text intent, get a recommended source.
 * This is heuristic — the decision table is authoritative.
 */
export function recommendSource(intent: string): {
  source: SourceKindHelp;
  reason: string;
} {
  const text = intent.toLowerCase();
  if (
    /\b(tui|pty|terminal|ink|ratatui|curses|bubbletea|cli app|truecolor|ansi)\b/.test(
      text,
    )
  ) {
    return {
      source: "pty",
      reason:
        "Terminal/TUI intent detected — use pty for truecolor SGR fidelity (not desktop of a terminal app).",
    };
  }
  if (
    /\b(swiftui|appkit|electron|native app|macos app|mac app|native mac|menu ?bar|desktop window|bundle[- ]?id|window id)\b/.test(
      text,
    ) ||
    /\b(swiftui|appkit|electron)\b/.test(text) ||
    (/\bnative\b/.test(text) && /\b(window|app|desktop|macos|mac)\b/.test(text))
  ) {
    return {
      source: "desktop.window",
      reason:
        "Native desktop window intent detected — use desktop.window with --bundle-id or --window-id.",
    };
  }
  if (
    /\b(browser|web|spa|playwright|agent-browser|http|react page|url)\b/.test(
      text,
    )
  ) {
    return {
      source: "browser",
      reason:
        "Web/browser intent detected — use browser (headless Chromium), not a desktop capture of Chrome.",
    };
  }
  if (/\b(frame|png|jpeg|sequence|custom|unity|remote)\b/.test(text)) {
    return {
      source: "frames",
      reason: "Custom frame producer intent — use frames and push-frame.",
    };
  }
  return {
    source: "browser",
    reason:
      "No strong signal; defaulting to browser for web-shaped work. Run `astroshot movie which-source` and match the decision table.",
  };
}
