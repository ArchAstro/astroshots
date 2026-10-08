//! Port of `packages/astroshot-review/src/ui/theme.ts`.
//!
//! Color tokens mirroring the macOS app's palette, as ratatui colors.

use chrono::{DateTime, Local, SecondsFormat, TimeZone, Utc};
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub brand: Color,
    pub amber: Color,
    pub blue: Color,
    pub green: Color,
    pub purple: Color,
    pub red: Color,
    pub muted: Color,
    pub faint: Color,
    pub text: Color,
    pub surface: Color,
    pub stage: Color,
    pub selection: Color,
}

pub const THEME: Theme = Theme {
    brand: Color::Rgb(0x54, 0xe0, 0xa0),
    amber: Color::Rgb(0xf0, 0xb8, 0x6e),
    blue: Color::Rgb(0x7a, 0xa2, 0xf7),
    green: Color::Rgb(0x54, 0xe0, 0xa0),
    purple: Color::Rgb(0xb9, 0xa8, 0xff),
    red: Color::Rgb(0xf0, 0x72, 0x7a),
    muted: Color::Rgb(0x8a, 0x8a, 0x9a),
    faint: Color::Rgb(0x5a, 0x5a, 0x6a),
    text: Color::Rgb(0xe8, 0xe8, 0xf2),
    surface: Color::Rgb(0x1c, 0x1b, 0x19),
    stage: Color::Rgb(0x2a, 0x2a, 0x29),
    selection: Color::Rgb(0x2e, 0x31, 0x40),
};

pub fn theme() -> &'static Theme {
    &THEME
}

pub fn now_ms() -> f64 {
    Utc::now().timestamp_millis() as f64
}

/// `relativeTime(fromMs, nowMs = Date.now())`: pass `now_ms()` for the default.
pub fn relative_time(from_ms: f64, now_ms: f64) -> String {
    // Values here are non-negative, where JS Math.round and f64::round agree.
    let seconds = ((now_ms - from_ms) / 1000.0).round().max(0.0);
    if seconds < 45.0 {
        return "just now".to_string();
    }
    let minutes = (seconds / 60.0).round();
    if minutes < 60.0 {
        return format!("{minutes} min ago");
    }
    let hours = (minutes / 60.0).round();
    if hours < 24.0 {
        return format!("{hours} hr ago");
    }
    let days = (hours / 24.0).round();
    if days < 30.0 {
        return format!("{days} day{} ago", if days == 1.0 { "" } else { "s" });
    }
    let months = (days / 30.0).round();
    if months < 12.0 {
        return format!("{months} mo ago");
    }
    format!("{} yr ago", (months / 12.0).round())
}

fn date_from_ms(ms: f64) -> Option<DateTime<Utc>> {
    if !ms.is_finite() {
        return None;
    }
    Utc.timestamp_millis_opt(ms.trunc() as i64).single()
}

/// `HH:MM:SS` in the local timezone.
pub fn clock_time(ms: f64) -> String {
    match date_from_ms(ms) {
        Some(date) => date.with_timezone(&Local).format("%H:%M:%S").to_string(),
        // `new Date(NaN)` yields NaN fields, which pad to "NaN".
        None => "NaN:NaN:NaN".to_string(),
    }
}

/// ISO-8601 UTC without milliseconds, e.g. `2026-08-11T14:54:00Z`.
pub fn iso_date_time(ms: f64) -> String {
    let date = date_from_ms(ms).expect("iso_date_time: timestamp out of range");
    // `toISOString().replace(/\.\d{3}Z$/, "Z")`
    let full = date.to_rfc3339_opts(SecondsFormat::Millis, true);
    format!("{}Z", &full[..full.len() - 5])
}

#[derive(Debug, Clone, Copy)]
pub enum DateInput<'a> {
    Millis(f64),
    Text(&'a str),
}

fn parse_date_text(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&Utc));
    }
    // Forms JS `Date` accepts without an offset: date-only is UTC, date-time is local.
    if let Ok(date) = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S%.f",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|date| date.with_timezone(&Utc));
        }
    }
    None
}

/// `Aug 11, 2026, 2:54 PM` in the local timezone (en-US `toLocaleString`).
/// Unparseable text is returned unchanged; an invalid number gives "".
pub fn abbreviated_date_time(input: DateInput<'_>) -> String {
    let date = match input {
        DateInput::Millis(ms) => date_from_ms(ms),
        DateInput::Text(text) => parse_date_text(text),
    };
    match date {
        Some(date) => date
            .with_timezone(&Local)
            .format("%b %-d, %Y, %-I:%M %p")
            .to_string(),
        None => match input {
            DateInput::Text(text) => text.to_string(),
            DateInput::Millis(_) => String::new(),
        },
    }
}

/// JS `\s`: whitespace plus line terminators plus U+FEFF (not U+0085).
fn is_js_whitespace(c: char) -> bool {
    matches!(c, '\u{FEFF}') || (c.is_whitespace() && c != '\u{85}')
}

/// Collapse whitespace and cut to `width` characters with a trailing `…`.
/// Width counts `char`s (TS counts UTF-16 units; identical outside the BMP-astral range).
pub fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let single = text
        .split(is_js_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if single.chars().count() <= width {
        return single;
    }
    if width <= 1 {
        return "…".to_string();
    }
    let cut: String = single.chars().take(width - 1).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_buckets() {
        let now = 1_000_000_000.0;
        let ago = |s: f64| relative_time(now - s * 1000.0, now);
        assert_eq!(ago(10.0), "just now");
        assert_eq!(ago(90.0), "2 min ago");
        assert_eq!(ago(3.0 * 3600.0), "3 hr ago");
        assert_eq!(ago(86400.0 * 24.0), "24 days ago");
        assert_eq!(ago(86400.0 * 40.0), "1 mo ago");
        assert_eq!(ago(86400.0 * 800.0), "2 yr ago");
        assert_eq!(relative_time(now + 5000.0, now), "just now");
    }

    #[test]
    fn iso_and_truncate() {
        assert_eq!(iso_date_time(1_786_460_040_123.0), "2026-08-11T14:54:00Z");
        assert_eq!(truncate("  a \n b  ", 10), "a b");
        assert_eq!(truncate("abcdefgh", 4), "abc…");
        assert_eq!(truncate("abcdefgh", 1), "…");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn abbreviated_falls_back_for_invalid_input() {
        assert_eq!(abbreviated_date_time(DateInput::Text("nope")), "nope");
        assert_eq!(abbreviated_date_time(DateInput::Millis(f64::NAN)), "");
        assert!(abbreviated_date_time(DateInput::Text("2026-08-11T14:54:00Z")).contains("2026"));
    }

    #[test]
    fn theme_keeps_palette() {
        assert_eq!(theme().brand, Color::Rgb(0x54, 0xe0, 0xa0));
        assert_eq!(theme().selection, Color::Rgb(0x2e, 0x31, 0x40));
    }
}
