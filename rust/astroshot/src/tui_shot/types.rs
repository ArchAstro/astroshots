//! Port of `packages/tui-shot/src/types.ts`.
//!
//! Wire format: camelCase JSON field names, absent optional fields omitted,
//! field order as declared in TS.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Terminal-shot fixture. Divergence: the TS interface carries `component`
/// (a React element rendered by Ink). Ink fixtures render in the Node helper
/// (PORTING.md decision 1), so only the serializable settings live here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TuiShotFixture {
    /// Visible strings that must exist before a PNG is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_text: Option<Vec<String>>,
    /// Terminal grid dimensions, not image pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<f64>,
    /// PNG device scale factor. Defaults to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TuiShotRequest {
    pub fixture_path: String,
    pub out_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PtyKey {
    Enter,
    Up,
    Down,
    Left,
    Right,
    Tab,
    Escape,
    Backspace,
    Space,
    CtrlC,
    CtrlD,
}

/// The literal `true` in `{ waitForExit: true }`; deserializing `false` or
/// any other value fails, like the TS literal type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlwaysTrue;

impl Serialize for AlwaysTrue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for AlwaysTrue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("expected true"))
        }
    }
}

/// One step of a PTY script. Matched by shape, as the TS union is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PtyAction {
    #[serde(rename_all = "camelCase")]
    WaitFor {
        wait_for: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    WaitForExit {
        wait_for_exit: AlwaysTrue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<f64>,
    },
    Key {
        key: PtyKey,
    },
    Text {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Pause {
        pause_ms: f64,
    },
}

/// Fixture `version`; only `1` is valid (the TS literal type `1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionOne;

impl Serialize for VersionOne {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}

impl<'de> Deserialize<'de> for VersionOne {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if u8::deserialize(deserializer)? == 1 {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("expected version 1"))
        }
    }
}

/// `graphics?: "kitty"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PtyGraphics {
    Kitty,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyShotFixture {
    pub version: VersionOne,
    /// Executable launched directly, without an intermediary shell.
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Working directory, relative to the fixture file by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_ms: Option<f64>,
    /// Permit a child that exits nonzero before capture. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_non_zero_exit: Option<bool>,
    /// Emulate a graphics-capable terminal: answer the kitty graphics query and
    /// cell-size reports, record transmitted images, and paint them into the
    /// PNG at their placements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics: Option<PtyGraphics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<PtyAction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_text: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyShotRequest {
    pub fixture_path: String,
    pub out_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchEntry {
    pub fixture: String,
    pub out: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchManifest {
    pub shots: Vec<BatchEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pty_actions_match_by_shape() {
        let actions: Vec<PtyAction> = serde_json::from_value(json!([
            { "waitFor": "ready", "timeoutMs": 500 },
            { "waitForExit": true },
            { "key": "ctrl-c" },
            { "text": "hello" },
            { "pauseMs": 50 }
        ]))
        .unwrap();
        assert_eq!(
            actions,
            vec![
                PtyAction::WaitFor {
                    wait_for: "ready".into(),
                    timeout_ms: Some(500.0)
                },
                PtyAction::WaitForExit {
                    wait_for_exit: AlwaysTrue,
                    timeout_ms: None
                },
                PtyAction::Key { key: PtyKey::CtrlC },
                PtyAction::Text {
                    text: "hello".into()
                },
                PtyAction::Pause { pause_ms: 50.0 },
            ]
        );
    }

    #[test]
    fn wait_for_exit_must_be_literal_true() {
        assert!(serde_json::from_value::<PtyAction>(json!({ "waitForExit": false })).is_err());
    }

    #[test]
    fn pty_fixture_round_trips_with_declared_key_order() {
        let text = r#"{"version":1,"command":"node","args":["a.js"],"graphics":"kitty","actions":[{"key":"enter"}],"expectText":["ok"]}"#;
        let fixture: PtyShotFixture = serde_json::from_str(text).unwrap();
        assert_eq!(fixture.graphics, Some(PtyGraphics::Kitty));
        assert_eq!(serde_json::to_string(&fixture).unwrap(), text);
    }

    #[test]
    fn pty_fixture_rejects_other_versions() {
        let result =
            serde_json::from_value::<PtyShotFixture>(json!({ "version": 2, "command": "x" }));
        assert!(result.is_err());
    }

    #[test]
    fn batch_manifest_parses() {
        let manifest: BatchManifest =
            serde_json::from_value(json!({ "shots": [{ "fixture": "a.tsx", "out": "a.png" }] }))
                .unwrap();
        assert_eq!(manifest.shots[0].out, "a.png");
    }
}
