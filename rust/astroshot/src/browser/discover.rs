//! Chrome discovery. Playwright downloaded its own Chromium; the Rust port
//! uses whatever Chrome-family binary is on the machine.
//!
//! Search order (first existing file wins):
//! 1. `ASTROSHOT_CHROME`, then `CHROME_PATH` (a set-but-missing path is an
//!    error rather than a silent fallthrough)
//! 2. installed Chrome/Chromium: macOS app bundles, then Linux names via `which`
//! 3. Playwright's browser cache (`PLAYWRIGHT_BROWSERS_PATH`,
//!    `~/Library/Caches/ms-playwright`, `~/.cache/ms-playwright`), newest
//!    `chromium-<rev>` first, then `chromium_headless_shell-<rev>`

use std::path::{Path, PathBuf};

use super::BrowserError;

/// Env vars checked for an explicit Chrome path, in order.
pub const CHROME_ENV_VARS: [&str; 2] = ["ASTROSHOT_CHROME", "CHROME_PATH"];

const MAC_APPS: [&str; 5] = [
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Chromium.app/Contents/MacOS/Chromium",
    "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

const LINUX_NAMES: [&str; 7] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "chrome",
    "chrome-browser",
    "microsoft-edge",
];

/// Executable locations inside a Playwright `chromium-<rev>` directory.
const PLAYWRIGHT_EXES: [&str; 9] = [
    "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
    "chrome-mac/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "chrome-linux64/chrome",
    "chrome-linux/chrome",
    "chrome-headless-shell-mac-arm64/chrome-headless-shell",
    "chrome-headless-shell-linux64/chrome-headless-shell",
    "chrome-win/chrome.exe",
];

/// Everything discovery reads from the environment, so tests can fake it.
pub struct Probe<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub home: Option<PathBuf>,
    pub app_dirs: Vec<PathBuf>,
    /// Looks a command up on `PATH`.
    pub which: &'a dyn Fn(&str) -> Option<PathBuf>,
}

/// Find a Chrome executable, or an error listing everything searched.
pub fn find_chrome() -> Result<PathBuf, BrowserError> {
    let home = dirs::home_dir();
    let mut app_dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = &home {
        app_dirs.push(home.join("Applications"));
    }
    let env = |key: &str| std::env::var(key).ok();
    let which = |name: &str| which::which(name).ok();
    find_chrome_with(&Probe {
        env: &env,
        home,
        app_dirs,
        which: &which,
    })
}

pub fn find_chrome_with(probe: &Probe<'_>) -> Result<PathBuf, BrowserError> {
    let mut searched: Vec<String> = Vec::new();

    for var in CHROME_ENV_VARS {
        if let Some(value) = (probe.env)(var).filter(|v| !v.trim().is_empty()) {
            let path = PathBuf::from(value.trim());
            if path.is_file() {
                return Ok(path);
            }
            searched.push(format!("${var}={} (not a file)", path.display()));
        } else {
            searched.push(format!("${var} (unset)"));
        }
    }

    for dir in &probe.app_dirs {
        for app in MAC_APPS {
            let path = dir.join(app);
            if path.is_file() {
                return Ok(path);
            }
            searched.push(path.display().to_string());
        }
    }
    for name in LINUX_NAMES {
        if let Some(path) = (probe.which)(name) {
            return Ok(path);
        }
    }
    searched.push(format!("PATH: {}", LINUX_NAMES.join(", ")));

    for root in playwright_cache_roots(probe) {
        if let Some(path) = find_in_playwright_cache(&root) {
            return Ok(path);
        }
        searched.push(format!("{}/chromium-*", root.display()));
    }

    Err(BrowserError::ChromeNotFound { searched })
}

fn playwright_cache_roots(probe: &Probe<'_>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(custom) = (probe.env)("PLAYWRIGHT_BROWSERS_PATH").filter(|v| v != "0") {
        roots.push(PathBuf::from(custom));
    }
    if let Some(home) = &probe.home {
        roots.push(home.join("Library/Caches/ms-playwright"));
        roots.push(home.join(".cache/ms-playwright"));
    }
    roots
}

/// Newest `chromium-<rev>` with a runnable executable, falling back to
/// `chromium_headless_shell-<rev>`.
pub fn find_in_playwright_cache(root: &Path) -> Option<PathBuf> {
    for prefix in ["chromium-", "chromium_headless_shell-"] {
        let mut revisions: Vec<(u64, PathBuf)> = std::fs::read_dir(root)
            .ok()?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let rev = name.strip_prefix(prefix)?.parse::<u64>().ok()?;
                Some((rev, entry.path()))
            })
            .collect();
        revisions.sort_by_key(|(rev, _)| std::cmp::Reverse(*rev));
        for (_, dir) in revisions {
            for exe in PLAYWRIGHT_EXES {
                let path = dir.join(exe);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    fn probe_in<'a>(
        home: &Path,
        env: &'a dyn Fn(&str) -> Option<String>,
        which: &'a dyn Fn(&str) -> Option<PathBuf>,
    ) -> Probe<'a> {
        Probe {
            env,
            home: Some(home.to_path_buf()),
            app_dirs: vec![home.join("Applications")],
            which,
        }
    }

    #[test]
    fn env_var_wins_and_astroshot_chrome_beats_chrome_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a-chrome");
        let b = dir.path().join("b-chrome");
        touch(&a);
        touch(&b);
        let (a2, b2) = (a.clone(), b.clone());
        let env = move |k: &str| match k {
            "ASTROSHOT_CHROME" => Some(a2.display().to_string()),
            "CHROME_PATH" => Some(b2.display().to_string()),
            _ => None,
        };
        let which = |_: &str| None;
        let found = find_chrome_with(&probe_in(dir.path(), &env, &which)).unwrap();
        assert_eq!(found, a);
    }

    #[test]
    fn playwright_cache_picks_newest_revision() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".cache/ms-playwright");
        for rev in ["1200", "1243", "999"] {
            touch(&root.join(format!("chromium-{rev}/chrome-linux/chrome")));
        }
        touch(&root.join(
            "chromium_headless_shell-9999/chrome-headless-shell-linux64/chrome-headless-shell",
        ));
        let env = |_: &str| None;
        let which = |_: &str| None;
        let found = find_chrome_with(&probe_in(dir.path(), &env, &which)).unwrap();
        assert!(
            found.to_string_lossy().contains("chromium-1243"),
            "{found:?}"
        );
    }

    #[test]
    fn error_lists_everything_searched() {
        let dir = tempfile::tempdir().unwrap();
        let env = |k: &str| (k == "CHROME_PATH").then(|| "/nope/chrome".to_string());
        let which = |_: &str| None;
        let err = find_chrome_with(&probe_in(dir.path(), &env, &which)).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("$CHROME_PATH=/nope/chrome (not a file)"),
            "{text}"
        );
        assert!(text.contains("$ASTROSHOT_CHROME (unset)"), "{text}");
        assert!(text.contains("PATH: google-chrome"), "{text}");
        assert!(text.contains("ms-playwright/chromium-*"), "{text}");
    }
}
