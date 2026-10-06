//! Port of `packages/astroshot-review/src/data/model.ts`.
//!
//! These types describe data read from the on-disk `.astroshot/` contract.
//! Field names are the TS camelCase names and fields are declared in the TS
//! order, so serialized JSON matches what the TS writes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeatureStatus {
    Running,
    Pass,
    Fail,
    Idle,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewState {
    Seen,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub state: ReviewState,
    /// Raw decision from disk, before hash scoping.
    pub decision: Option<String>,
    pub hash_matches: bool,
    /// A decision exists but the bytes changed since it was recorded.
    pub is_stale: bool,
    pub comments: Vec<ReviewComment>,
    pub reviewed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    /// Absolute image path; stable identity.
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub worktree_path: String,
    pub worktree: String,
    pub worktree_short: String,
    pub feature: String,
    pub feature_dir: String,
    pub sequence: Option<String>,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub url: Option<String>,
    pub run_id: Option<String>,
    pub status: Option<FeatureStatus>,
    /// Epoch milliseconds.
    pub captured_at: f64,
    pub mtime_ms: f64,
    pub is_movie: bool,
    pub video_file_name: Option<String>,
    /// Absolute path when the video exists on disk.
    pub video_path: Option<String>,
    pub duration_ms: Option<f64>,
    pub source: Option<String>,
    pub chapters: Vec<Chapter>,
    pub review: Option<ReviewSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionStep {
    pub id: String,
    pub step: i64,
    pub step_id: String,
    pub title: String,
    pub description: String,
    pub transcript: String,
    /// Absolute paths that exist on disk.
    pub screenshots: Vec<String>,
    pub good: Vec<String>,
    pub improve: Vec<String>,
    pub url: Option<String>,
    pub captured_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionRun {
    pub run_id: String,
    pub directory: String,
    pub log_path: Option<String>,
    pub captured_at: f64,
    pub status: Option<String>,
    pub steps: Vec<FrictionStep>,
    pub review: Option<ReviewSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrictionLog {
    /// `<worktreePath>::<slug>`
    pub id: String,
    pub slug: String,
    pub directory: String,
    pub worktree_path: String,
    pub worktree: String,
    pub worktree_short: String,
    pub title: String,
    pub description: String,
    pub status: Option<String>,
    pub updated_at: f64,
    pub prompt_path: Option<String>,
    pub runs: Vec<FrictionRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstroshotTree {
    pub astroshot_dir: String,
    pub worktree_path: String,
    pub worktree: String,
    pub shots: Vec<Shot>,
    pub friction_logs: Vec<FrictionLog>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_omits_absent_fields_and_uses_t_ms() {
        let chapter: Chapter = serde_json::from_str(r#"{"title":"A","tMs":1500}"#).unwrap();
        assert_eq!(chapter.t_ms, Some(1500.0));
        assert_eq!(
            serde_json::to_string(&Chapter {
                title: Some("A".into()),
                ..Chapter::default()
            })
            .unwrap(),
            r#"{"title":"A"}"#
        );
    }

    #[test]
    fn snapshot_serializes_in_ts_field_order_with_lowercase_state() {
        let snapshot = ReviewSnapshot {
            state: ReviewState::Pending,
            decision: None,
            hash_matches: true,
            is_stale: false,
            comments: vec![],
            reviewed_at: None,
        };
        assert_eq!(
            serde_json::to_string(&snapshot).unwrap(),
            r#"{"state":"pending","decision":null,"hashMatches":true,"isStale":false,"comments":[],"reviewedAt":null}"#
        );
    }
}
