//! Request, manifest, and session types for the movie harness.
//!
//! Port of `packages/movie-harness/src/types.ts`. Several of these are written
//! to disk (`PersistedFrameSession`) or passed between processes, so field
//! names (camelCase) and declaration order match the TS interfaces; optional
//! fields are omitted when absent, as `JSON.stringify` omits `undefined`.
//! Numbers that JS may hold as fractions use [`js_number`] so whole values
//! serialize as `15`, not `15.0`.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// serde helpers for JS `number` fields held as `f64`: whole values are
/// written as integers (`15`), matching `JSON.stringify`.
pub mod js_number {
    use super::{Deserialize, Deserializer, Serializer};

    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE {
            serializer.serialize_i64(*value as i64)
        } else {
            serializer.serialize_f64(*value)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        f64::deserialize(deserializer)
    }

    /// For `Option<f64>` fields (pair with `skip_serializing_if`).
    pub mod option {
        use super::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(
            value: &Option<f64>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(value) => super::serialize(value, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<f64>, D::Error> {
            Option::<f64>::deserialize(deserializer)
        }
    }
}

/// The TS loader casts a PTY fixture without checking its fields, and the
/// values then go through JS coercion: node-pty stringifies env values and
/// timers truncate fractional delays. These accept the same input.
mod js_coerced {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    /// `Record<string, string>` whose values may be numbers or booleans.
    pub fn env<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<BTreeMap<String, String>>, D::Error> {
        let Some(raw) = Option::<BTreeMap<String, Value>>::deserialize(deserializer)? else {
            return Ok(None);
        };
        raw.into_iter()
            .map(|(key, value)| match value {
                Value::String(text) => Ok((key, text)),
                Value::Number(number) => Ok((key, number.to_string())),
                Value::Bool(flag) => Ok((key, flag.to_string())),
                other => Err(D::Error::custom(format!(
                    "env.{key}: expected a string, number or boolean, got {other}"
                ))),
            })
            .collect::<Result<_, _>>()
            .map(Some)
    }

    /// A delay in milliseconds: fractions are dropped, negatives are 0.
    pub fn millis<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
        Ok(Option::<f64>::deserialize(deserializer)?.map(|ms| ms.max(0.0) as u64))
    }
}

/// The literal `1` of `version: 1`; any other value fails to deserialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Version1;

impl Serialize for Version1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}

impl<'de> Deserialize<'de> for Version1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u8::deserialize(deserializer)? {
            1 => Ok(Self),
            other => Err(serde::de::Error::custom(format!(
                "expected version 1, got {other}"
            ))),
        }
    }
}

/// The literal `true` of `waitForExit: true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct True;

impl Serialize for True {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for True {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("expected true"))
        }
    }
}

/// The literal `"frames"` of `source: "frames"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FramesSource;

impl Serialize for FramesSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("frames")
    }
}

impl<'de> Deserialize<'de> for FramesSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value == "frames" {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected \"frames\", got {value:?}"
            )))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovieSourceKind {
    #[serde(rename = "frames")]
    Frames,
    #[serde(rename = "browser")]
    Browser,
    #[serde(rename = "pty")]
    Pty,
    #[serde(rename = "desktop.window")]
    DesktopWindow,
    #[serde(rename = "desktop.display")]
    DesktopDisplay,
    #[serde(rename = "desktop.region")]
    DesktopRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MovieFormat {
    Webm,
    Mp4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ManifestStatus {
    Running,
    Pass,
    Fail,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieChapter {
    pub slug: String,
    #[serde(with = "js_number")]
    pub t_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieArtifact {
    pub video_path: String,
    pub poster_path: String,
    #[serde(with = "js_number")]
    pub duration_ms: f64,
    pub chapters: Vec<MovieChapter>,
    pub source: MovieSourceKind,
    pub feature: String,
    pub slug: String,
    pub sequence: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieSessionOptions {
    /// Feature directory under .astroshot/ (kebab-case).
    pub feature: String,
    /// Filename slug for this movie.
    pub slug: String,
    /// Worktree / project root that contains (or will contain) .astroshot/.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Stable run id; defaults to feature-timestamp-pid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    /// Target encode frame rate. Default 15.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub fps: Option<f64>,
    /// Output container. Default webm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<MovieFormat>,
    /// Manifest status while recording. Default running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ManifestStatus>,
    pub source: MovieSourceKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeFramesRequest {
    pub frame_paths: Vec<String>,
    pub out_path: String,
    pub size: Size,
    #[serde(with = "js_number")]
    pub fps: f64,
    /// Wall-clock duration override when frame timestamps are irregular.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SinkMovieRequest {
    pub root: String,
    pub feature: String,
    pub slug: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ManifestStatus>,
    pub source: MovieSourceKind,
    pub poster_path: String,
    pub video_path: String,
    #[serde(with = "js_number")]
    pub duration_ms: f64,
    pub chapters: Vec<MovieChapter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SinkMovieResult {
    pub sequence: String,
    pub poster_dest: String,
    pub video_dest: String,
    pub manifest_path: String,
    pub feature_dir: String,
}

/// On-disk state for multi-process frames CLI (start / push / mark / stop).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedFrameSession {
    pub version: Version1,
    pub id: String,
    pub feature: String,
    pub slug: String,
    pub root: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub size: Size,
    #[serde(with = "js_number")]
    pub fps: f64,
    pub format: MovieFormat,
    pub source: FramesSource,
    pub started_at_ms: u64,
    pub frame_dir: String,
    pub frame_count: u64,
    pub chapters: Vec<MovieChapter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// One step of a PTY fixture; the TS union is untagged (shape decides).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PtyAction {
    #[serde(rename_all = "camelCase")]
    WaitFor {
        wait_for: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    WaitForExit {
        wait_for_exit: True,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Key {
        key: PtyKey,
    },
    Text {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Pause {
        pause_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyMovieFixture {
    pub version: Version1,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// TS `Record<string, string>`; sorted by key here (fixtures are read-only input).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "js_coerced::env"
    )]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "js_coerced::millis"
    )]
    pub timeout_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "js_coerced::millis"
    )]
    pub settle_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_non_zero_exit: Option<bool>,
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub font_size: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub line_height: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub padding: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub border_radius: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub scale: Option<f64>,
    /// Sample rate for the movie. Defaults to session fps.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "js_number::option"
    )]
    pub movie_fps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMovieOptions {
    /// Navigate here when the harness owns the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Optional script module that exports `default` async (page) => void.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    /// Reuse an existing headed browser (debug). Default headless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headed: Option<bool>,
    /// How long to keep recording after script ends (ms). Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_frame_session_json_matches_ts_key_order_and_numbers() {
        let session = PersistedFrameSession {
            version: Version1,
            id: "abc".into(),
            feature: "f".into(),
            slug: "s".into(),
            root: "/r".into(),
            run_id: "f-1".into(),
            title: None,
            description: Some("d".into()),
            size: Size {
                width: 1280,
                height: 720,
            },
            fps: 15.0,
            format: MovieFormat::Webm,
            source: FramesSource,
            started_at_ms: 1_700_000_000_000,
            frame_dir: "/r/.astroshot/f/.movie/frames".into(),
            frame_count: 2,
            chapters: vec![MovieChapter {
                slug: "a".into(),
                t_ms: 12.0,
                note: None,
            }],
        };
        let json = serde_json::to_string(&session).unwrap();
        assert_eq!(
            json,
            r#"{"version":1,"id":"abc","feature":"f","slug":"s","root":"/r","runId":"f-1","description":"d","size":{"width":1280,"height":720},"fps":15,"format":"webm","source":"frames","startedAtMs":1700000000000,"frameDir":"/r/.astroshot/f/.movie/frames","frameCount":2,"chapters":[{"slug":"a","tMs":12}]}"#
        );
        let back: PersistedFrameSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back, session);
    }

    #[test]
    fn fractional_numbers_keep_their_fraction() {
        let chapter = MovieChapter {
            slug: "a".into(),
            t_ms: 12.5,
            note: Some("n".into()),
        };
        assert_eq!(
            serde_json::to_string(&chapter).unwrap(),
            r#"{"slug":"a","tMs":12.5,"note":"n"}"#
        );
    }

    #[test]
    fn source_kinds_use_dotted_wire_names() {
        for (kind, wire) in [
            (MovieSourceKind::DesktopWindow, "\"desktop.window\""),
            (MovieSourceKind::DesktopDisplay, "\"desktop.display\""),
            (MovieSourceKind::DesktopRegion, "\"desktop.region\""),
            (MovieSourceKind::Frames, "\"frames\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), wire);
        }
    }

    #[test]
    fn version_must_be_one() {
        assert!(serde_json::from_str::<Version1>("2").is_err());
        assert!(serde_json::from_str::<Version1>("1").is_ok());
    }

    #[test]
    fn pty_fixture_parses_untagged_actions_and_keys() {
        let json = r#"{"version":1,"command":"tui","actions":[
            {"waitFor":"ready","timeoutMs":500},
            {"waitForExit":true},
            {"key":"ctrl-c"},
            {"text":"hi"},
            {"pauseMs":250}],"cols":80,"movieFps":10}"#;
        let fixture: PtyMovieFixture = serde_json::from_str(json).unwrap();
        assert_eq!(
            fixture.actions.as_ref().unwrap(),
            &vec![
                PtyAction::WaitFor {
                    wait_for: "ready".into(),
                    timeout_ms: Some(500)
                },
                PtyAction::WaitForExit {
                    wait_for_exit: True,
                    timeout_ms: None
                },
                PtyAction::Key { key: PtyKey::CtrlC },
                PtyAction::Text { text: "hi".into() },
                PtyAction::Pause { pause_ms: 250 },
            ]
        );
        assert_eq!(fixture.movie_fps, Some(10.0));
        let out = serde_json::to_string(&fixture.actions).unwrap();
        assert_eq!(
            out,
            r#"[{"waitFor":"ready","timeoutMs":500},{"waitForExit":true},{"key":"ctrl-c"},{"text":"hi"},{"pauseMs":250}]"#
        );
    }

    #[test]
    fn session_options_omit_absent_fields() {
        let options = MovieSessionOptions {
            feature: "f".into(),
            slug: "s".into(),
            root: None,
            run_id: None,
            title: None,
            description: None,
            size: None,
            fps: None,
            format: Some(MovieFormat::Mp4),
            status: Some(ManifestStatus::Running),
            source: MovieSourceKind::Pty,
        };
        assert_eq!(
            serde_json::to_string(&options).unwrap(),
            r#"{"feature":"f","slug":"s","format":"mp4","status":"running","source":"pty"}"#
        );
    }
}
