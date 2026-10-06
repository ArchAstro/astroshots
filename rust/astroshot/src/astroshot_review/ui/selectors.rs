//! Port of `packages/astroshot-review/src/ui/selectors.ts`.
//!
//! Pure selection logic over the review data model. Results borrow from the
//! input slices (TS returned the same object references).

use std::cmp::Ordering;

use crate::astroshot_review::data::model::{FrictionLog, FrictionRun, ReviewState, Shot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFilter {
    Unseen,
    History,
}

pub fn review_state_of(shot: &Shot) -> ReviewState {
    shot.review
        .as_ref()
        .map_or(ReviewState::Pending, |review| review.state)
}

fn pool(shots: &[Shot], movies_only: bool) -> impl Iterator<Item = &Shot> {
    shots
        .iter()
        .filter(move |shot| !movies_only || shot.is_movie)
}

pub fn filter_shots(shots: &[Shot], filter: StreamFilter, movies_only: bool) -> Vec<&Shot> {
    pool(shots, movies_only)
        .filter(|shot| match filter {
            StreamFilter::Unseen => review_state_of(shot) != ReviewState::Seen,
            StreamFilter::History => review_state_of(shot) == ReviewState::Seen,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamCounts {
    pub pending: usize,
    pub seen: usize,
    pub movies: usize,
}

pub fn stream_counts(shots: &[Shot], movies_only: bool) -> StreamCounts {
    StreamCounts {
        pending: pool(shots, movies_only)
            .filter(|shot| review_state_of(shot) != ReviewState::Seen)
            .count(),
        seen: pool(shots, movies_only)
            .filter(|shot| review_state_of(shot) == ReviewState::Seen)
            .count(),
        movies: shots.iter().filter(|shot| shot.is_movie).count(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamGroup<'a> {
    /// Path of the last shot in the group: a stable anchor.
    pub id: &'a str,
    pub worktree_path: &'a str,
    pub worktree: &'a str,
    pub worktree_short: &'a str,
    pub shots: Vec<&'a Shot>,
}

/// Contiguous runs of the same worktree, exactly like the app's stream.
pub fn contiguous_groups(shots: &[Shot]) -> Vec<StreamGroup<'_>> {
    let mut groups: Vec<StreamGroup<'_>> = Vec::new();
    for shot in shots {
        match groups.last_mut() {
            Some(last) if last.worktree_path == shot.worktree_path => {
                last.shots.push(shot);
                last.id = &shot.path;
            }
            _ => groups.push(StreamGroup {
                id: &shot.path,
                worktree_path: &shot.worktree_path,
                worktree: &shot.worktree,
                worktree_short: &shot.worktree_short,
                shots: vec![shot],
            }),
        }
    }
    groups
}

fn sibling_order(a: &Shot, b: &Shot) -> Ordering {
    match (a.sequence.as_deref(), b.sequence.as_deref()) {
        (Some(x), Some(y)) if !x.is_empty() && !y.is_empty() && x != y => x.cmp(y),
        _ => a
            .captured_at
            .partial_cmp(&b.captured_at)
            .unwrap_or(Ordering::Equal),
    }
}

/// Same worktree, feature, and run; oldest first by sequence, else capture time.
pub fn review_siblings<'a>(all: &'a [Shot], shot: &Shot) -> Vec<&'a Shot> {
    let mut siblings: Vec<&Shot> = all
        .iter()
        .filter(|candidate| {
            candidate.worktree_path == shot.worktree_path
                && candidate.feature == shot.feature
                && candidate.run_id == shot.run_id
        })
        .collect();
    // The comparator mixes sequence and time, so it is not a total order for
    // arbitrary data. `sort_by` may panic on that; a stable insertion sort
    // (like `Array#sort`) never does.
    for i in 1..siblings.len() {
        let mut j = i;
        while j > 0 && sibling_order(siblings[j - 1], siblings[j]) == Ordering::Greater {
            siblings.swap(j - 1, j);
            j -= 1;
        }
    }
    siblings
}

pub fn latest_run(log: &FrictionLog) -> Option<&FrictionRun> {
    log.runs.first()
}

pub fn friction_state(log: &FrictionLog) -> ReviewState {
    latest_run(log)
        .and_then(|run| run.review.as_ref())
        .map_or(ReviewState::Pending, |review| review.state)
}

pub fn filter_friction_logs(logs: &[FrictionLog], filter: StreamFilter) -> Vec<&FrictionLog> {
    logs.iter()
        .filter(|log| match filter {
            StreamFilter::Unseen => friction_state(log) != ReviewState::Seen,
            StreamFilter::History => friction_state(log) == ReviewState::Seen,
        })
        .collect()
}

pub fn friction_summary(log: &FrictionLog) -> String {
    let run = latest_run(log);
    let steps = run.map_or(0, |run| run.steps.len());
    let improve: usize = run.map_or(0, |run| {
        run.steps.iter().map(|step| step.improve.len()).sum()
    });
    let mut parts: Vec<String> = Vec::new();
    if steps > 0 {
        parts.push(if steps == 1 {
            "1 step".to_string()
        } else {
            format!("{steps} steps")
        });
    }
    if improve > 0 {
        parts.push(format!("{improve} improve"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astroshot_review::data::model::{FrictionStep, ReviewSnapshot};

    fn shot(path: &str, worktree_path: &str, f: impl FnOnce(&mut Shot)) -> Shot {
        let mut shot = Shot {
            id: path.into(),
            path: path.into(),
            file_name: "0001-a.png".into(),
            worktree_path: worktree_path.into(),
            worktree: "w".into(),
            worktree_short: "w".into(),
            feature: "f".into(),
            feature_dir: "/w/.astroshot/f".into(),
            sequence: Some("0001".into()),
            slug: "a".into(),
            title: "A".into(),
            description: String::new(),
            url: None,
            run_id: Some("r".into()),
            status: None,
            captured_at: 0.0,
            mtime_ms: 0.0,
            is_movie: false,
            video_file_name: None,
            video_path: None,
            duration_ms: None,
            source: None,
            chapters: vec![],
            review: None,
        };
        f(&mut shot);
        shot
    }

    fn seen() -> ReviewSnapshot {
        ReviewSnapshot {
            state: ReviewState::Seen,
            decision: Some("seen".into()),
            hash_matches: true,
            is_stale: false,
            comments: vec![],
            reviewed_at: None,
        }
    }

    fn shots() -> Vec<Shot> {
        vec![
            shot("/w1/1", "/w1", |s| {
                s.sequence = Some("0002".into());
                s.captured_at = 4.0;
                s.is_movie = true;
            }),
            shot("/w2/1", "/w2", |s| s.captured_at = 3.0),
            shot("/w1/2", "/w1", |s| {
                s.captured_at = 2.0;
                s.review = Some(seen());
            }),
            shot("/w1/3", "/w1", |s| {
                s.feature = "g".into();
                s.captured_at = 1.0;
            }),
        ]
    }

    fn paths<'a>(shots: &[&'a Shot]) -> Vec<&'a str> {
        shots.iter().map(|shot| shot.path.as_str()).collect()
    }

    #[test]
    fn groups_contiguously_not_by_dictionary() {
        let shots = shots();
        let groups = contiguous_groups(&shots);
        let summary: Vec<_> = groups
            .iter()
            .map(|group| (group.worktree_path, group.shots.len(), group.id))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("/w1", 1, "/w1/1"),
                ("/w2", 1, "/w2/1"),
                ("/w1", 2, "/w1/3")
            ]
        );
    }

    #[test]
    fn filters_unseen_history_and_movies() {
        let shots = shots();
        assert_eq!(
            paths(&filter_shots(&shots, StreamFilter::Unseen, false)),
            ["/w1/1", "/w2/1", "/w1/3"]
        );
        assert_eq!(
            paths(&filter_shots(&shots, StreamFilter::History, false)),
            ["/w1/2"]
        );
        assert_eq!(
            paths(&filter_shots(&shots, StreamFilter::Unseen, true)),
            ["/w1/1"]
        );
        assert_eq!(
            stream_counts(&shots, false),
            StreamCounts {
                pending: 3,
                seen: 1,
                movies: 1
            }
        );
        assert_eq!(
            stream_counts(&shots, true),
            StreamCounts {
                pending: 1,
                seen: 0,
                movies: 1
            }
        );
    }

    #[test]
    fn orders_review_siblings_oldest_first_within_worktree_feature_and_run() {
        let shots = shots();
        assert_eq!(
            paths(&review_siblings(&shots, &shots[0])),
            ["/w1/2", "/w1/1"]
        );
    }

    #[test]
    fn siblings_fall_back_to_capture_time_without_distinct_sequences() {
        let shots = vec![
            shot("/a", "/w", |s| {
                s.sequence = None;
                s.captured_at = 9.0;
            }),
            shot("/b", "/w", |s| {
                s.sequence = Some("0001".into());
                s.captured_at = 5.0;
            }),
            shot("/c", "/w", |s| {
                s.sequence = Some("0001".into());
                s.captured_at = 1.0;
            }),
            shot("/other-run", "/w", |s| s.run_id = None),
        ];
        assert_eq!(
            paths(&review_siblings(&shots, &shots[0])),
            ["/c", "/b", "/a"]
        );
    }

    fn log(runs: Vec<FrictionRun>) -> FrictionLog {
        FrictionLog {
            id: "/w::l".into(),
            slug: "l".into(),
            directory: "/w/l".into(),
            worktree_path: "/w".into(),
            worktree: "w".into(),
            worktree_short: "w".into(),
            title: "L".into(),
            description: String::new(),
            status: None,
            updated_at: 0.0,
            prompt_path: None,
            runs,
        }
    }

    fn step(improve: usize) -> FrictionStep {
        FrictionStep {
            id: "s".into(),
            step: 1.0,
            step_id: "s".into(),
            title: "S".into(),
            description: String::new(),
            transcript: String::new(),
            screenshots: vec![],
            good: vec![],
            improve: vec!["x".into(); improve],
            url: None,
            captured_at: None,
        }
    }

    fn run(steps: Vec<FrictionStep>, review: Option<ReviewSnapshot>) -> FrictionRun {
        FrictionRun {
            run_id: "r".into(),
            directory: "/w/l/r".into(),
            log_path: None,
            captured_at: 0.0,
            status: None,
            steps,
            review,
        }
    }

    #[test]
    fn friction_state_filter_and_summary_use_the_latest_run() {
        let logs = vec![
            log(vec![run(vec![step(0)], Some(seen())), run(vec![], None)]),
            log(vec![run(vec![step(2), step(1)], None)]),
            log(vec![]),
        ];
        assert_eq!(friction_state(&logs[0]), ReviewState::Seen);
        assert_eq!(friction_state(&logs[2]), ReviewState::Pending);
        assert_eq!(filter_friction_logs(&logs, StreamFilter::Unseen).len(), 2);
        assert_eq!(filter_friction_logs(&logs, StreamFilter::History).len(), 1);
        assert_eq!(friction_summary(&logs[0]), "1 step");
        assert_eq!(friction_summary(&logs[1]), "2 steps · 3 improve");
        assert_eq!(friction_summary(&logs[2]), "");
        assert_eq!(latest_run(&logs[2]), None);
    }
}
