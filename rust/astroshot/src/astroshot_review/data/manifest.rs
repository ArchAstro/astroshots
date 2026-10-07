//! Port of `packages/astroshot-review/src/data/manifest.ts`.
//!
//! `manifest.json` is written by many tools, and the TS reads it with a bare
//! `as FeatureManifest` cast. Here each field is read leniently from the JSON
//! value: a field of the wrong type reads as absent, and non-object entries in
//! `shots` / `chapters` are dropped.

use std::path::Path;

use serde_json::{Map, Value};

use super::model::{Chapter, FeatureStatus};
use super::paths::sequence_and_slug;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ManifestChapter {
    pub slug: Option<String>,
    pub title: Option<String>,
    pub t_ms: Option<f64>,
    /// The `tMs` spelling.
    pub t_ms_camel: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ManifestShot {
    pub id: Option<String>,
    pub file: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub captured_at: Option<String>,
    pub url: Option<String>,
    pub viewport: Option<Value>,
    pub kind: Option<String>,
    pub video: Option<String>,
    pub poster: Option<String>,
    pub duration_ms: Option<f64>,
    pub source: Option<String>,
    pub chapters: Option<Vec<ManifestChapter>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureManifest {
    pub version: Option<f64>,
    pub feature: Option<String>,
    pub run_id: Option<String>,
    pub status: Option<String>,
    pub description: Option<String>,
    pub shots: Option<Vec<ManifestShot>>,
}

fn string_field(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number_field(map: &Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(Value::as_f64)
}

impl ManifestChapter {
    fn from_map(map: &Map<String, Value>) -> Self {
        Self {
            slug: string_field(map, "slug"),
            title: string_field(map, "title"),
            t_ms: number_field(map, "t_ms"),
            t_ms_camel: number_field(map, "tMs"),
        }
    }
}

impl ManifestShot {
    fn from_map(map: &Map<String, Value>) -> Self {
        Self {
            id: string_field(map, "id"),
            file: string_field(map, "file"),
            slug: string_field(map, "slug"),
            title: string_field(map, "title"),
            description: string_field(map, "description"),
            captured_at: string_field(map, "captured_at"),
            url: string_field(map, "url"),
            viewport: map.get("viewport").cloned(),
            kind: string_field(map, "kind"),
            video: string_field(map, "video"),
            poster: string_field(map, "poster"),
            duration_ms: number_field(map, "duration_ms"),
            source: string_field(map, "source"),
            chapters: match map.get("chapters") {
                Some(Value::Array(items)) => Some(
                    items
                        .iter()
                        .filter_map(Value::as_object)
                        .map(ManifestChapter::from_map)
                        .collect(),
                ),
                _ => None,
            },
        }
    }
}

impl FeatureManifest {
    fn from_map(map: &Map<String, Value>) -> Self {
        Self {
            version: number_field(map, "version"),
            feature: string_field(map, "feature"),
            run_id: string_field(map, "run_id"),
            status: string_field(map, "status"),
            description: string_field(map, "description"),
            // A present but non-array `shots` becomes `[]`.
            shots: match map.get("shots") {
                None => None,
                Some(Value::Array(items)) => Some(
                    items
                        .iter()
                        .filter_map(Value::as_object)
                        .map(ManifestShot::from_map)
                        .collect(),
                ),
                Some(_) => Some(Vec::new()),
            },
        }
    }
}

pub async fn read_manifest(feature_dir: &str) -> Option<FeatureManifest> {
    let bytes = tokio::fs::read(Path::new(feature_dir).join("manifest.json"))
        .await
        .ok()?;
    parse_manifest(&String::from_utf8_lossy(&bytes))
}

/// The JSON half of `readManifest`: `None` unless the text is a JSON object.
pub fn parse_manifest(raw: &str) -> Option<FeatureManifest> {
    match serde_json::from_str::<Value>(raw).ok()? {
        Value::Object(map) => Some(FeatureManifest::from_map(&map)),
        _ => None,
    }
}

/// First match wins: file, poster, id (sequence), slug.
pub fn match_manifest_shot<'a>(
    manifest: Option<&'a FeatureManifest>,
    file_name: &str,
) -> Option<&'a ManifestShot> {
    let shots = manifest?.shots.as_deref().unwrap_or(&[]);
    if shots.is_empty() {
        return None;
    }
    let parts = sequence_and_slug(file_name);
    shots
        .iter()
        .find(|shot| shot.file.as_deref() == Some(file_name))
        .or_else(|| {
            shots
                .iter()
                .find(|shot| shot.poster.as_deref() == Some(file_name))
        })
        .or_else(|| {
            parts.sequence.as_deref().and_then(|sequence| {
                shots
                    .iter()
                    .find(|shot| shot.id.as_deref() == Some(sequence))
            })
        })
        .or_else(|| {
            shots
                .iter()
                .find(|shot| shot.slug.as_deref() == Some(parts.slug.as_str()))
        })
}

pub fn parse_feature_status(raw: Option<&str>) -> Option<FeatureStatus> {
    match raw.unwrap_or("").to_lowercase().as_str() {
        "running" | "run" | "in_progress" | "in-progress" => Some(FeatureStatus::Running),
        "pass" | "passed" | "ok" | "success" => Some(FeatureStatus::Pass),
        "fail" | "failed" | "error" => Some(FeatureStatus::Fail),
        "idle" | "pending" => Some(FeatureStatus::Idle),
        _ => None,
    }
}

/// `Date.parse(value)` as epoch milliseconds, or `None` for empty / NaN.
///
/// Handles the ECMAScript date-time format (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`,
/// optional `THH:mm[:ss[.sss]]` and `Z` / `+HH:mm`; date-only forms are UTC,
/// date-times without an offset are local time) plus RFC 2822. V8's other
/// legacy formats are not accepted.
pub fn parse_iso_date(value: Option<&str>) -> Option<f64> {
    let value = value.filter(|value| !value.is_empty())?;
    parse_es_date(value.trim()).or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value.trim())
            .ok()
            .map(|date| date.timestamp_millis() as f64)
    })
}

fn parse_es_date(text: &str) -> Option<f64> {
    use chrono::{Local, NaiveDate, TimeZone};
    use regex::Regex;
    use std::sync::OnceLock;

    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r"^([+-]\d{6}|\d{4})(?:-(\d{2})(?:-(\d{2}))?)?(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:[.,](\d+))?)?\s*(Z|[+-]\d{2}(?::?\d{2})?)?)?$",
        )
        .expect("date pattern compiles")
    });
    let captures = pattern.captures(text)?;
    let number = |index: usize| -> Option<i64> {
        captures
            .get(index)
            .map(|found| found.as_str().parse::<i64>().ok())
            .unwrap_or(Some(0))
    };
    let year_text = &captures[1];
    if year_text == "-000000" {
        return None;
    }
    let year = year_text.parse::<i64>().ok()?;
    let month = captures.get(2).map_or(Some(1), |_| number(2))?;
    let day = captures.get(3).map_or(Some(1), |_| number(3))?;
    let (hour, minute, second) = (number(4)?, number(5)?, number(6)?);
    let millis = captures.get(7).map_or(0, |found| {
        let digits: String = found
            .as_str()
            .chars()
            .chain("000".chars())
            .take(3)
            .collect();
        digits.parse::<i64>().unwrap_or(0)
    });
    if !(1..=12).contains(&month) || !(0..=59).contains(&minute) || !(0..=59).contains(&second) {
        return None;
    }
    if hour > 24 || (hour == 24 && (minute, second, millis) != (0, 0, 0)) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month as u32, day as u32)?;
    let naive = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis()
        + ((hour * 60 + minute) * 60 + second) * 1000
        + millis;
    let has_time = captures.get(4).is_some();
    let offset = captures.get(8).map(|found| found.as_str());
    let millis_utc = match offset {
        Some("Z") => naive,
        Some(zone) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
            let zone_hours: i64 = digits.get(0..2)?.parse().ok()?;
            let zone_minutes: i64 = digits.get(2..4).map_or(Some(0), |m| m.parse().ok())?;
            naive - sign * (zone_hours * 60 + zone_minutes) * 60_000
        }
        None if has_time => {
            // Date-time without an offset is local time.
            let local = chrono::DateTime::from_timestamp_millis(naive)?.naive_utc();
            Local
                .from_local_datetime(&local)
                .earliest()?
                .timestamp_millis()
        }
        None => naive,
    };
    Some(millis_utc as f64)
}

pub fn chapters_of(entry: Option<&ManifestShot>) -> Vec<Chapter> {
    let Some(chapters) = entry.and_then(|entry| entry.chapters.as_ref()) else {
        return Vec::new();
    };
    chapters
        .iter()
        .map(|chapter| Chapter {
            slug: chapter.slug.clone(),
            title: chapter.title.clone(),
            t_ms: chapter.t_ms.or(chapter.t_ms_camel),
        })
        .collect()
}

/// `Math.round`: halves round toward +Infinity.
fn js_round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `Number.prototype.toFixed(1)`: exact ties pick the larger multiple (Rust's
/// formatter rounds ties to even).
fn to_fixed_1(value: f64) -> String {
    let quarters = value * 4.0;
    if value >= 0.0 && quarters.fract() == 0.0 && (quarters as i64) % 2 != 0 {
        let tenths = (value * 10.0).ceil() as i64;
        return format!("{}.{}", tenths / 10, tenths % 10);
    }
    format!("{value:.1}")
}

fn minutes_label(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor();
    let rest = (seconds % 60.0).floor();
    format!("{minutes}:{rest:02}")
}

/// `2.8s` under a minute, `m:ss` above; `None` when unknown.
pub fn duration_label(duration_ms: Option<f64>) -> Option<String> {
    let duration_ms = duration_ms.filter(|duration| *duration > 0.0)?;
    let seconds = duration_ms / 1000.0;
    if seconds < 60.0 {
        let rounded = js_round(seconds * 10.0) / 10.0;
        return Some(if rounded.fract() == 0.0 {
            format!("{rounded}s")
        } else {
            format!("{}s", to_fixed_1(rounded))
        });
    }
    Some(minutes_label(seconds))
}

/// Chapter timestamp label used by the detail card.
pub fn chapter_time_label(t_ms: Option<f64>) -> String {
    let Some(t_ms) = t_ms.filter(|t_ms| *t_ms >= 0.0) else {
        return "\u{2014}".to_string();
    };
    let seconds = t_ms / 1000.0;
    if seconds < 60.0 {
        return format!("{}s", to_fixed_1(seconds));
    }
    minutes_label(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> FeatureManifest {
        parse_manifest(
            r#"{"shots":[
              {"id":"0001","file":"0001-a.png","slug":"a"},
              {"id":"0002","poster":"0002-b.png","slug":"b","kind":"movie","video":"0002-b.webm"},
              {"id":"0003","slug":"configure"}]}"#,
        )
        .unwrap()
    }

    fn slug_of(shot: Option<&ManifestShot>) -> Option<&str> {
        shot.and_then(|shot| shot.slug.as_deref())
    }

    #[test]
    fn matches_by_file_poster_id_then_slug() {
        let manifest = manifest();
        let find = |name| slug_of(match_manifest_shot(Some(&manifest), name));
        assert_eq!(find("0001-a.png"), Some("a"));
        assert_eq!(find("0002-b.png"), Some("b"));
        assert_eq!(find("0003-anything.png"), Some("configure"));
        assert_eq!(find("configure.png"), Some("configure"));
        assert_eq!(find("0009-zzz.png"), None);
        assert!(match_manifest_shot(None, "x.png").is_none());
    }

    #[test]
    fn maps_execution_status_aliases() {
        assert_eq!(
            parse_feature_status(Some("in-progress")),
            Some(FeatureStatus::Running)
        );
        assert_eq!(parse_feature_status(Some("ok")), Some(FeatureStatus::Pass));
        assert_eq!(
            parse_feature_status(Some("error")),
            Some(FeatureStatus::Fail)
        );
        assert_eq!(
            parse_feature_status(Some("pending")),
            Some(FeatureStatus::Idle)
        );
        assert_eq!(parse_feature_status(Some("weird")), None);
        assert_eq!(parse_feature_status(None), None);
    }

    #[test]
    fn formats_durations_and_chapter_times() {
        assert_eq!(duration_label(Some(2800.0)).as_deref(), Some("2.8s"));
        assert_eq!(duration_label(Some(3000.0)).as_deref(), Some("3s"));
        assert_eq!(duration_label(Some(75_000.0)).as_deref(), Some("1:15"));
        assert_eq!(duration_label(Some(0.0)), None);
        assert_eq!(duration_label(None), None);
        assert_eq!(chapter_time_label(Some(900.0)), "0.9s");
        assert_eq!(chapter_time_label(Some(61_000.0)), "1:01");
        assert_eq!(chapter_time_label(None), "\u{2014}");
    }

    #[test]
    fn accepts_camel_case_chapter_times() {
        let shot = ManifestShot {
            chapters: Some(vec![
                ManifestChapter {
                    slug: Some("a".into()),
                    t_ms_camel: Some(5.0),
                    ..ManifestChapter::default()
                },
                ManifestChapter {
                    slug: Some("b".into()),
                    t_ms: Some(7.0),
                    ..ManifestChapter::default()
                },
            ]),
            ..ManifestShot::default()
        };
        assert_eq!(
            chapters_of(Some(&shot)),
            vec![
                Chapter {
                    slug: Some("a".into()),
                    title: None,
                    t_ms: Some(5.0)
                },
                Chapter {
                    slug: Some("b".into()),
                    title: None,
                    t_ms: Some(7.0)
                },
            ]
        );
    }

    // Beyond the TS cases: the helpers the TS tests do not reach.

    #[test]
    fn tofixed_ties_round_up_like_js() {
        // (0.25).toFixed(1) === "0.3"; Rust's formatter would print "0.2".
        assert_eq!(chapter_time_label(Some(250.0)), "0.3s");
        // (0.15).toFixed(1) === "0.1": 0.15 is just below the tie.
        assert_eq!(chapter_time_label(Some(150.0)), "0.1s");
        assert_eq!(chapter_time_label(Some(1250.0)), "1.3s");
    }

    #[test]
    fn parse_manifest_is_lenient_like_the_ts_cast() {
        assert!(parse_manifest("[]").is_none());
        assert!(parse_manifest("{ nope").is_none());
        assert_eq!(
            parse_manifest(r#"{"shots":"x"}"#).unwrap().shots,
            Some(vec![])
        );
        assert_eq!(parse_manifest("{}").unwrap().shots, None);
        let manifest =
            parse_manifest(r#"{"shots":[null,{"slug":"a","chapters":[1,{"tMs":2}]}]}"#).unwrap();
        assert_eq!(manifest.shots.as_ref().unwrap().len(), 1);
        assert_eq!(
            chapters_of(manifest.shots.as_ref().unwrap().first()),
            vec![Chapter {
                t_ms: Some(2.0),
                ..Chapter::default()
            }]
        );
    }

    #[test]
    fn parse_iso_date_matches_date_parse() {
        assert_eq!(
            parse_iso_date(Some("2026-09-05T17:42:00Z")),
            Some(1788630120000.0)
        );
        assert_eq!(
            parse_iso_date(Some("2026-09-05T17:42:00.123Z")),
            Some(1788630120123.0)
        );
        assert_eq!(
            parse_iso_date(Some("2026-09-05T19:42:00+02:00")),
            Some(1788630120000.0)
        );
        assert_eq!(parse_iso_date(Some("2026-09-05")), Some(1788566400000.0));
        assert_eq!(parse_iso_date(Some("2026-13-05")), None);
        assert_eq!(parse_iso_date(Some("garbage")), None);
        assert_eq!(parse_iso_date(Some("")), None);
        assert_eq!(parse_iso_date(None), None);
    }
}
