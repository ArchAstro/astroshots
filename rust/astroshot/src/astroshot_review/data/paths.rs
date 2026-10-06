//! Port of `packages/astroshot-review/src/data/paths.ts`.
//!
//! Paths are handled as `&str` with POSIX `node:path` semantics (`/` separator),
//! matching the TS behavior on macOS and Linux.

pub const ASTROSHOT_DIR: &str = ".astroshot";
pub const FRICTION_DIR: &str = "friction-logs";

/// Directories the scanner never descends into (mirrors the macOS app).
pub const SKIP_DIRECTORIES: &[&str] = &[
    "node_modules",
    ".git",
    "DerivedData",
    "build",
    ".build",
    "Pods",
    ".next",
    "dist",
    "out",
    "target",
    "vendor",
    "Checkouts",
    "xcuserdata",
    ".turbo",
    ".cache",
    "coverage",
    "tmp",
    ".pnpm-store",
    "Carthage",
    "bazel-bin",
    "bazel-out",
    "bazel-testlogs",
    ".gradle",
];

pub const MAX_SCAN_DEPTH: usize = 10;

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];
pub const VIDEO_EXTENSIONS: &[&str] = &["webm", "mp4", "mov", "m4v"];

const SEP: char = '/';

/// `path.basename` (POSIX).
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches(SEP);
    match trimmed.rfind(SEP) {
        Some(index) => &trimmed[index + 1..],
        None => trimmed,
    }
}

/// `path.dirname` (POSIX), including node's quirks (`/a//b` -> `/a/`).
pub fn dirname(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.is_empty() {
        return ".".to_string();
    }
    let has_root = bytes[0] == b'/';
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    for i in (1..bytes.len()).rev() {
        if bytes[i] == b'/' {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None => (if has_root { "/" } else { "." }).to_string(),
        Some(1) if has_root => "//".to_string(),
        Some(end) => path[..end].to_string(),
    }
}

/// `path.extname` (POSIX) of the final path segment.
fn extname(path: &str) -> &str {
    let base = basename(path);
    if base == ".." {
        return "";
    }
    match base.rfind('.') {
        Some(index) if index > 0 => &base[index..],
        _ => "",
    }
}

/// `path.join(a, b)` for a non-empty directory and a relative segment.
fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_string();
    }
    let joined = format!("{a}/{b}");
    let absolute = joined.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let body = parts.join("/");
    match (absolute, body.is_empty()) {
        (true, _) => format!("/{body}"),
        (false, true) => ".".to_string(),
        (false, false) => body,
    }
}

pub fn extension_of(file_name: &str) -> String {
    extname(file_name)
        .strip_prefix('.')
        .unwrap_or("")
        .to_lowercase()
}

pub fn is_image_file(file_name: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&extension_of(file_name).as_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotPath {
    pub worktree_path: String,
    pub worktree: String,
    pub feature: String,
    pub feature_dir: String,
    pub file_name: String,
}

/// Accept only `<worktree>/.astroshot/<feature>/<image>`; friction logs are excluded.
pub fn parse_shot_path(image_path: &str) -> Option<ShotPath> {
    let file_name = basename(image_path);
    if !is_image_file(file_name) {
        return None;
    }
    let feature_dir = dirname(image_path);
    let feature = basename(&feature_dir);
    if feature.is_empty() || feature == ASTROSHOT_DIR || feature == FRICTION_DIR {
        return None;
    }
    let astroshot_dir = dirname(&feature_dir);
    if basename(&astroshot_dir) != ASTROSHOT_DIR {
        return None;
    }
    if image_path.split(SEP).any(|part| part == FRICTION_DIR) {
        return None;
    }
    let worktree_path = dirname(&astroshot_dir);
    Some(ShotPath {
        worktree: basename(&worktree_path).to_string(),
        worktree_path,
        feature: feature.to_string(),
        feature_dir,
        file_name: file_name.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceAndSlug {
    pub sequence: Option<String>,
    pub slug: String,
}

pub fn sequence_and_slug(file_name: &str) -> SequenceAndSlug {
    // /\.[^.]+$/ : strip the last ".ext" when the extension is non-empty.
    let stem = match file_name.rfind('.') {
        Some(index) if index + 1 < file_name.len() => &file_name[..index],
        _ => file_name,
    };
    if let Some(dash) = stem.find('-')
        && dash > 0
        && stem[..dash].bytes().all(|b| b.is_ascii_digit())
    {
        return SequenceAndSlug {
            sequence: Some(stem[..dash].to_string()),
            slug: stem[dash + 1..].to_string(),
        };
    }
    SequenceAndSlug {
        sequence: None,
        slug: stem.to_string(),
    }
}

pub fn humanize(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn worktree_short(name: &str) -> String {
    // /wt\d+/ : leftmost "wt" followed by at least one ASCII digit.
    let bytes = name.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'w' && bytes[i + 1] == b't' && bytes[i + 2].is_ascii_digit() {
            let mut end = i + 3;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            return name[i..end].to_string();
        }
        i += 1;
    }
    // JS lengths and slices count UTF-16 code units.
    let units: Vec<u16> = name.encode_utf16().collect();
    if units.len() <= 8 {
        name.to_string()
    } else {
        String::from_utf16_lossy(&units[..6])
    }
}

pub fn is_inside_friction_logs(file_path: &str) -> bool {
    let parts: Vec<&str> = file_path.split(SEP).collect();
    match parts.iter().position(|part| *part == ASTROSHOT_DIR) {
        Some(index) => parts.get(index + 1) == Some(&FRICTION_DIR),
        None => false,
    }
}

pub fn friction_logs_dir(astroshot_dir: &str) -> String {
    join(astroshot_dir, FRICTION_DIR)
}

pub fn abbreviate_home(file_path: &str, home: &str) -> String {
    if file_path == home {
        return "~".to_string();
    }
    let prefix = if home.ends_with(SEP) {
        home.to_string()
    } else {
        format!("{home}{SEP}")
    };
    match file_path.strip_prefix(&prefix) {
        Some(rest) => format!("~{SEP}{rest}"),
        None => file_path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_worktree_astroshot_feature_image() {
        let parsed = parse_shot_path("/repos/app/.astroshot/checkout/0002-configure.png");
        assert_eq!(
            parsed,
            Some(ShotPath {
                worktree_path: "/repos/app".into(),
                worktree: "app".into(),
                feature: "checkout".into(),
                feature_dir: "/repos/app/.astroshot/checkout".into(),
                file_name: "0002-configure.png".into(),
            })
        );
    }

    #[test]
    fn rejects_friction_logs_non_images_and_malformed_layouts() {
        assert_eq!(
            parse_shot_path("/repos/app/.astroshot/friction-logs/x/runs/1/0001-a.png"),
            None
        );
        assert_eq!(
            parse_shot_path("/repos/app/.astroshot/checkout/manifest.json"),
            None
        );
        assert_eq!(parse_shot_path("/repos/app/.astroshot/0001-a.png"), None);
        assert_eq!(
            parse_shot_path("/repos/app/shots/checkout/0001-a.png"),
            None
        );
        assert!(is_inside_friction_logs(
            "/repos/app/.astroshot/friction-logs/x/prompt.md"
        ));
    }

    #[test]
    fn splits_sequence_and_slug_only_for_numeric_prefixes() {
        assert_eq!(
            sequence_and_slug("0004-configure.png"),
            SequenceAndSlug {
                sequence: Some("0004".into()),
                slug: "configure".into()
            }
        );
        assert_eq!(
            sequence_and_slug("hero-shot.png"),
            SequenceAndSlug {
                sequence: None,
                slug: "hero-shot".into()
            }
        );
        assert_eq!(humanize("next-steps_two"), "Next Steps Two");
    }

    #[test]
    fn shortens_worktree_names_like_the_app() {
        assert_eq!(worktree_short("firstlanding-wt12"), "wt12");
        assert_eq!(worktree_short("demo-app"), "demo-app");
        assert_eq!(worktree_short("very-long-worktree-name"), "very-l");
    }

    #[test]
    fn path_helpers_match_node_posix_semantics() {
        assert_eq!(dirname("/a//b"), "/a/");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("a"), ".");
        assert_eq!(basename("a/b///"), "b");
        assert_eq!(extension_of("x.PNG"), "png");
        assert_eq!(extension_of(".bashrc"), "");
        assert_eq!(
            friction_logs_dir("/r/.astroshot"),
            "/r/.astroshot/friction-logs"
        );
        assert_eq!(abbreviate_home("/home/me", "/home/me"), "~");
        assert_eq!(abbreviate_home("/home/me/x", "/home/me"), "~/x");
        assert_eq!(abbreviate_home("/home/mex", "/home/me"), "/home/mex");
    }
}
