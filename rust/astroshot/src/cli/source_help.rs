//! Source selection guidance for humans and coding agents.
//! Keep this the single source of truth for "which --source should I use?"
//!
//! Port of `packages/movie-harness/src/source-help.ts`. All text is
//! byte-identical to the TS output.

use std::sync::LazyLock;

use regex::Regex;

pub const SOURCE_DECISION_TABLE: &str = r##"Which --source should I use?
============================

Pick the FIRST row that matches the thing you need to record:

| You need to record…                              | Use --source        | Why |
|--------------------------------------------------|---------------------|-----|
| A web page / SPA / agent-browser session         | browser             | Headless Chromium; no Screen Recording TCC; deterministic viewport |
| An isolated React component (not a full app)     | browser (or still)  | Prefer stills via `astroshot react` unless motion matters |
| A TUI / CLI / Ink / Ratatui / truecolor terminal | pty                 | SGR→xterm truecolor path; NEVER screenshot Terminal.app |
| Color-critical terminal (brand purple, etc.)     | pty                 | Host terminal themes remapping 16 colors would lie |
| A native macOS app window (SwiftUI, Electron…)   | desktop.window      | Real window pixels via screencapture; needs Screen Recording |
| Menu-bar / tray / status-item / LSUIElement app  | desktop.window      | Real app chrome; list-windows may include popover layers |
| The whole monitor / multi-window desktop         | desktop.display     | (not implemented yet — use desktop.window or frames) |
| Frames from any other tool (Unity, remote, custom)| frames              | You push PNG/JPEG; harness only encodes + sinks |
| You already have PNG frames on disk              | frames              | Multi-process start/push-frame/stop |

Hard rules for agents
---------------------
1. Terminal/TUI color → always `pty` (or `pty-demo` for a truecolor smoke test).
   Do NOT use desktop.window on Terminal.app / iTerm / Ghostty for TUI review.
2. Web UI → `browser`, not desktop of a browser window (loses headless CI + viewport control).
3. Native Mac app chrome → `desktop.window` with --bundle-id or --window-id.
4. Unknown engine that can dump images → `frames`.
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
  astroshot movie run --source desktop.window --feature f --slug s \
    --bundle-id com.example.App --duration-ms 3000

  # List windows (macOS)
  astroshot movie list-windows

  # Push your own frames
  astroshot movie start --feature f --slug s
  astroshot movie push-frame --feature f --file ./frame.png
  astroshot movie stop --feature f --status pass"##;

/// TS `SourceKindHelp`. Declaration order is the TS `SOURCE_CATALOG` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKindHelp {
    Browser,
    Pty,
    PtyDemo,
    DesktopWindow,
    DesktopDisplay,
    DesktopRegion,
    Frames,
}

impl SourceKindHelp {
    /// The `--source` spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Pty => "pty",
            Self::PtyDemo => "pty-demo",
            Self::DesktopWindow => "desktop.window",
            Self::DesktopDisplay => "desktop.display",
            Self::DesktopRegion => "desktop.region",
            Self::Frames => "frames",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        SOURCE_CATALOG
            .iter()
            .map(|entry| entry.source)
            .find(|source| source.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceAdvice {
    pub source: SourceKindHelp,
    pub summary: &'static str,
    pub use_when: &'static [&'static str],
    pub never_when: &'static [&'static str],
    pub required_flags: &'static [&'static str],
    pub example: &'static str,
}

/// TS `SOURCE_CATALOG` (a `Record` whose insertion order is iteration order).
pub static SOURCE_CATALOG: [SourceAdvice; 7] = [
    SourceAdvice {
        source: SourceKindHelp::Browser,
        summary: "Headless Chromium viewport movie via Playwright recordVideo.",
        use_when: &[
            "Recording a web app, SPA, or agent-browser session",
            "You need a fixed viewport and no OS permissions",
            "CI / headless environments",
        ],
        never_when: &[
            "Recording a TUI (use pty)",
            "Recording native app chrome (use desktop.window)",
        ],
        required_flags: &["--feature", "--slug"],
        example: "astroshot movie run --source browser --feature web --slug home --url https://example.com --settle-ms 500",
    },
    SourceAdvice {
        source: SourceKindHelp::Pty,
        summary: "Truecolor terminal movie: node-pty → xterm SGR cells → Chromium frames.",
        use_when: &[
            "Ink, Ratatui, Bubble Tea, curses, or any CLI TUI",
            "Color accuracy matters (truecolor / 256-color)",
            "Deterministic fixture YAML/JSON journeys",
        ],
        never_when: &[
            "Screenshotting Terminal.app/iTerm/Ghostty with desktop.window",
            "Web UIs",
        ],
        required_flags: &["--feature", "--slug", "--fixture"],
        example: "astroshot movie run --source pty --feature tui --slug flow --fixture ./flow.pty.yaml",
    },
    SourceAdvice {
        source: SourceKindHelp::PtyDemo,
        summary: "Built-in truecolor SGR smoke test (no external program).",
        use_when: &[
            "Verifying the truecolor paint/encode path",
            "CI smoke without a real TUI binary",
        ],
        never_when: &["Production journey capture (use pty + fixture)"],
        required_flags: &["--feature", "--slug"],
        example: "astroshot movie run --source pty-demo --feature tui --slug brand",
    },
    SourceAdvice {
        source: SourceKindHelp::DesktopWindow,
        summary: "macOS native window pixels via CGWindowList + screencapture -l sampling.",
        use_when: &[
            "SwiftUI / AppKit / Electron / any real Mac window",
            "Menu-bar / tray / status-item / LSUIElement apps (e.g. Astroshots)",
            "You need the actual app chrome and OS rendering",
        ],
        never_when: &[
            "TUIs (use pty — terminal themes lie about color)",
            "Web-only journeys you can drive headlessly (use browser)",
            "Linux/Windows CI without a Mac (not supported yet)",
        ],
        required_flags: &[
            "--feature",
            "--slug",
            "one of: --window-id | --bundle-id | --title-regex | --owner | --pid",
        ],
        example: "astroshot movie run --source desktop.window --feature app --slug onboard --bundle-id com.example.App --duration-ms 4000 --fps 10",
    },
    SourceAdvice {
        source: SourceKindHelp::DesktopDisplay,
        summary: "Full display capture (planned).",
        use_when: &["Whole-monitor demos"],
        never_when: &["Prefer desktop.window when a single app matters"],
        required_flags: &["(not implemented)"],
        example: "astroshot movie run --source desktop.window …  # until desktop.display ships",
    },
    SourceAdvice {
        source: SourceKindHelp::DesktopRegion,
        summary: "Display region crop (planned).",
        use_when: &["Fixed rectangle on a display"],
        never_when: &["Prefer desktop.window when possible"],
        required_flags: &["(not implemented)"],
        example: "astroshot movie start --feature x --slug region  # push cropped frames for now",
    },
    SourceAdvice {
        source: SourceKindHelp::Frames,
        summary: "Encode a PNG/JPEG sequence you already produce.",
        use_when: &[
            "Custom engines, remote desktops, game captures",
            "Multi-process producers that write images over time",
            "Synthetic / test patterns",
        ],
        never_when: &["You have a first-class source above that fits — use it instead"],
        required_flags: &["--feature", "--slug", "then push-frame or --demo-frames"],
        example: "astroshot movie start --feature x --slug walk && astroshot movie push-frame --feature x --file f.png && astroshot movie stop --feature x",
    },
];

/// Look up the catalog entry for a source kind.
pub fn source_advice(kind: SourceKindHelp) -> &'static SourceAdvice {
    SOURCE_CATALOG
        .iter()
        .find(|entry| entry.source == kind)
        .expect("SOURCE_CATALOG covers every SourceKindHelp")
}

/// Short one-liner for errors when source is wrong/missing.
pub fn source_hint_for_error(kind: Option<&str>) -> String {
    if let Some(entry) = kind.and_then(SourceKindHelp::parse).map(source_advice) {
        return format!("{}\n  example: {}", entry.summary, entry.example);
    }
    "See `astroshot movie which-source` or `astroshot movie --help` for the decision table."
        .to_string()
}

pub fn format_source_catalog() -> String {
    let bullets = |lines: &[&str]| {
        lines
            .iter()
            .map(|line| format!("    • {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let blocks: Vec<String> = SOURCE_CATALOG
        .iter()
        .map(|entry| {
            [
                format!("--source {}", entry.source.as_str()),
                format!("  {}", entry.summary),
                "  Use when:".to_string(),
                bullets(entry.use_when),
                "  Never when:".to_string(),
                bullets(entry.never_when),
                format!("  Required: {}", entry.required_flags.join(", ")),
                format!("  Example: {}", entry.example),
            ]
            .join("\n")
        })
        .collect();
    format!(
        "{SOURCE_DECISION_TABLE}\n\nSource catalog\n--------------\n\n{}\n",
        blocks.join("\n\n")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRecommendation {
    pub source: SourceKindHelp,
    pub reason: &'static str,
}

// JS `\b` is ASCII-only, so each is written `(?-u:\b)`.
static TERMINAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?-u:\b)(tui|pty|terminal|ink|ratatui|curses|bubbletea|cli app|truecolor|ansi)(?-u:\b)",
    )
    .expect("valid regex")
});
static NATIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:\b)(swiftui|appkit|electron|native app|macos app|mac app|native mac|menu[- ]?bar|menubar|status[- ]?item|lsuielement|popover|desktop window|bundle[- ]?id|window id|window[- ]?id)(?-u:\b)")
        .expect("valid regex")
});
static TRAY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)(tray|menu bar tray)(?-u:\b)").expect("valid regex"));
static ASTROSHOTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)astroshots?(?-u:\b)").expect("valid regex"));
static NATIVE_WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)native(?-u:\b)").expect("valid regex"));
static NATIVE_CONTEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:\b)(window|app|desktop|macos|mac|chrome|ui)(?-u:\b)").expect("valid regex")
});
static WEB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:\b)(browser|web|spa|playwright|agent-browser|http|react page|url)(?-u:\b)")
        .expect("valid regex")
});
static FRAMES_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:\b)(frame|png|jpeg|sequence|custom|unity|remote)(?-u:\b)")
        .expect("valid regex")
});

/// Lightweight advisor for agents: pass free-text intent, get a recommended source.
/// This is heuristic — the decision table is authoritative.
///
/// Order matters: TUI beats native (don't desktop Terminal.app); native/menu-bar
/// beats browser so "record the Astroshots tray" never defaults to Chromium.
pub fn recommend_source(intent: &str) -> SourceRecommendation {
    let text = intent.to_lowercase();
    if TERMINAL_RE.is_match(&text) {
        return SourceRecommendation {
            source: SourceKindHelp::Pty,
            reason: "Terminal/TUI intent detected — use pty for truecolor SGR fidelity (not desktop of a terminal app).",
        };
    }
    if NATIVE_RE.is_match(&text)
        // "tray" alone is ambiguous (web trays exist); with an app name / menu-bar
        // product context, treat as native desktop capture.
        || TRAY_RE.is_match(&text)
        || ASTROSHOTS_RE.is_match(&text)
        || (NATIVE_WORD_RE.is_match(&text) && NATIVE_CONTEXT_RE.is_match(&text))
    {
        return SourceRecommendation {
            source: SourceKindHelp::DesktopWindow,
            reason: "Native desktop window intent detected — use desktop.window with --bundle-id or --window-id.",
        };
    }
    if WEB_RE.is_match(&text) {
        return SourceRecommendation {
            source: SourceKindHelp::Browser,
            reason: "Web/browser intent detected — use browser (headless Chromium), not a desktop capture of Chrome.",
        };
    }
    if FRAMES_RE.is_match(&text) {
        return SourceRecommendation {
            source: SourceKindHelp::Frames,
            reason: "Custom frame producer intent — use frames and push-frame.",
        };
    }
    SourceRecommendation {
        source: SourceKindHelp::Browser,
        reason: "No strong signal; defaulting to browser for web-shaped work. Run `astroshot movie which-source` and match the decision table.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_table_mentions_every_implemented_source() {
        for source in ["browser", "pty", "desktop.window", "frames"] {
            assert!(SOURCE_DECISION_TABLE.contains(source));
        }
        assert!(
            SOURCE_DECISION_TABLE
                .to_lowercase()
                .contains("never screenshot terminal")
        );
    }

    #[test]
    fn catalog_has_required_flags_and_examples_for_agents() {
        for entry in &SOURCE_CATALOG {
            assert!(entry.summary.chars().count() > 10);
            assert!(!entry.use_when.is_empty());
            assert!(!entry.never_when.is_empty());
            assert!(entry.example.contains("astroshot movie"));
        }
        assert!(format_source_catalog().contains("--source desktop.window"));
    }

    #[test]
    fn recommends_pty_for_tui_intent() {
        assert_eq!(
            recommend_source("record ratatui truecolor dashboard").source,
            SourceKindHelp::Pty
        );
    }

    #[test]
    fn recommends_desktop_window_for_native_app_intent() {
        assert_eq!(
            recommend_source("SwiftUI onboarding window bundle id").source,
            SourceKindHelp::DesktopWindow
        );
        assert_eq!(
            recommend_source("native mac app").source,
            SourceKindHelp::DesktopWindow
        );
    }

    #[test]
    fn recommends_desktop_window_for_menu_bar_tray_astroshots_intents() {
        assert_eq!(
            recommend_source("record the Astroshots menu-bar tray").source,
            SourceKindHelp::DesktopWindow
        );
        assert_eq!(
            recommend_source("capture Astroshots tray").source,
            SourceKindHelp::DesktopWindow
        );
        assert_eq!(
            recommend_source("status-item popover of my LSUIElement app").source,
            SourceKindHelp::DesktopWindow
        );
    }

    #[test]
    fn recommends_browser_for_web_intent() {
        assert_eq!(
            recommend_source("agent-browser SPA signup flow").source,
            SourceKindHelp::Browser
        );
    }

    #[test]
    fn catalog_text_matches_typescript_output_when_fixture_provided() {
        // Set ASTROSHOT_TS_CATALOG to a dump of TS `formatSourceCatalog()` to
        // check byte identity.
        if let Ok(path) = std::env::var("ASTROSHOT_TS_CATALOG") {
            let expected = std::fs::read_to_string(path).expect("read dump");
            assert_eq!(format_source_catalog(), expected);
        }
    }

    #[test]
    fn source_hint_for_error_uses_entry_or_fallback() {
        assert_eq!(
            source_hint_for_error(Some("pty-demo")),
            "Built-in truecolor SGR smoke test (no external program).\n  example: astroshot movie run --source pty-demo --feature tui --slug brand"
        );
        assert!(source_hint_for_error(None).starts_with("See `astroshot movie which-source`"));
        assert!(source_hint_for_error(Some("bogus")).starts_with("See `astroshot"));
    }
}
