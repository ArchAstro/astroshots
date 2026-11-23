//! Chrome discovery. Playwright downloaded its own Chromium; the Rust port
//! uses whatever Chrome-family binary is on the machine.
//!
//! Search order:
//! 1. First non-blank override: `ASTROSHOT_CHROME`, `CHROME_PATH`, then
//!    `PLAYWRIGHT_MCP_EXECUTABLE_PATH`. An invalid override stops discovery.
//! 2. installed Chrome/Chromium: macOS app bundles, then Linux names via `which`
//! 3. Playwright's browser cache (`PLAYWRIGHT_BROWSERS_PATH`,
//!    `~/Library/Caches/ms-playwright`, `~/.cache/ms-playwright`), newest
//!    revision first. A headless launch takes `chromium_headless_shell-<rev>`
//!    before `chromium-<rev>`, as Playwright does: the shell starts in a
//!    fraction of the time of Chrome for Testing. A headed launch takes
//!    `chromium-<rev>` first.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::BrowserError;

/// Env vars checked for an explicit Chrome path, in order.
pub const CHROME_ENV_VARS: [&str; 3] = [
    "ASTROSHOT_CHROME",
    "CHROME_PATH",
    "PLAYWRIGHT_MCP_EXECUTABLE_PATH",
];

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
    pub env: &'a dyn Fn(&str) -> Option<OsString>,
    pub home: Option<PathBuf>,
    pub app_dirs: Vec<PathBuf>,
    /// Looks a command up on `PATH`.
    pub which: &'a dyn Fn(&str) -> Option<PathBuf>,
    /// Whether the browser will be launched with a window.
    pub headed: bool,
}

/// Find a Chrome executable for a headless launch, or an error listing
/// everything searched.
pub fn find_chrome() -> Result<PathBuf, BrowserError> {
    find_chrome_for(false)
}

/// [`find_chrome`] for a headed or headless launch.
pub fn find_chrome_for(headed: bool) -> Result<PathBuf, BrowserError> {
    let home = dirs::home_dir();
    let mut app_dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = &home {
        app_dirs.push(home.join("Applications"));
    }
    let env = |key: &str| std::env::var_os(key);
    let which = |name: &str| which::which(name).ok();
    find_chrome_with(&Probe {
        env: &env,
        home,
        app_dirs,
        which: &which,
        headed,
    })
}

pub fn find_chrome_with(probe: &Probe<'_>) -> Result<PathBuf, BrowserError> {
    let mut searched: Vec<String> = Vec::new();

    for var in CHROME_ENV_VARS {
        let value = (probe.env)(var).unwrap_or_default();
        let value = value
            .to_str()
            .map(str::trim)
            .map(OsStr::new)
            .unwrap_or(&value);
        if value.is_empty() {
            searched.push(format!("${var} (unset)"));
            continue;
        }
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(BrowserError::InvalidChromeOverride { var, path });
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
        if let Some(path) = find_in_playwright_cache(&root, probe.headed) {
            return Ok(path);
        }
        searched.push(format!("{}/chromium-*", root.display()));
    }

    Err(BrowserError::ChromeNotFound { searched })
}

fn playwright_cache_roots(probe: &Probe<'_>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(custom) = (probe.env)("PLAYWRIGHT_BROWSERS_PATH")
        .filter(|value| value.to_str().is_some_and(|value| value != "0"))
    {
        roots.push(PathBuf::from(custom));
    }
    if let Some(home) = &probe.home {
        roots.push(home.join("Library/Caches/ms-playwright"));
        roots.push(home.join(".cache/ms-playwright"));
    }
    roots
}

/// Newest revision with a runnable executable: the headless shell first for
/// a headless launch, the full browser first for a headed one.
pub fn find_in_playwright_cache(root: &Path, headed: bool) -> Option<PathBuf> {
    let prefixes = if headed {
        ["chromium-", "chromium_headless_shell-"]
    } else {
        ["chromium_headless_shell-", "chromium-"]
    };
    for prefix in prefixes {
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
        env: &'a dyn Fn(&str) -> Option<OsString>,
        which: &'a dyn Fn(&str) -> Option<PathBuf>,
    ) -> Probe<'a> {
        Probe {
            env,
            home: Some(home.to_path_buf()),
            app_dirs: vec![home.join("Applications")],
            which,
            headed: false,
        }
    }

    #[test]
    fn explicit_overrides_win_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let overrides = [
            ("ASTROSHOT_CHROME", home.join("astroshot-chrome")),
            ("CHROME_PATH", home.join("chrome-path")),
            (
                "PLAYWRIGHT_MCP_EXECUTABLE_PATH",
                home.join("harness-chrome"),
            ),
        ];
        for (_, path) in &overrides {
            touch(path);
        }
        touch(&home.join("Applications").join(MAC_APPS[0]));
        let which = |_: &str| panic!("explicit override must precede PATH discovery");
        for (first, (_, expected)) in overrides.iter().enumerate() {
            let env = |key: &str| {
                overrides
                    .iter()
                    .enumerate()
                    .find_map(|(index, (var, path))| {
                        (*var == key).then(|| {
                            if index < first {
                                "  ".into()
                            } else {
                                format!(" {} ", path.display()).into()
                            }
                        })
                    })
            };
            let probe = probe_in(home, &env, &which);
            assert_eq!(&find_chrome_with(&probe).unwrap(), expected);
        }
    }

    #[test]
    fn invalid_explicit_override_does_not_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let harness = home.join("harness-chrome");
        let app = home.join("Applications").join(MAC_APPS[0]);
        touch(&app);
        let which = |_: &str| panic!("invalid override must stop discovery");

        for bad_path in [harness.as_path(), home] {
            let env = |key: &str| {
                (key == "PLAYWRIGHT_MCP_EXECUTABLE_PATH").then(|| bad_path.as_os_str().to_owned())
            };
            let err = find_chrome_with(&probe_in(home, &env, &which)).unwrap_err();
            assert!(
                matches!(err, BrowserError::InvalidChromeOverride { var: "PLAYWRIGHT_MCP_EXECUTABLE_PATH", path } if path == bad_path)
            );
        }

        touch(&harness);
        for legacy_var in ["ASTROSHOT_CHROME", "CHROME_PATH"] {
            let missing = home.join("missing-chrome");
            let env = |key: &str| match key {
                "PLAYWRIGHT_MCP_EXECUTABLE_PATH" => Some(harness.as_os_str().to_owned()),
                key if key == legacy_var => Some(missing.as_os_str().to_owned()),
                _ => None,
            };
            let err = find_chrome_with(&probe_in(home, &env, &which)).unwrap_err();
            assert!(
                matches!(err, BrowserError::InvalidChromeOverride { var, path } if var == legacy_var && path == missing)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_override_is_not_treated_as_unset() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(OsStr::from_bytes(b"chrome-\xff"));
        let env = |key: &str| {
            (key == "PLAYWRIGHT_MCP_EXECUTABLE_PATH").then(|| path.as_os_str().to_owned())
        };
        let which = |_: &str| panic!("invalid override must stop discovery");
        let err = find_chrome_with(&probe_in(dir.path(), &env, &which)).unwrap_err();
        assert!(
            matches!(err, BrowserError::InvalidChromeOverride { path: invalid, .. } if invalid == path)
        );
    }

    #[test]
    fn without_overrides_apps_then_path_then_cache_are_used() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let app = home.join("Applications").join(MAC_APPS[0]);
        let path = home.join("bin").join(LINUX_NAMES[0]);
        let cache = home.join(".cache/ms-playwright/chromium-1248/chrome-linux/chrome");
        for candidate in [&app, &path, &cache] {
            touch(candidate);
        }
        let env = |_: &str| None;
        let which = |key: &str| (key == LINUX_NAMES[0] && path.is_file()).then(|| path.clone());
        let probe = probe_in(home, &env, &which);
        assert_eq!(find_chrome_with(&probe).unwrap(), app);
        std::fs::remove_file(app).unwrap();
        assert_eq!(find_chrome_with(&probe).unwrap(), path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(find_chrome_with(&probe).unwrap(), cache);
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
        // Headless: the shell, as Playwright launches. Headed: the full browser.
        let found = find_chrome_with(&probe_in(dir.path(), &env, &which)).unwrap();
        assert!(
            found
                .to_string_lossy()
                .contains("chromium_headless_shell-9999"),
            "{found:?}"
        );
        let headed = Probe {
            headed: true,
            ..probe_in(dir.path(), &env, &which)
        };
        let found = find_chrome_with(&headed).unwrap();
        assert!(
            found.to_string_lossy().contains("chromium-1243"),
            "{found:?}"
        );
    }

    #[test]
    fn a_headless_launch_falls_back_to_the_full_browser() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".cache/ms-playwright");
        touch(&root.join("chromium-1200/chrome-linux/chrome"));
        let env = |_: &str| None;
        let which = |_: &str| None;
        let found = find_chrome_with(&probe_in(dir.path(), &env, &which)).unwrap();
        assert!(
            found.to_string_lossy().contains("chromium-1200"),
            "{found:?}"
        );
    }

    #[test]
    fn error_lists_everything_searched() {
        let dir = tempfile::tempdir().unwrap();
        let env = |_: &str| None;
        let which = |_: &str| None;
        let err = find_chrome_with(&probe_in(dir.path(), &env, &which)).unwrap_err();
        let text = err.to_string();
        for var in CHROME_ENV_VARS {
            assert!(text.contains(&format!("${var} (unset)")), "{text}");
        }
        assert!(text.contains("PATH: google-chrome"), "{text}");
        assert!(text.contains("ms-playwright/chromium-*"), "{text}");
    }
}
