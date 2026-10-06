//! Port of `packages/react-shot/src/types.ts`.
//!
//! Only plain data crosses into Rust. `ReactShotFixture.component` is a React
//! tree and `ReactNode` has no Rust equivalent; the Node helper mounts it, and
//! Rust sees the capture controls as [`ReactShotFixtureMeta`] (TS
//! `Omit<ReactShotFixture, "component">`, also `ShotMeta` in `meta.ts`).

use serde::{Deserialize, Serialize};

/// Capture controls from a fixture, excluding its React tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactShotFixtureMeta {
    /// Viewport width in CSS pixels. Defaults to 1280.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// Viewport height in CSS pixels. Defaults to 800.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Page or canvas background. Defaults to transparent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// CSS selector to capture. Defaults to `[data-react-shot-root]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// Optional selector or `text=...` value that must become visible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_for: Option<String>,
    /// Additional layout settling time in milliseconds. Defaults to 150.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settle_ms: Option<f64>,
    /// Capture the full page instead of the selected element.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_page: Option<bool>,
    /// Remove a full-screen overlay around the target before capture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strip_overlay: Option<bool>,
    /// Preserve transparency in the PNG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omit_background: Option<bool>,
}

/// Package-level config exported from `react-shot.config.*`, after the helper
/// has resolved its relative paths against the config directory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactShotConfig {
    /// Package root used for module resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Vite aliases, such as `@` to an application's source directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<std::collections::BTreeMap<String, String>>,
    /// CSS files imported into every fixture host page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub styles: Option<Vec<String>>,
    /// Path to a PostCSS config file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postcss_config: Option<String>,
    /// Additional dependencies Vite should deduplicate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedupe: Option<Vec<String>>,
    /// Module names that should resolve to an empty browser-safe stub.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stub_modules: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShotRequest {
    pub fixture_path: String,
    pub out_path: String,
    pub root: Option<String>,
    pub config_path: Option<String>,
    pub headed: Option<bool>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchEntry {
    pub fixture: String,
    pub out: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    pub shots: Vec<BatchEntry>,
}
