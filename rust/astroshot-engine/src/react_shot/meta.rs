//! Port of `packages/react-shot/src/meta.ts`.

use std::sync::LazyLock;

use regex::Regex;

use super::types::ReactShotFixtureMeta;

/// Capture controls from a fixture, excluding its React tree.
pub type ShotMeta = ReactShotFixtureMeta;

/// JS `\s` (differs from Rust's: U+FEFF in JS only, U+0085 in Rust only).
const JS_WS: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

/// `/role\s*=\s*["']?dialog["']?/i`, ASCII case folding only like JS without `u`.
static DIALOG_SELECTOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i-u:role){JS_WS}*={JS_WS}*["']?(?i-u:dialog)["']?"#
    ))
    .expect("dialog selector regex")
});

/// CLI viewport overrides (`cli` parameter of `resolveShotMeta`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CliViewport {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// `Required<Pick<ShotMeta, ...>> & ShotMeta`: the required keys are always
/// set, `wait_for` and `background` stay optional.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedShotMeta {
    pub width: f64,
    pub height: f64,
    pub background: Option<String>,
    pub selector: String,
    pub wait_for: Option<String>,
    pub settle_ms: f64,
    pub full_page: bool,
    pub strip_overlay: bool,
    pub omit_background: bool,
}

fn pick<T: Clone>(browser: &Option<T>, node: &Option<T>) -> Option<T> {
    browser.clone().or_else(|| node.clone())
}

/// `{ ...nodeMeta, ...browserMeta }` with defaults applied; browser values win.
pub fn resolve_shot_meta(
    node_meta: &ShotMeta,
    browser_meta: &ShotMeta,
    cli: CliViewport,
) -> ResolvedShotMeta {
    let width = pick(&browser_meta.width, &node_meta.width);
    let height = pick(&browser_meta.height, &node_meta.height);
    let background = pick(&browser_meta.background, &node_meta.background);
    let selector_meta = pick(&browser_meta.selector, &node_meta.selector);
    let wait_for = pick(&browser_meta.wait_for, &node_meta.wait_for);
    let settle_ms = pick(&browser_meta.settle_ms, &node_meta.settle_ms);
    let full_page = pick(&browser_meta.full_page, &node_meta.full_page);
    let strip_overlay = pick(&browser_meta.strip_overlay, &node_meta.strip_overlay);
    let omit_background = pick(&browser_meta.omit_background, &node_meta.omit_background);

    let selector = selector_meta.unwrap_or_else(|| "[data-react-shot-root]".to_string());
    let looks_like_dialog = DIALOG_SELECTOR_RE.is_match(&selector);

    ResolvedShotMeta {
        width: cli.width.or(width).unwrap_or(1280.0),
        height: cli.height.or(height).unwrap_or(800.0),
        background,
        wait_for,
        settle_ms: settle_ms.unwrap_or(150.0),
        full_page: full_page.unwrap_or(false),
        strip_overlay: strip_overlay.unwrap_or(looks_like_dialog),
        omit_background: omit_background.unwrap_or(strip_overlay.unwrap_or(looks_like_dialog)),
        selector,
    }
}

pub fn is_dialog_selector(selector: &str) -> bool {
    DIALOG_SELECTOR_RE.is_match(selector)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(selector: Option<&str>, strip_overlay: Option<bool>) -> ShotMeta {
        ShotMeta {
            selector: selector.map(str::to_string),
            strip_overlay,
            ..ShotMeta::default()
        }
    }

    #[test]
    fn isolates_dialog_targets_and_preserves_transparent_corners_by_default() {
        let resolved = resolve_shot_meta(
            &ShotMeta::default(),
            &meta(Some("[role=dialog]"), None),
            CliViewport::default(),
        );
        assert_eq!(resolved.selector, "[role=dialog]");
        assert!(resolved.strip_overlay);
        assert!(resolved.omit_background);
    }

    #[test]
    fn captures_the_fixture_root_without_overlay_removal_by_default() {
        let resolved = resolve_shot_meta(
            &ShotMeta::default(),
            &ShotMeta::default(),
            CliViewport::default(),
        );
        assert_eq!(resolved.selector, "[data-react-shot-root]");
        assert!(!resolved.strip_overlay);
    }

    #[test]
    fn uses_browser_fixture_metadata_when_node_cannot_import_tsx() {
        let browser = ShotMeta {
            width: Some(960.0),
            height: Some(1000.0),
            selector: Some("[role=dialog]".to_string()),
            strip_overlay: Some(true),
            ..ShotMeta::default()
        };
        let resolved = resolve_shot_meta(&ShotMeta::default(), &browser, CliViewport::default());
        assert_eq!(resolved.width, 960.0);
        assert_eq!(resolved.height, 1000.0);
        assert_eq!(resolved.selector, "[role=dialog]");
        assert!(resolved.strip_overlay);
    }

    #[test]
    fn honors_explicit_dialog_overlay_behavior() {
        let resolved = resolve_shot_meta(
            &ShotMeta::default(),
            &meta(Some("[role=dialog]"), Some(false)),
            CliViewport::default(),
        );
        assert!(!resolved.strip_overlay);
    }

    #[test]
    fn recognizes_common_dialog_selectors() {
        assert!(is_dialog_selector("[role=dialog]"));
        assert!(is_dialog_selector("[role=\"dialog\"]"));
        assert!(is_dialog_selector("[role='dialog']"));
        assert!(!is_dialog_selector("#root"));
    }
}
