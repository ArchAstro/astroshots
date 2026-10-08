//! Port of `packages/astroshot-review/src/data/scan.ts`.
//!
//! Discover `.astroshot` trees under the watch roots and load their shots.
//!
//! Divergences: `AbortSignal` is an `Arc<AtomicBool>` (set it to abort);
//! `fs.Stats` is [`FileStat`]; `FeatureListing.files` is a `BTreeSet` so
//! iteration is byte-sorted like libuv's `readdir`; the `{ worktreePath,
//! worktree }` context is [`FrictionContext`] (same shape).

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fs::FileType;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::task::JoinSet;

use super::friction::{FrictionContext, load_user_stories};
use super::hash_cache::{FileStat, HashCache};
use super::manifest::{
    FeatureManifest, chapters_of, match_manifest_shot, parse_feature_status, parse_iso_date,
    read_manifest,
};
use super::model::{AstroshotTree, ReviewSnapshot, Shot};
use super::paths::{
    ASTROSHOT_DIR, MAX_SCAN_DEPTH, SKIP_DIRECTORIES, VIDEO_EXTENSIONS, basename, dirname, humanize,
    is_image_file, is_reserved_dir, join, sequence_and_slug, worktree_short,
};
use super::review_store::{
    ReviewDocument, entry_needs_hash, read_review_document, scoped_entry, snapshot_from_entry,
};

/// `findAstroshotDirs` default for `concurrency`.
pub const DEFAULT_CONCURRENCY: usize = 16;

pub type FoundCallback = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default, Clone)]
pub struct FindOptions {
    pub max_depth: Option<usize>,
    pub skip: Option<HashSet<String>>,
    pub concurrency: Option<usize>,
    pub on_found: Option<FoundCallback>,
    /// `AbortSignal`: set to `true` to stop dispatching new directories.
    pub signal: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
    Other,
}

fn kind_of_type(file_type: &FileType) -> Option<Kind> {
    if file_type.is_file() {
        Some(Kind::File)
    } else if file_type.is_dir() {
        Some(Kind::Dir)
    } else if file_type.is_symlink() {
        None
    } else {
        Some(Kind::Other)
    }
}

/// Dirent kinds with symlinks resolved, so linked trees behave like real ones.
async fn kind_of(parent: &str, name: &str, file_type: &FileType) -> Kind {
    if let Some(kind) = kind_of_type(file_type) {
        return kind;
    }
    match tokio::fs::metadata(Path::new(parent).join(name)).await {
        Ok(stat) if stat.is_file() => Kind::File,
        Ok(stat) if stat.is_dir() => Kind::Dir,
        _ => Kind::Other,
    }
}

/// `fs.promises.readdir(dir, { withFileTypes: true })`: byte-sorted names.
async fn read_entries(dir: &str) -> std::io::Result<Vec<(String, FileType)>> {
    let mut reader = tokio::fs::read_dir(dir).await?;
    let mut entries = Vec::new();
    while let Some(entry) = reader.next_entry().await? {
        let file_type = entry.file_type().await?;
        entries.push((entry.file_name().to_string_lossy().into_owned(), file_type));
    }
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    Ok(entries)
}

struct Visited {
    found: Vec<String>,
    next: Vec<(String, usize)>,
}

async fn visit(
    dir: String,
    depth: usize,
    max_depth: usize,
    skip: Arc<HashSet<String>>,
    on_found: Option<FoundCallback>,
) -> Visited {
    let mut visited = Visited {
        found: Vec::new(),
        next: Vec::new(),
    };
    let Ok(entries) = read_entries(&dir).await else {
        return visited;
    };
    for (name, file_type) in entries {
        // Follow a linked `.astroshot` itself, but never descend through other
        // symlinks: they can loop and are rarely where captures live.
        let is_dir = file_type.is_dir()
            || (name == ASTROSHOT_DIR && kind_of(&dir, &name, &file_type).await == Kind::Dir);
        if !is_dir {
            continue;
        }
        if name == ASTROSHOT_DIR {
            let astroshot_dir = join(&dir, &name);
            if let Some(callback) = &on_found {
                callback(&astroshot_dir);
            }
            visited.found.push(astroshot_dir);
            continue;
        }
        if name.starts_with('.') || skip.contains(&name) {
            continue;
        }
        if depth + 1 > max_depth {
            continue;
        }
        visited.next.push((join(&dir, &name), depth + 1));
    }
    visited
}

/// Breadth-first walk with bounded concurrency; never descends into a found tree.
pub async fn find_astroshot_dirs(roots: &[String], options: &FindOptions) -> Vec<String> {
    let max_depth = options.max_depth.unwrap_or(MAX_SCAN_DEPTH);
    let skip: Arc<HashSet<String>> = Arc::new(match &options.skip {
        Some(skip) => skip.clone(),
        None => SKIP_DIRECTORIES
            .iter()
            .map(|name| name.to_string())
            .collect(),
    });
    let concurrency = options.concurrency.unwrap_or(DEFAULT_CONCURRENCY);
    let mut found: Vec<String> = Vec::new();
    let mut queue: VecDeque<(String, usize)> = roots.iter().map(|root| (root.clone(), 0)).collect();
    let mut active: JoinSet<Visited> = JoinSet::new();
    let aborted = || {
        options
            .signal
            .as_ref()
            .is_some_and(|signal| signal.load(Ordering::SeqCst))
    };

    loop {
        if !aborted() {
            while active.len() < concurrency.max(1)
                && let Some((dir, depth)) = queue.pop_front()
            {
                active.spawn(visit(
                    dir,
                    depth,
                    max_depth,
                    skip.clone(),
                    options.on_found.clone(),
                ));
            }
        }
        let Some(result) = active.join_next().await else {
            break;
        };
        if let Ok(visited) = result {
            found.extend(visited.found);
            queue.extend(visited.next);
        }
    }
    // `Array#sort()` compares UTF-16 code units.
    found.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    found
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotContext {
    pub worktree_path: String,
    pub worktree: String,
    pub feature: String,
    pub feature_dir: String,
}

#[derive(Debug, Clone, Default)]
pub struct FeatureListing {
    pub files: BTreeSet<String>,
}

async fn list_feature(feature_dir: &str) -> Option<FeatureListing> {
    let entries = read_entries(feature_dir).await.ok()?;
    let mut files = BTreeSet::new();
    for (name, file_type) in entries {
        if kind_of(feature_dir, &name, &file_type).await == Kind::File {
            files.insert(name);
        }
    }
    Some(FeatureListing { files })
}

fn resolve_video(
    entry_video: Option<&str>,
    file_name: &str,
    files: &BTreeSet<String>,
) -> Option<String> {
    if let Some(video) = entry_video
        && !super::review_store::js_trim(video).is_empty()
    {
        return Some(video.to_string());
    }
    // /\.[^.]+$/ : strip the last ".ext" when the extension is non-empty.
    let stem = match file_name.rfind('.') {
        Some(index) if index + 1 < file_name.len() => &file_name[..index],
        _ => file_name,
    };
    VIDEO_EXTENSIONS
        .iter()
        .map(|extension| format!("{stem}.{extension}"))
        .find(|candidate| files.contains(candidate))
}

async fn review_for(
    document: Option<&ReviewDocument>,
    file_name: &str,
    run_id: Option<&str>,
    image_path: &str,
    stat: FileStat,
    hashes: &HashCache,
) -> Option<ReviewSnapshot> {
    let document = document?;
    let entry = scoped_entry(document, file_name, run_id);
    let mut sha: Option<String> = None;
    if entry_needs_hash(entry) {
        sha = hashes.hash(image_path, Some(stat)).await.ok();
    }
    Some(snapshot_from_entry(entry, sha.as_deref()))
}

pub async fn build_shot(
    image_path: &str,
    context: &ShotContext,
    manifest: Option<&FeatureManifest>,
    review: Option<&ReviewDocument>,
    listing: &FeatureListing,
    hashes: &HashCache,
) -> Option<Shot> {
    let stat = FileStat::of(image_path).ok()?;
    let file_name = basename(image_path).to_string();
    let entry = match_manifest_shot(manifest, &file_name);
    let parsed = sequence_and_slug(&file_name);
    let slug = entry
        .and_then(|entry| entry.slug.clone())
        .unwrap_or_else(|| parsed.slug.clone());
    let run_id = manifest.and_then(|manifest| manifest.run_id.clone());
    let video_file_name = resolve_video(
        entry.and_then(|entry| entry.video.as_deref()),
        &file_name,
        &listing.files,
    );
    let video_exists = video_file_name
        .as_ref()
        .is_some_and(|video| listing.files.contains(video));
    let duration_ms = entry
        .and_then(|entry| entry.duration_ms)
        .filter(|duration| *duration > 0.0);
    let kind = entry
        .and_then(|entry| entry.kind.as_deref())
        .unwrap_or("")
        .to_lowercase();
    let review = review_for(
        review,
        &file_name,
        run_id.as_deref(),
        image_path,
        stat,
        hashes,
    )
    .await;
    Some(Shot {
        id: image_path.to_string(),
        path: image_path.to_string(),
        file_name,
        worktree_path: context.worktree_path.clone(),
        worktree: context.worktree.clone(),
        worktree_short: worktree_short(&context.worktree),
        feature: context.feature.clone(),
        feature_dir: context.feature_dir.clone(),
        sequence: parsed
            .sequence
            .clone()
            .or_else(|| entry.and_then(|entry| entry.id.clone())),
        title: entry
            .and_then(|entry| entry.title.clone())
            .unwrap_or_else(|| humanize(&slug)),
        slug,
        description: entry
            .and_then(|entry| entry.description.clone())
            .unwrap_or_default(),
        url: entry.and_then(|entry| entry.url.clone()),
        status: parse_feature_status(manifest.and_then(|manifest| manifest.status.as_deref())),
        run_id,
        captured_at: parse_iso_date(entry.and_then(|entry| entry.captured_at.as_deref()))
            .unwrap_or(stat.mtime_ms),
        mtime_ms: stat.mtime_ms,
        is_movie: kind == "movie" || video_file_name.is_some(),
        video_path: if video_exists {
            video_file_name
                .as_deref()
                .map(|video| join(&context.feature_dir, video))
        } else {
            None
        },
        video_file_name,
        duration_ms,
        source: entry.and_then(|entry| entry.source.clone()),
        chapters: chapters_of(entry),
        review,
    })
}

async fn read_review_safely(directory: &str) -> Option<ReviewDocument> {
    read_review_document(directory).await.ok().flatten()
}

/// Every shot inside one feature directory, in directory order.
pub async fn scan_feature_dir(
    feature_dir: &str,
    context: &FrictionContext,
    hashes: &HashCache,
) -> Vec<Shot> {
    let Some(listing) = list_feature(feature_dir).await else {
        return Vec::new();
    };
    let feature = basename(feature_dir).to_string();
    let (manifest, review) =
        tokio::join!(read_manifest(feature_dir), read_review_safely(feature_dir));
    let shot_context = ShotContext {
        worktree_path: context.worktree_path.clone(),
        worktree: context.worktree.clone(),
        feature,
        feature_dir: feature_dir.to_string(),
    };
    let mut shots = Vec::new();
    for name in &listing.files {
        if !is_image_file(name) {
            continue;
        }
        if let Some(shot) = build_shot(
            &join(feature_dir, name),
            &shot_context,
            manifest.as_ref(),
            review.as_ref(),
            &listing,
            hashes,
        )
        .await
        {
            shots.push(shot);
        }
    }
    shots
}

/// Re-read one shot in place (after its image or sidecars changed).
pub async fn rebuild_shot(
    image_path: &str,
    context: &ShotContext,
    hashes: &HashCache,
) -> Option<Shot> {
    let listing = list_feature(&context.feature_dir).await?;
    let (manifest, review) = tokio::join!(
        read_manifest(&context.feature_dir),
        read_review_safely(&context.feature_dir)
    );
    build_shot(
        image_path,
        context,
        manifest.as_ref(),
        review.as_ref(),
        &listing,
        hashes,
    )
    .await
}

pub async fn scan_tree(astroshot_dir: &str, hashes: &HashCache) -> std::io::Result<AstroshotTree> {
    let worktree_path = dirname(astroshot_dir);
    let worktree = basename(&worktree_path).to_string();
    let context = FrictionContext {
        worktree_path: worktree_path.clone(),
        worktree: worktree.clone(),
    };
    let entries = read_entries(astroshot_dir).await.unwrap_or_default();
    let mut shots: Vec<Shot> = Vec::new();
    for (name, file_type) in entries {
        if name.starts_with('.') || is_reserved_dir(&name) {
            continue;
        }
        if kind_of(astroshot_dir, &name, &file_type).await != Kind::Dir {
            continue;
        }
        shots.extend(scan_feature_dir(&join(astroshot_dir, &name), &context, hashes).await);
    }
    // TS awaits `loadFrictionLogs` without a catch, so its failure rejects.
    let friction_logs = load_user_stories(astroshot_dir, &context, hashes).await?;
    Ok(AstroshotTree {
        astroshot_dir: astroshot_dir.to_string(),
        worktree_path,
        worktree,
        shots,
        friction_logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_data::model::ReviewState;
    use crate::review_data::review_store::{MarkSeenOptions, ReviewWriteRequest, mark_seen};
    use std::path::{Path, PathBuf};

    fn write(file: &Path, content: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    fn s(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new().prefix("scan-").tempdir().unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    // discovery

    #[tokio::test]
    async fn finds_astroshot_trees_skips_heavy_and_hidden_directories_and_does_not_descend_into_trees()
     {
        let (_guard, root) = root();
        write(&root.join("app/.astroshot/feature/0001-a.png"), "x");
        write(
            &root.join("app/.astroshot/nested/.astroshot/deeper/0001-b.png"),
            "x",
        );
        write(
            &root.join("app/node_modules/pkg/.astroshot/f/0001-c.png"),
            "x",
        );
        write(&root.join(".hidden/.astroshot/f/0001-d.png"), "x");
        write(&root.join("deep/a/b/c/.astroshot/f/0001-e.png"), "x");
        let roots = vec![s(&root)];
        let found = find_astroshot_dirs(&roots, &FindOptions::default()).await;
        assert_eq!(
            found,
            vec![
                s(&root.join("app/.astroshot")),
                s(&root.join("deep/a/b/c/.astroshot"))
            ]
        );
        let shallow = find_astroshot_dirs(
            &roots,
            &FindOptions {
                max_depth: Some(2),
                ..FindOptions::default()
            },
        )
        .await;
        assert_eq!(shallow, vec![s(&root.join("app/.astroshot"))]);
    }

    // scanTree

    #[tokio::test]
    async fn builds_shots_with_manifest_metadata_movie_pairing_and_review_scoping() {
        let (_guard, root) = root();
        let feature = root.join("demo-app/.astroshot/checkout");
        write(&feature.join("0001-welcome.png"), "welcome");
        write(&feature.join("0002-journey.png"), "poster");
        write(&feature.join("0002-journey.webm"), "video");
        write(&feature.join("0003-orphan.png"), "orphan");
        write(
            &feature.join("manifest.json"),
            &serde_json::json!({
                "version": 1,
                "run_id": "run-1",
                "status": "passed",
                "shots": [
                    { "id": "0001", "file": "0001-welcome.png", "title": "Welcome", "description": "Landing", "captured_at": "2026-09-05T10:00:00Z", "url": "/welcome" },
                    { "id": "0002", "file": "0002-journey.png", "kind": "movie", "duration_ms": 4200, "source": "browser", "chapters": [{ "slug": "a", "t_ms": 100 }] },
                ],
            })
            .to_string(),
        );
        mark_seen(
            &ReviewWriteRequest {
                directory: s(&feature),
                file_name: "0001-welcome.png".into(),
                run_id: Some("run-1".into()),
                target_path: s(&feature.join("0001-welcome.png")),
            },
            &MarkSeenOptions::default(),
        )
        .await
        .unwrap();
        write(
            &root.join("demo-app/.astroshot/friction-logs/scenario/prompt.md"),
            "# prompt",
        );
        write(
            &root.join("demo-app/.astroshot/friction-logs/scenario/runs/20260811T153000Z/log.jsonl"),
            &serde_json::json!({ "step": 1, "id": "s1", "title": "Step", "screenshots": ["0001-s1.png"] })
                .to_string(),
        );
        write(
            &root.join(
                "demo-app/.astroshot/friction-logs/scenario/runs/20260811T153000Z/0001-s1.png",
            ),
            "x",
        );

        let tree = scan_tree(&s(&root.join("demo-app/.astroshot")), &HashCache::default())
            .await
            .unwrap();
        assert_eq!(tree.worktree, "demo-app");
        let by_file = |name: &str| {
            tree.shots
                .iter()
                .find(|shot| shot.file_name == name)
                .unwrap()
        };
        let mut names: Vec<&str> = tree
            .shots
            .iter()
            .map(|shot| shot.file_name.as_str())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["0001-welcome.png", "0002-journey.png", "0003-orphan.png"]
        );

        let welcome = by_file("0001-welcome.png");
        assert_eq!(welcome.title, "Welcome");
        assert_eq!(welcome.url.as_deref(), Some("/welcome"));
        assert_eq!(
            welcome.status,
            Some(crate::review_data::model::FeatureStatus::Pass)
        );
        assert_eq!(welcome.captured_at, 1_788_602_400_000.0);
        assert_eq!(welcome.review.as_ref().unwrap().state, ReviewState::Seen);

        let journey = by_file("0002-journey.png");
        assert!(journey.is_movie);
        assert_eq!(
            journey.video_file_name.as_deref(),
            Some("0002-journey.webm")
        );
        assert_eq!(
            journey.video_path.as_deref(),
            Some(s(&feature.join("0002-journey.webm")).as_str())
        );
        assert_eq!(journey.duration_ms, Some(4200.0));
        assert_eq!(
            journey.chapters,
            vec![crate::review_data::model::Chapter {
                slug: Some("a".into()),
                title: None,
                t_ms: Some(100.0)
            }]
        );
        assert_eq!(journey.title, "Journey");
        assert_eq!(journey.review.as_ref().unwrap().state, ReviewState::Pending);

        let orphan = by_file("0003-orphan.png");
        assert_eq!(orphan.title, "Orphan");
        assert!(!orphan.is_movie);

        assert_eq!(tree.friction_logs.len(), 1);
        assert_eq!(tree.friction_logs[0].runs[0].steps[0].screenshots.len(), 1);
        assert!(
            !tree
                .shots
                .iter()
                .any(|shot| shot.path.contains("friction-logs"))
        );
    }

    #[tokio::test]
    async fn reserved_directories_never_appear_as_features_and_both_hold_stories() {
        let (_guard, root) = root();
        write(&root.join("app/.astroshot/checkout/0001-a.png"), "x");
        for tree in ["stories", "friction-logs"] {
            let run = format!("app/.astroshot/{tree}/{tree}-flow/runs/20260811T153000Z");
            write(&root.join(format!("{run}/log.jsonl")), "{\"step\":1}\n");
            write(&root.join(format!("{run}/0001-s1.png")), "x");
            write(
                &root.join(format!("app/.astroshot/{tree}/0001-stray.png")),
                "x",
            );
        }
        let tree = scan_tree(&s(&root.join("app/.astroshot")), &HashCache::default())
            .await
            .unwrap();
        assert_eq!(tree.shots.len(), 1);
        assert_eq!(tree.shots[0].feature, "checkout");
        let mut slugs: Vec<&str> = tree.friction_logs.iter().map(|l| l.slug.as_str()).collect();
        slugs.sort_unstable();
        assert_eq!(slugs, ["friction-logs-flow", "stories-flow"]);
    }

    #[tokio::test]
    async fn un_sees_a_shot_whose_bytes_changed_and_keeps_its_comments() {
        let (_guard, root) = root();
        let feature = root.join("app/.astroshot/f");
        let image = feature.join("0001-a.png");
        write(&image, "v1");
        mark_seen(
            &ReviewWriteRequest {
                directory: s(&feature),
                file_name: "0001-a.png".into(),
                run_id: None,
                target_path: s(&image),
            },
            &MarkSeenOptions {
                comment: Some("note".into()),
                now: None,
            },
        )
        .await
        .unwrap();
        write(&image, "v2");
        let tree = scan_tree(&s(&root.join("app/.astroshot")), &HashCache::default())
            .await
            .unwrap();
        let review = tree.shots[0].review.as_ref().unwrap();
        assert_eq!(review.state, ReviewState::Pending);
        assert!(review.is_stale);
        let bodies: Vec<&str> = review.comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, vec!["note"]);
    }

    // Behaviours the TS suite does not pin down.

    #[cfg(unix)]
    #[tokio::test]
    async fn follows_a_linked_astroshot_dir_but_not_other_symlinks() {
        let (_guard, root) = root();
        write(&root.join("real/feature/0001-a.png"), "x");
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("app/.astroshot")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let found = find_astroshot_dirs(&[s(&root)], &FindOptions::default()).await;
        assert_eq!(found, vec![s(&root.join("app/.astroshot"))]);
    }

    #[tokio::test]
    async fn aborted_signal_stops_the_walk_and_on_found_sees_each_tree() {
        let (_guard, root) = root();
        write(&root.join("a/.astroshot/f/0001-a.png"), "x");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let options = FindOptions {
            on_found: Some(Arc::new(move |dir| {
                sink.lock().unwrap().push(dir.to_string())
            })),
            ..FindOptions::default()
        };
        let found = find_astroshot_dirs(&[s(&root)], &options).await;
        assert_eq!(*seen.lock().unwrap(), found);
        let signal = Arc::new(AtomicBool::new(true));
        let aborted = find_astroshot_dirs(
            &[s(&root)],
            &FindOptions {
                signal: Some(signal),
                ..FindOptions::default()
            },
        )
        .await;
        assert!(aborted.is_empty());
    }
}
