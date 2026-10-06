//! `astroshot doctor`: report every way Astroshots setup silently fails.
//!
//! Port of `packages/astroshot/bin/doctor.mjs`. Read-only by contract: it never
//! installs, launches, or writes preferences. Each check prints pass/fail plus
//! the exact remediation command, and required failures exit non-zero.
//!
//! Deliberate divergences from the TS checks (everything else, including the
//! check order, ids, titles and every line of text, is unchanged):
//!
//! | check | change |
//! |---|---|
//! | `astroshot` | new first check: the astroshot binary version (TS reported npm package versions only through the install itself) |
//! | `node` | now optional: Node is needed only for React and Ink shots; probes `node --version` |
//! | `node-helper` | new: whether `astroshot::node_helper::find_helper` finds `helper.mjs` |
//! | `demo-fixtures` | remediation points at the release page (the demo payload is embedded) |
//! | `chromium` | `astroshot::browser::find_chrome()` replaces Playwright's `executablePath()` |
//! | `screen-recording` | probes by re-running this binary as `astroshot movie check-screen-access` |
//!
//! Every side effect goes through [`Host`], so tests inject the environment,
//! command runner and filesystem instead of reading the real machine.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use super::demo::load_demo_fixtures;
use super::mac_preferences::{
    ASTROSHOTS_DOMAIN, CoverageState, ReadWatchConfigurationOptions, WatchConfiguration,
    WatchCoverage, evaluate_watch_coverage, preference_tools_available, read_watch_configuration,
};

const APP_PROCESS_NAME: &str = "Astroshots";
/// The TS `engines.node` fallback; also what the `node` check compares against.
const NODE_RANGE: &str = ">=22.14.0";
/// `process.platform === "darwin"` is `"macos"` in Rust.
const MACOS: &str = "macos";

pub fn doctor_help() -> String {
    "astroshot doctor — diagnose Astroshots capture setup (read-only)

Usage:
  astroshot doctor [options]

Options:
  --root <dir>     Project directory to test for watch coverage (default: cwd)
  --json           Machine-readable report
  --skip-screen    Skip the macOS Screen Recording probe (avoids a Swift run)
  -h, --help       Show this help

Reports Node version, watched-folder coverage, whether the Astroshots app is
installed and running, the managed Chromium runtime, and macOS Screen Recording
state for desktop.window. Exits non-zero when a required check fails.
doctor never installs anything and never changes app state."
        .to_string()
}

// ------------------------------------------------------------------- seams

/// Result of running an external command (`spawnSync` with `encoding: utf8`).
#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    /// `None` when the process could not be spawned or was killed by a signal.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Everything doctor reads from the machine.
pub trait Host {
    /// `std::env::consts::OS` (`"macos"` for darwin).
    fn platform(&self) -> String;
    fn home_dir(&self) -> Option<PathBuf>;
    fn current_dir(&self) -> PathBuf;
    fn exists(&self, path: &Path) -> bool;
    fn run(&self, program: &str, args: &[&str]) -> CommandOutput;
    /// `node --version` without the leading `v`, or why it could not run.
    fn node_version(&self) -> std::result::Result<String, String>;
    /// Path of the node helper script, or the lookup error text.
    fn node_helper(&self) -> std::result::Result<PathBuf, String>;
    /// `astroshot::browser::find_chrome()`.
    fn chrome(&self) -> std::result::Result<PathBuf, String>;
    fn read_watch_configuration(&self) -> WatchConfiguration;
    /// The `astroshot movie check-screen-access` probe.
    fn screen_access(&self) -> CommandOutput;
    fn binary_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

/// The real machine.
pub struct SystemHost;

impl Host for SystemHost {
    fn platform(&self) -> String {
        std::env::consts::OS.to_string()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        dirs::home_dir()
    }

    fn current_dir(&self) -> PathBuf {
        std::env::current_dir().unwrap_or_default()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn run(&self, program: &str, args: &[&str]) -> CommandOutput {
        run_command(Command::new(program).args(args))
    }

    fn node_version(&self) -> std::result::Result<String, String> {
        let node = crate::node_helper::find_node().map_err(|error| error.to_string())?;
        let output = Command::new(&node)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("could not run {}: {error}", node.display()))?;
        if !output.status.success() {
            return Err(format!("{} --version failed", node.display()));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.trim().trim_start_matches('v').to_string())
    }

    fn node_helper(&self) -> std::result::Result<PathBuf, String> {
        crate::node_helper::find_helper().map_err(|error| error.to_string())
    }

    fn chrome(&self) -> std::result::Result<PathBuf, String> {
        crate::browser::find_chrome().map_err(|error| error.to_string())
    }

    fn read_watch_configuration(&self) -> WatchConfiguration {
        read_watch_configuration(&ReadWatchConfigurationOptions::default())
    }

    fn screen_access(&self) -> CommandOutput {
        match std::env::current_exe() {
            Ok(exe) => run_command(Command::new(exe).args(["movie", "check-screen-access"])),
            Err(error) => CommandOutput {
                status: None,
                stdout: String::new(),
                stderr: error.to_string(),
            },
        }
    }
}

fn run_command(command: &mut Command) -> CommandOutput {
    match command.stdin(Stdio::null()).output() {
        Ok(output) => CommandOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(error) => CommandOutput {
            status: None,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

// ------------------------------------------------------------------ checks

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    Warn,
    Skip,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Warn => "warn",
            Status::Skip => "skip",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Warn => "WARN",
            Status::Skip => "SKIP",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub id: &'static str,
    pub title: String,
    pub required: bool,
    pub status: Status,
    pub detail: String,
    pub remediation: String,
    /// Watch coverage evidence, only on `watch-roots`.
    pub data: Option<WatchCoverage>,
    /// TS builds the `chromium` check from a base object that already holds
    /// `remediation`, so its JSON key order is id, title, required,
    /// remediation, status, detail.
    remediation_first: bool,
}

impl Check {
    fn new(
        id: &'static str,
        title: &str,
        required: bool,
        status: Status,
        detail: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Check {
            id,
            title: title.to_string(),
            required,
            status,
            detail: detail.into(),
            remediation: remediation.into(),
            data: None,
            remediation_first: false,
        }
    }

    /// The JSON object, keys in the TS insertion order.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("title".into(), json!(self.title));
        map.insert("required".into(), json!(self.required));
        if self.remediation_first {
            map.insert("remediation".into(), json!(self.remediation));
        }
        map.insert("status".into(), json!(self.status.as_str()));
        map.insert("detail".into(), json!(self.detail));
        if !self.remediation_first {
            map.insert("remediation".into(), json!(self.remediation));
        }
        if let Some(data) = &self.data {
            map.insert(
                "data".into(),
                serde_json::to_value(data).unwrap_or(Value::Null),
            );
        }
        Value::Object(map)
    }
}

fn parse_version(value: &str) -> Option<[u64; 3]> {
    // /(\d+)\.(\d+)\.(\d+)/ — first match anywhere in the string.
    let bytes = value.as_bytes();
    let digits = |from: usize| {
        let end = bytes[from..]
            .iter()
            .position(|b| !b.is_ascii_digit())
            .map_or(bytes.len(), |n| from + n);
        (end > from).then_some(end)
    };
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        let mut parts = [0u64; 3];
        let mut at = start;
        let mut ok = true;
        for (slot, part) in parts.iter_mut().enumerate() {
            let Some(end) = digits(at) else {
                ok = false;
                break;
            };
            *part = value[at..end].parse().unwrap_or(u64::MAX);
            at = end;
            if slot < 2 {
                if bytes.get(at) == Some(&b'.') {
                    at += 1;
                } else {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Some(parts);
        }
    }
    None
}

fn check_astroshot(host: &dyn Host) -> Check {
    Check::new(
        "astroshot",
        "astroshot binary version",
        true,
        Status::Pass,
        format!("astroshot {}", host.binary_version()),
        "Reinstall astroshot: https://github.com/ArchAstro/astroshots/releases",
    )
}

fn check_node(host: &dyn Host, range: &str) -> Check {
    let title = "Node.js version";
    let remediation = "nvm install 22.14.0 && nvm use 22.14.0";
    let version = match host.node_version() {
        Ok(version) => version,
        Err(reason) => {
            return Check::new(
                "node",
                title,
                false,
                Status::Warn,
                format!("{reason}; react and ink shots need it"),
                "Install Node.js 22.14.0 or newer: https://nodejs.org/en/download",
            );
        }
    };
    let (Some(minimum), Some(current)) = (parse_version(range), parse_version(&version)) else {
        return Check::new(
            "node",
            title,
            false,
            Status::Warn,
            format!("could not compare {version} with \"{range}\""),
            "Install Node.js 22.14.0 or newer: https://nodejs.org/en/download",
        );
    };
    let ok = current >= minimum;
    Check::new(
        "node",
        title,
        false,
        if ok { Status::Pass } else { Status::Fail },
        if ok {
            format!("{version} satisfies {range}")
        } else {
            format!("{version} is older than required {range}")
        },
        remediation,
    )
}

fn check_node_helper(host: &dyn Host) -> Check {
    let title = "Node helper (react and ink shots)";
    let remediation = "Set ASTROSHOT_NODE_HELPER to the helper.mjs shipped with astroshot";
    match host.node_helper() {
        Ok(path) => Check::new(
            "node-helper",
            title,
            false,
            Status::Pass,
            path.display().to_string(),
            remediation,
        ),
        Err(reason) => Check::new(
            "node-helper",
            title,
            false,
            Status::Warn,
            reason,
            remediation,
        ),
    }
}

fn check_demo_fixtures() -> Check {
    let title = "Bundled demo payload";
    let remediation = "Reinstall astroshot: https://github.com/ArchAstro/astroshots/releases";
    match load_demo_fixtures(None) {
        Ok(index) => {
            let shots = index.shots();
            let movies = shots
                .iter()
                .filter(|shot| match shot.get("video") {
                    None | Some(Value::Null) | Some(Value::Bool(false)) => false,
                    Some(Value::String(text)) => !text.is_empty(),
                    Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
                    Some(_) => true,
                })
                .count();
            Check::new(
                "demo-fixtures",
                title,
                true,
                Status::Pass,
                format!(
                    "{} fixtures ({movies} movie) ready for \"astroshot demo\"",
                    shots.len()
                ),
                remediation,
            )
        }
        Err(error) => Check::new(
            "demo-fixtures",
            title,
            true,
            Status::Fail,
            error.to_string(),
            remediation,
        ),
    }
}

/// `read_configuration` is the test seam: the tool-failure path is otherwise
/// only reachable by breaking the host's `/usr/bin`.
pub fn check_watch_coverage(
    project_path: &str,
    platform: &str,
    read_configuration: &dyn Fn() -> WatchConfiguration,
) -> Check {
    let id = "watch-roots";
    let title = "Watched folder covers this project";
    if platform != MACOS {
        return Check::new(
            id,
            title,
            false,
            Status::Skip,
            "the Astroshots app is macOS-only; .astroshot files are still written",
            "Run captures on macOS to review them in the Astroshots tray",
        );
    }

    let configuration = read_configuration();
    let coverage = evaluate_watch_coverage(project_path, &configuration, None);
    let source = if configuration.source.as_deref() == Some("plist-file") {
        format!(
            " (read from {}; cfprefsd unavailable)",
            configuration.plist_path.as_deref().unwrap_or("undefined")
        )
    } else {
        String::new()
    };
    let legacy = if configuration.used_legacy_key {
        " via legacy watchRoot key"
    } else {
        ""
    };
    let project = coverage.project_path.clone().unwrap_or_default();

    let mut check = match coverage.state {
        CoverageState::InsideRoot => Check::new(
            id,
            title,
            true,
            Status::Pass,
            format!(
                "{project} is inside {}{legacy}{source}",
                coverage.matched_root.as_deref().unwrap_or_default()
            ),
            "Astroshots menu-bar icon → gear → Add folders…",
        ),
        CoverageState::SetupIncomplete => Check::new(
            id,
            title,
            true,
            Status::Fail,
            "first-launch folder setup has not completed, so Astroshots watches nothing yet",
            "open -a Astroshots   # then choose the folder that holds your projects",
        ),
        CoverageState::OutsideRoots => Check::new(
            id,
            title,
            true,
            Status::Fail,
            format!(
                "{project} is outside every watched folder ({})",
                coverage.roots.join(", ")
            ),
            "Astroshots menu-bar icon → gear → Add folders… → add a parent folder of this project",
        ),
        // The configuration could not be read. Say exactly that: claiming
        // "setup incomplete" here would tell a correctly configured user to
        // redo work they already did.
        CoverageState::Unknown | CoverageState::Unsupported => {
            let tools = preference_tools_available(Some(platform));
            let tool_problem =
                coverage.reason.as_deref() == Some("tool-unavailable") || !tools.available;
            let missing = if tools.missing.is_empty() {
                String::new()
            } else {
                format!(" (missing {})", tools.missing.join(", "))
            };
            Check::new(
                id,
                title,
                true,
                Status::Warn,
                if tool_problem {
                    format!(
                        "watch coverage is UNKNOWN, not unconfigured: could not read {ASTROSHOTS_DOMAIN} because the macOS preference tools are unavailable{missing}"
                    )
                } else {
                    format!(
                        "watch coverage is UNKNOWN: could not read {ASTROSHOTS_DOMAIN} preferences ({})",
                        coverage.reason.as_deref().unwrap_or("unknown")
                    )
                },
                if tool_problem {
                    "Re-run with /usr/bin on PATH — doctor needs /usr/bin/defaults and /usr/bin/plutil"
                } else {
                    "Install and launch Astroshots once: https://github.com/ArchAstro/astroshots/releases"
                },
            )
        }
    };
    check.data = Some(coverage);
    check
}

fn find_installed_app(host: &dyn Host) -> Option<String> {
    let mut candidates = vec![PathBuf::from("/Applications/Astroshots.app")];
    candidates.push(
        host.home_dir()
            .unwrap_or_default()
            .join("Applications")
            .join("Astroshots.app"),
    );
    for candidate in candidates {
        if host.exists(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    let query = format!("kMDItemCFBundleIdentifier == '{ASTROSHOTS_DOMAIN}'");
    let found = host.run("/usr/bin/mdfind", &["-0", &query]);
    if found.status == Some(0) && !found.stdout.is_empty() {
        return found
            .stdout
            .split('\0')
            .map(|entry| entry.trim_matches(is_js_whitespace))
            .find(|entry| entry.ends_with(".app") && host.exists(Path::new(entry)))
            .map(str::to_string);
    }
    None
}

fn app_version(host: &dyn Host, app_path: &str) -> Option<String> {
    let plist = Path::new(app_path).join("Contents").join("Info.plist");
    if !host.exists(&plist) {
        return None;
    }
    let plist = plist.to_string_lossy().into_owned();
    let result = host.run(
        "/usr/bin/plutil",
        &[
            "-extract",
            "CFBundleShortVersionString",
            "raw",
            "-o",
            "-",
            &plist,
        ],
    );
    if result.status != Some(0) {
        return None;
    }
    let version = result.stdout.trim_matches(is_js_whitespace);
    (!version.is_empty()).then(|| version.to_string())
}

fn check_app(host: &dyn Host, platform: &str) -> Check {
    let id = "app";
    let title = "Astroshots app installed";
    if platform != MACOS {
        return Check::new(
            id,
            title,
            false,
            Status::Skip,
            "macOS-only review app",
            "Review captured files directly on this platform",
        );
    }
    let Some(app_path) = find_installed_app(host) else {
        return Check::new(
            id,
            title,
            true,
            Status::Fail,
            "no Astroshots.app found in /Applications or ~/Applications",
            "Download the latest DMG: https://github.com/ArchAstro/astroshots/releases",
        );
    };
    let version = app_version(host, &app_path);
    Check::new(
        id,
        title,
        true,
        Status::Pass,
        match version {
            Some(version) => format!("{app_path} ({version})"),
            None => app_path,
        },
        "open -a Astroshots",
    )
}

fn check_app_running(host: &dyn Host, platform: &str) -> Check {
    let id = "app-running";
    let title = "Astroshots app running";
    if platform != MACOS {
        return Check::new(
            id,
            title,
            false,
            Status::Skip,
            "macOS-only review app",
            "Review captured files directly on this platform",
        );
    }
    let result = host.run("/usr/bin/pgrep", &["-x", APP_PROCESS_NAME]);
    let pids = result.stdout.trim_matches(is_js_whitespace);
    let running = result.status == Some(0) && !pids.is_empty();
    Check::new(
        id,
        title,
        false,
        if running { Status::Pass } else { Status::Warn },
        if running {
            format!(
                "pid {} (menu-bar only, no Dock icon)",
                pids.split(is_js_whitespace)
                    .filter(|pid| !pid.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            "not running, so new captures will not stream or flash an overlay".to_string()
        },
        "open -a Astroshots",
    )
}

fn check_chromium(host: &dyn Host) -> Check {
    let (status, detail) = match host.chrome() {
        Ok(path) => (Status::Pass, path.display().to_string()),
        Err(reason) => (
            Status::Warn,
            format!(
                "{} — needed for react/ink/pty stills and browser movies (not for \"astroshot demo\")",
                reason
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('.')
            ),
        ),
    };
    let mut check = Check::new(
        "chromium",
        "Managed Chromium runtime",
        false,
        status,
        detail,
        "astroshot install-browser   # add --with-deps on Linux CI",
    );
    check.remediation_first = true;
    check
}

fn check_screen_recording(host: &dyn Host, platform: &str, skip: bool) -> Check {
    let id = "screen-recording";
    let title = "Screen Recording permission (movie --source desktop.window)";
    if platform != MACOS {
        return Check::new(
            id,
            title,
            false,
            Status::Skip,
            "desktop.window is macOS-only; use --source browser, pty, or frames",
            "astroshot movie which-source \"<intent>\"",
        );
    }
    if skip {
        return Check::new(
            id,
            title,
            false,
            Status::Skip,
            "skipped with --skip-screen",
            "astroshot movie check-screen-access",
        );
    }

    let result = host.screen_access();
    let report = serde_json::from_str::<Value>(&result.stdout)
        .ok()
        .filter(|value| !matches!(value, Value::Null | Value::Bool(false)));
    let Some(report) = report else {
        let first_line = result.stderr.trim_matches(is_js_whitespace);
        let first_line = first_line.split('\n').next().unwrap_or_default();
        let reason = if first_line.is_empty() {
            format!(
                "status {}",
                result
                    .status
                    .map_or_else(|| "null".to_string(), |code| code.to_string())
            )
        } else {
            first_line.to_string()
        };
        return Check::new(
            id,
            title,
            false,
            Status::Warn,
            format!("could not probe Screen Recording ({reason})"),
            "xcode-select --install && astroshot movie check-screen-access",
        );
    };
    let granted = report.get("granted").is_some_and(truthy);
    let enable_app = report
        .get("enableApp")
        .map_or_else(|| "undefined".to_string(), js_to_string);
    Check::new(
        id,
        title,
        false,
        if granted { Status::Pass } else { Status::Warn },
        if granted {
            format!("granted for {enable_app}")
        } else {
            format!("denied — enable \"{enable_app}\" then quit and reopen it")
        },
        if granted {
            "astroshot movie check-screen-access"
        } else {
            "astroshot movie open-screen-settings"
        },
    )
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

fn js_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() && c != '\u{85}' || c == '\u{FEFF}'
}

/// `collectDoctorChecks`. The TS order is kept; `astroshot`, `node-helper`
/// are the only additions.
pub fn collect_doctor_checks(
    host: &dyn Host,
    project_path: &str,
    platform: &str,
    skip_screen: bool,
) -> Vec<Check> {
    vec![
        check_astroshot(host),
        check_node(host, NODE_RANGE),
        check_node_helper(host),
        check_demo_fixtures(),
        check_app(host, platform),
        check_app_running(host, platform),
        check_watch_coverage(project_path, platform, &|| host.read_watch_configuration()),
        check_chromium(host),
        check_screen_recording(host, platform, skip_screen),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorSummary {
    pub ok: bool,
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn summarize_doctor(checks: &[Check]) -> DoctorSummary {
    let failures: Vec<String> = checks
        .iter()
        .filter(|check| check.required && check.status == Status::Fail)
        .map(|check| check.id.to_string())
        .collect();
    let warnings = checks
        .iter()
        .filter(|check| check.status == Status::Warn)
        .map(|check| check.id.to_string())
        .collect();
    DoctorSummary {
        ok: failures.is_empty(),
        failures,
        warnings,
    }
}

/// `runDoctor` against the real machine. Errors (bad flags) are returned; the
/// caller prints them to stderr and exits 1.
pub fn run_doctor(argv: &[String], log: &mut dyn FnMut(&str)) -> Result<i32> {
    run_doctor_with(&SystemHost, argv, log)
}

pub fn run_doctor_with(host: &dyn Host, argv: &[String], log: &mut dyn FnMut(&str)) -> Result<i32> {
    let mut root: Option<String> = None;
    let (mut json_output, mut skip_screen) = (false, false);
    let mut index = 0;
    while index < argv.len() {
        let token = argv[index].as_str();
        match token {
            "-h" | "--help" | "help" => {
                log(&doctor_help());
                return Ok(0);
            }
            "--json" => json_output = true,
            "--skip-screen" => skip_screen = true,
            "--root" => {
                let value = argv
                    .get(index + 1)
                    .filter(|v| !v.is_empty() && !v.starts_with('-'));
                let Some(value) = value else {
                    bail!("--root requires a value");
                };
                root = Some(value.clone());
                index += 1;
            }
            _ => bail!("Unknown doctor argument: {token}"),
        }
        index += 1;
    }

    let project_path = resolve_project_path(host, root.as_deref());
    let platform = host.platform();
    let checks = collect_doctor_checks(host, &project_path, &platform, skip_screen);
    let summary = summarize_doctor(&checks);

    if json_output {
        let mut report = Map::new();
        report.insert("project".into(), json!(project_path));
        report.insert("ok".into(), json!(summary.ok));
        report.insert("failures".into(), json!(summary.failures));
        report.insert("warnings".into(), json!(summary.warnings));
        report.insert(
            "checks".into(),
            Value::Array(checks.iter().map(Check::to_value).collect()),
        );
        log(&serde_json::to_string_pretty(&Value::Object(report))?);
        return Ok(if summary.ok { 0 } else { 1 });
    }

    log(&format!("astroshot doctor — {project_path}"));
    log("");
    for check in &checks {
        let scope = if check.required {
            "required"
        } else {
            "optional"
        };
        log(&format!(
            "{}  {} [{scope}]",
            check.status.symbol(),
            check.title
        ));
        log(&format!("      {}", check.detail));
        if matches!(check.status, Status::Fail | Status::Warn) {
            log(&format!("      fix: {}", check.remediation));
        }
    }
    log("");
    if summary.ok {
        let count = summary.warnings.len();
        log(&if count > 0 {
            format!(
                "All required checks pass ({count} optional warning{} above).",
                if count == 1 { "" } else { "s" }
            )
        } else {
            "All checks pass. Try: astroshot demo".to_string()
        });
        return Ok(0);
    }
    let count = summary.failures.len();
    log(&format!(
        "{count} required check{} failed: {}",
        if count == 1 { "" } else { "s" },
        summary.failures.join(", ")
    ));
    log("Run the fix line under each failure, then re-run: astroshot doctor");
    Ok(1)
}

/// `path.resolve`: join onto the cwd when relative, then normalize lexically.
fn resolve_path(cwd: &Path, path: &str) -> String {
    let joined = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

fn resolve_project_path(host: &dyn Host, explicit_root: Option<&str>) -> String {
    if let Some(root) = explicit_root.filter(|root| !root.is_empty()) {
        return resolve_path(&host.current_dir(), root);
    }
    let output = host.run("git", &["rev-parse", "--show-toplevel"]);
    if output.status == Some(0) {
        return output.stdout.trim_matches(is_js_whitespace).to_string();
    }
    host.current_dir().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[derive(Default)]
    struct FakeHost {
        platform: String,
        home: Option<PathBuf>,
        cwd: PathBuf,
        existing: HashSet<PathBuf>,
        /// (program, first arg) -> output
        commands: Vec<(String, Option<String>, CommandOutput)>,
        node: Option<std::result::Result<String, String>>,
        helper: Option<std::result::Result<PathBuf, String>>,
        chrome: Option<std::result::Result<PathBuf, String>>,
        configuration: Option<WatchConfiguration>,
        screen: CommandOutput,
    }

    impl FakeHost {
        fn macos() -> Self {
            FakeHost {
                platform: "macos".into(),
                home: Some(PathBuf::from("/Users/tester")),
                cwd: PathBuf::from("/Users/tester/proj"),
                node: Some(Ok("22.14.0".into())),
                helper: Some(Ok(PathBuf::from("/opt/astroshot/node-helper/helper.mjs"))),
                chrome: Some(Ok(PathBuf::from("/Applications/Google Chrome.app/x"))),
                configuration: Some(readable(&["/Users/tester/proj"], true)),
                screen: CommandOutput {
                    status: Some(0),
                    stdout: r#"{"granted":true,"enableApp":"Terminal"}"#.into(),
                    stderr: String::new(),
                },
                ..Default::default()
            }
        }

        fn linux() -> Self {
            FakeHost {
                platform: "linux".into(),
                ..FakeHost::macos()
            }
        }

        fn with_command(mut self, program: &str, first: Option<&str>, out: CommandOutput) -> Self {
            self.commands
                .push((program.into(), first.map(String::from), out));
            self
        }

        fn with_file(mut self, path: &str) -> Self {
            self.existing.insert(PathBuf::from(path));
            self
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    impl Host for FakeHost {
        fn platform(&self) -> String {
            self.platform.clone()
        }
        fn home_dir(&self) -> Option<PathBuf> {
            self.home.clone()
        }
        fn current_dir(&self) -> PathBuf {
            self.cwd.clone()
        }
        fn exists(&self, path: &Path) -> bool {
            self.existing.contains(path)
        }
        fn run(&self, program: &str, args: &[&str]) -> CommandOutput {
            self.commands
                .iter()
                .find(|(p, first, _)| {
                    p == program && first.as_deref().is_none_or(|f| args.first() == Some(&f))
                })
                .map(|(_, _, out)| out.clone())
                .unwrap_or(CommandOutput {
                    status: None,
                    stdout: String::new(),
                    stderr: "spawn failed".into(),
                })
        }
        fn node_version(&self) -> std::result::Result<String, String> {
            self.node.clone().unwrap()
        }
        fn node_helper(&self) -> std::result::Result<PathBuf, String> {
            self.helper.clone().unwrap()
        }
        fn chrome(&self) -> std::result::Result<PathBuf, String> {
            self.chrome.clone().unwrap()
        }
        fn read_watch_configuration(&self) -> WatchConfiguration {
            self.configuration.clone().unwrap()
        }
        fn screen_access(&self) -> CommandOutput {
            self.screen.clone()
        }
        fn binary_version(&self) -> String {
            "1.2.3".into()
        }
    }

    fn readable(roots: &[&str], first_run: bool) -> WatchConfiguration {
        WatchConfiguration {
            available: true,
            source: Some("cfprefsd".into()),
            roots: roots.iter().map(|r| (*r).to_string()).collect(),
            has_completed_first_run_setup: first_run,
            ..Default::default()
        }
    }

    fn unreadable(reason: &str) -> WatchConfiguration {
        WatchConfiguration {
            available: false,
            reason: Some(reason.into()),
            error: Some("spawn /usr/bin/plutil ENOENT".into()),
            ..Default::default()
        }
    }

    fn collect(host: &FakeHost, skip_screen: bool) -> Vec<Check> {
        collect_doctor_checks(host, "/Users/tester/proj", &host.platform(), skip_screen)
    }

    fn run(host: &FakeHost, args: &[&str]) -> (i32, Vec<String>) {
        let argv: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        let mut lines = Vec::new();
        let code = run_doctor_with(host, &argv, &mut |l| lines.push(l.to_string())).unwrap();
        (code, lines)
    }

    // ---- ported from demo-doctor.test.mjs

    #[test]
    fn doctor_reports_every_check_with_pass_fail_and_a_remediation_line() {
        let checks = collect(&FakeHost::macos(), true);
        let ids: Vec<&str> = checks.iter().map(|c| c.id).collect();
        for id in [
            "node",
            "watch-roots",
            "app",
            "app-running",
            "chromium",
            "screen-recording",
        ] {
            assert!(ids.contains(&id), "doctor is missing the {id} check");
        }
        for check in &checks {
            assert!(!check.title.is_empty());
            assert!(!check.detail.is_empty(), "{} has no detail", check.id);
            assert!(
                !check.remediation.is_empty(),
                "{} has no remediation",
                check.id
            );
        }
    }

    #[test]
    fn doctor_exit_status_follows_required_checks_only() {
        let make = |id, required, status| Check::new(id, "t", required, status, "d", "r");
        assert!(
            summarize_doctor(&[
                make("node", true, Status::Pass),
                make("chromium", false, Status::Warn),
            ])
            .ok
        );
        let failed = summarize_doctor(&[
            make("node", true, Status::Pass),
            make("watch-roots", true, Status::Fail),
            make("chromium", false, Status::Fail),
        ]);
        assert!(!failed.ok);
        assert_eq!(failed.failures, vec!["watch-roots".to_string()]);
    }

    #[test]
    fn doctor_prints_a_fix_line_for_each_failure_and_never_mutates_state() {
        // A macOS machine where the project is not covered and nothing else
        // is installed: every FAIL/WARN line needs exactly one fix line.
        let mut host = FakeHost::macos();
        host.configuration = Some(readable(&["/Users/tester/watched"], true));
        host.chrome = Some(Err("No Chrome or Chromium found.".into()));
        let (code, lines) = run(&host, &["--skip-screen", "--root", "/Users/tester/proj"]);
        let text = lines.join("\n");
        assert_eq!(code, 1);
        assert!(text.contains("astroshot doctor — /Users/tester/proj"));
        assert!(text.contains("Node.js version [optional]"));
        assert!(text.contains("Watched folder covers this project [required]"));
        assert!(text.contains("Managed Chromium runtime [optional]"));
        let failure_lines = lines
            .iter()
            .filter(|l| l.starts_with("FAIL") || l.starts_with("WARN"))
            .count();
        let fix_lines = lines
            .iter()
            .filter(|l| l.trim_start().starts_with("fix:"))
            .count();
        assert_eq!(fix_lines, failure_lines);
        assert!(failure_lines > 0);
        assert!(text.contains("required check") && text.contains("failed"));
    }

    #[test]
    fn doctor_watch_tag_is_optional_off_macos() {
        let (_, lines) = run(&FakeHost::linux(), &["--skip-screen", "--root", "/p"]);
        assert!(
            lines
                .iter()
                .any(|l| l == "SKIP  Watched folder covers this project [optional]")
        );
    }

    #[test]
    fn doctor_json_is_machine_readable_and_reports_the_project_it_checked() {
        let host = FakeHost::linux();
        let (code, lines) = run(&host, &["--json", "--skip-screen", "--root", "/work/proj"]);
        let report: Value = serde_json::from_str(&lines.join("\n")).unwrap();
        assert_eq!(report["project"], "/work/proj");
        assert_eq!(report["ok"], json!(code == 0));
        assert!(report["checks"].is_array());
        assert!(report["failures"].is_array());
        let keys: Vec<&str> = report
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["project", "ok", "failures", "warnings", "checks"]);
    }

    #[test]
    fn doctor_never_claims_setup_incomplete_when_the_tools_are_unreadable() {
        // Regression: when `defaults`/`plutil` cannot answer, every key read
        // as missing and doctor told correctly configured users to redo
        // first-run setup. Unknown must stay unknown.
        let check = check_watch_coverage("/Users/tester/proj", "macos", &|| {
            unreadable("tool-unavailable")
        });
        assert_eq!(check.id, "watch-roots");
        assert_ne!(check.status, Status::Fail);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("UNKNOWN"));
        let lower = check.detail.to_lowercase();
        for forbidden in [
            "setup has not completed",
            "no watched folders",
            "outside every watched folder",
        ] {
            assert!(!lower.contains(forbidden), "{forbidden}: {}", check.detail);
        }
        assert!(check.remediation.contains("/usr/bin"));
        assert_eq!(check.data.unwrap().state, CoverageState::Unknown);
    }

    #[test]
    fn doctor_still_reports_the_real_three_states_when_the_config_is_readable() {
        let never_set_up =
            check_watch_coverage("/Users/tester/proj", "macos", &|| readable(&[], false));
        assert_eq!(never_set_up.status, Status::Fail);
        assert!(never_set_up.detail.contains("first-launch folder setup"));

        let watched = || readable(&["/Users/tester/watched"], true);
        let outside = check_watch_coverage("/Users/tester/elsewhere", "macos", &watched);
        assert_eq!(outside.status, Status::Fail);
        assert!(outside.detail.contains("outside every watched folder"));

        let inside = check_watch_coverage("/Users/tester/watched/app", "macos", &watched);
        assert_eq!(inside.status, Status::Pass);
    }

    #[test]
    fn an_unreadable_snapshot_never_becomes_an_empty_watch_configuration_in_doctor() {
        // The doctor half of the TS case: an unreadable configuration is a
        // warning, never "setup has not completed". (The extraction half lives
        // in mac_preferences' tests.)
        let configuration = WatchConfiguration {
            available: false,
            reason: Some("tool-unavailable".into()),
            ..Default::default()
        };
        let check = check_watch_coverage("/Users/tester/proj", "macos", &|| configuration.clone());
        assert_eq!(check.status, Status::Warn);
        assert!(
            !check
                .detail
                .to_lowercase()
                .contains("setup has not completed")
        );
    }

    // ---- per-check pass/fail text

    #[test]
    fn astroshot_check_reports_the_binary_version() {
        let check = check_astroshot(&FakeHost::macos());
        assert_eq!(check.status, Status::Pass);
        assert!(check.required);
        assert_eq!(check.detail, "astroshot 1.2.3");
    }

    #[test]
    fn node_check_text_for_pass_fail_warn_and_missing() {
        let mut host = FakeHost::macos();
        let pass = check_node(&host, NODE_RANGE);
        assert_eq!(pass.status, Status::Pass);
        assert_eq!(pass.detail, "22.14.0 satisfies >=22.14.0");
        assert!(!pass.required);

        host.node = Some(Ok("20.1.0".into()));
        let old = check_node(&host, NODE_RANGE);
        assert_eq!(old.status, Status::Fail);
        assert_eq!(old.detail, "20.1.0 is older than required >=22.14.0");
        assert_eq!(old.remediation, "nvm install 22.14.0 && nvm use 22.14.0");

        host.node = Some(Ok("garbage".into()));
        let unparsable = check_node(&host, NODE_RANGE);
        assert_eq!(unparsable.status, Status::Warn);
        assert_eq!(
            unparsable.detail,
            "could not compare garbage with \">=22.14.0\""
        );

        host.node = Some(Err("node not found".into()));
        let missing = check_node(&host, NODE_RANGE);
        assert_eq!(missing.status, Status::Warn);
        assert_eq!(
            missing.detail,
            "node not found; react and ink shots need it"
        );
    }

    #[test]
    fn parse_version_finds_the_first_triple_anywhere() {
        assert_eq!(parse_version(">=22.14.0"), Some([22, 14, 0]));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("x 1.2 then 3.4.5"), Some([3, 4, 5]));
    }

    #[test]
    fn node_helper_check_text() {
        let mut host = FakeHost::macos();
        let pass = check_node_helper(&host);
        assert_eq!(pass.status, Status::Pass);
        assert_eq!(pass.detail, "/opt/astroshot/node-helper/helper.mjs");
        host.helper = Some(Err("could not find the astroshot Node helper".into()));
        let warn = check_node_helper(&host);
        assert_eq!(warn.status, Status::Warn);
        assert_eq!(warn.detail, "could not find the astroshot Node helper");
        assert!(!warn.required);
    }

    #[test]
    fn demo_fixtures_check_passes_with_the_embedded_payload() {
        let check = check_demo_fixtures();
        assert_eq!(check.status, Status::Pass);
        assert!(
            check.detail.ends_with("ready for \"astroshot demo\""),
            "{}",
            check.detail
        );
        assert!(check.detail.contains("fixtures ("));
    }

    #[test]
    fn app_check_skips_off_macos() {
        let check = check_app(&FakeHost::linux(), "linux");
        assert_eq!(check.status, Status::Skip);
        assert_eq!(check.detail, "macOS-only review app");
        assert!(!check.required);
    }

    #[test]
    fn app_check_fails_when_no_app_is_installed() {
        let host = FakeHost::macos().with_command(
            "/usr/bin/mdfind",
            None,
            CommandOutput {
                status: Some(0),
                ..Default::default()
            },
        );
        let check = check_app(&host, "macos");
        assert_eq!(check.status, Status::Fail);
        assert_eq!(
            check.detail,
            "no Astroshots.app found in /Applications or ~/Applications"
        );
        assert!(check.required);
    }

    #[test]
    fn app_check_passes_with_version_from_plutil() {
        let host = FakeHost::macos()
            .with_file("/Applications/Astroshots.app")
            .with_file("/Applications/Astroshots.app/Contents/Info.plist")
            .with_command("/usr/bin/plutil", None, ok("0.9.1\n"));
        let check = check_app(&host, "macos");
        assert_eq!(check.status, Status::Pass);
        assert_eq!(check.detail, "/Applications/Astroshots.app (0.9.1)");
        assert_eq!(check.remediation, "open -a Astroshots");
    }

    #[test]
    fn app_check_finds_the_user_applications_copy_without_a_version() {
        let host = FakeHost::macos().with_file("/Users/tester/Applications/Astroshots.app");
        let check = check_app(&host, "macos");
        assert_eq!(check.detail, "/Users/tester/Applications/Astroshots.app");
    }

    #[test]
    fn app_check_falls_back_to_spotlight() {
        let host = FakeHost::macos()
            .with_file("/Volumes/x/Astroshots.app")
            .with_command(
                "/usr/bin/mdfind",
                Some("-0"),
                ok("/nowhere/Other.app\0/Volumes/x/Astroshots.app\0"),
            );
        let check = check_app(&host, "macos");
        assert_eq!(check.status, Status::Pass);
        assert_eq!(check.detail, "/Volumes/x/Astroshots.app");
    }

    #[test]
    fn app_running_check_text() {
        let skip = check_app_running(&FakeHost::linux(), "linux");
        assert_eq!(skip.status, Status::Skip);

        let running = FakeHost::macos().with_command("/usr/bin/pgrep", None, ok("12\n345\n"));
        let check = check_app_running(&running, "macos");
        assert_eq!(check.status, Status::Pass);
        assert_eq!(check.detail, "pid 12, 345 (menu-bar only, no Dock icon)");
        assert!(!check.required);

        let stopped = FakeHost::macos().with_command(
            "/usr/bin/pgrep",
            None,
            CommandOutput {
                status: Some(1),
                ..Default::default()
            },
        );
        let check = check_app_running(&stopped, "macos");
        assert_eq!(check.status, Status::Warn);
        assert_eq!(
            check.detail,
            "not running, so new captures will not stream or flash an overlay"
        );
    }

    #[test]
    fn watch_roots_check_skips_off_macos() {
        let check = check_watch_coverage("/p", "linux", &|| unreachable!());
        assert_eq!(check.status, Status::Skip);
        assert!(!check.required);
        assert_eq!(
            check.detail,
            "the Astroshots app is macOS-only; .astroshot files are still written"
        );
        assert!(check.data.is_none());
    }

    #[test]
    fn watch_roots_pass_text_mentions_legacy_key_and_plist_source() {
        let check = check_watch_coverage("/Users/tester/watched/app", "macos", &|| {
            WatchConfiguration {
                available: true,
                source: Some("plist-file".into()),
                plist_path: Some("/Users/tester/Library/Preferences/a.plist".into()),
                roots: vec!["/Users/tester/watched".into()],
                used_legacy_key: true,
                has_completed_first_run_setup: true,
                ..Default::default()
            }
        });
        assert_eq!(check.status, Status::Pass);
        assert_eq!(
            check.detail,
            "/Users/tester/watched/app is inside /Users/tester/watched via legacy watchRoot key (read from /Users/tester/Library/Preferences/a.plist; cfprefsd unavailable)"
        );
        assert_eq!(
            check.remediation,
            "Astroshots menu-bar icon → gear → Add folders…"
        );
    }

    #[test]
    fn watch_roots_outside_text_lists_every_root() {
        let check = check_watch_coverage("/Users/tester/elsewhere", "macos", &|| {
            readable(&["/Users/tester/a", "/Users/tester/b"], true)
        });
        assert_eq!(
            check.detail,
            "/Users/tester/elsewhere is outside every watched folder (/Users/tester/a, /Users/tester/b)"
        );
    }

    #[test]
    fn watch_roots_unknown_without_tool_problem_names_the_reason() {
        // Preference tools exist on this host only on macOS; force the
        // non-tool branch through a reason that is not tool-unavailable.
        let check = check_watch_coverage("/p", "macos", &|| unreadable("domain-not-found"));
        assert_eq!(check.status, Status::Warn);
        if preference_tools_available(Some("macos")).available {
            assert_eq!(
                check.detail,
                "watch coverage is UNKNOWN: could not read ai.archastro.Astroshots preferences (domain-not-found)"
            );
            assert!(
                check
                    .remediation
                    .starts_with("Install and launch Astroshots once")
            );
        } else {
            assert!(check.detail.contains("preference tools are unavailable"));
        }
    }

    #[test]
    fn chromium_check_text_and_json_key_order() {
        let host = FakeHost::macos();
        let pass = check_chromium(&host);
        assert_eq!(pass.status, Status::Pass);
        assert_eq!(pass.detail, "/Applications/Google Chrome.app/x");
        let keys: Vec<String> = pass
            .to_value()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            ["id", "title", "required", "remediation", "status", "detail"]
        );

        let mut host = FakeHost::macos();
        host.chrome = Some(Err("No Chrome or Chromium found.\n  - $X".into()));
        let warn = check_chromium(&host);
        assert_eq!(warn.status, Status::Warn);
        assert_eq!(
            warn.detail,
            "No Chrome or Chromium found — needed for react/ink/pty stills and browser movies (not for \"astroshot demo\")"
        );
        assert_eq!(
            warn.remediation,
            "astroshot install-browser   # add --with-deps on Linux CI"
        );
    }

    #[test]
    fn screen_recording_check_text() {
        let host = FakeHost::macos();
        let skip = check_screen_recording(&FakeHost::linux(), "linux", false);
        assert_eq!(
            skip.detail,
            "desktop.window is macOS-only; use --source browser, pty, or frames"
        );
        let skipped = check_screen_recording(&host, "macos", true);
        assert_eq!(skipped.status, Status::Skip);
        assert_eq!(skipped.detail, "skipped with --skip-screen");

        let granted = check_screen_recording(&host, "macos", false);
        assert_eq!(granted.status, Status::Pass);
        assert_eq!(granted.detail, "granted for Terminal");
        assert_eq!(granted.remediation, "astroshot movie check-screen-access");

        let mut denied_host = FakeHost::macos();
        denied_host.screen = ok(r#"{"granted":false,"enableApp":"Terminal"}"#);
        let denied = check_screen_recording(&denied_host, "macos", false);
        assert_eq!(denied.status, Status::Warn);
        assert_eq!(
            denied.detail,
            "denied — enable \"Terminal\" then quit and reopen it"
        );
        assert_eq!(denied.remediation, "astroshot movie open-screen-settings");

        let mut broken = FakeHost::macos();
        broken.screen = CommandOutput {
            status: Some(3),
            stdout: "not json".into(),
            stderr: "swift: command not found\nmore".into(),
        };
        let warn = check_screen_recording(&broken, "macos", false);
        assert_eq!(warn.status, Status::Warn);
        assert_eq!(
            warn.detail,
            "could not probe Screen Recording (swift: command not found)"
        );
        broken.screen = CommandOutput {
            status: Some(3),
            ..Default::default()
        };
        assert_eq!(
            check_screen_recording(&broken, "macos", false).detail,
            "could not probe Screen Recording (status 3)"
        );
    }

    #[test]
    fn check_order_and_json_shape_match_the_ts_report() {
        let host = FakeHost::macos()
            .with_file("/Applications/Astroshots.app")
            .with_command("/usr/bin/pgrep", None, ok("7\n"));
        let (code, lines) = run(&host, &["--json", "--root", "/Users/tester/proj"]);
        assert_eq!(code, 0);
        let report: Value = serde_json::from_str(&lines.join("\n")).unwrap();
        let ids: Vec<&str> = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "astroshot",
                "node",
                "node-helper",
                "demo-fixtures",
                "app",
                "app-running",
                "watch-roots",
                "chromium",
                "screen-recording"
            ]
        );
        let watch = &report["checks"][6];
        let keys: Vec<&str> = watch
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "id",
                "title",
                "required",
                "status",
                "detail",
                "remediation",
                "data"
            ]
        );
        let data: Vec<&str> = watch["data"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(data, ["state", "matchedRoot", "projectPath", "roots"]);
        // pretty-printed with two-space indent, like JSON.stringify(_, null, 2)
        assert!(lines[0].starts_with("{\n  \"project\""));
    }

    #[test]
    fn human_report_summarizes_warnings_and_success() {
        let host = FakeHost::macos()
            .with_file("/Applications/Astroshots.app")
            .with_command("/usr/bin/pgrep", None, ok("7\n"));
        let (code, lines) = run(&host, &["--root", "/Users/tester/proj"]);
        assert_eq!(code, 0);
        assert_eq!(lines[0], "astroshot doctor — /Users/tester/proj");
        assert_eq!(lines[1], "");
        assert_eq!(lines[2], "PASS  astroshot binary version [required]");
        assert_eq!(lines[3], "      astroshot 1.2.3");
        assert_eq!(
            lines.last().unwrap(),
            "All checks pass. Try: astroshot demo"
        );

        let mut warned = FakeHost::macos().with_file("/Applications/Astroshots.app");
        warned.chrome = Some(Err("No Chrome or Chromium found.".into()));
        let (code, lines) = run(&warned, &["--root", "/Users/tester/proj", "--skip-screen"]);
        assert_eq!(code, 0);
        assert_eq!(
            lines.last().unwrap(),
            "All required checks pass (2 optional warnings above)."
        );
    }

    #[test]
    fn human_report_counts_required_failures() {
        let mut host = FakeHost::macos();
        host.configuration = Some(readable(&[], false));
        let (code, lines) = run(&host, &["--root", "/Users/tester/proj", "--skip-screen"]);
        assert_eq!(code, 1);
        let n = lines.len();
        assert_eq!(lines[n - 2], "2 required checks failed: app, watch-roots");
        assert_eq!(
            lines[n - 1],
            "Run the fix line under each failure, then re-run: astroshot doctor"
        );
    }

    #[test]
    fn argument_errors_and_help() {
        let host = FakeHost::linux();
        let err = |args: &[&str]| {
            let argv: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
            run_doctor_with(&host, &argv, &mut |_| {})
                .unwrap_err()
                .to_string()
        };
        assert_eq!(err(&["--root"]), "--root requires a value");
        assert_eq!(err(&["--root", "--json"]), "--root requires a value");
        assert_eq!(err(&["--bogus"]), "Unknown doctor argument: --bogus");
        let (code, lines) = run(&host, &["-h"]);
        assert_eq!(code, 0);
        assert!(lines[0].starts_with("astroshot doctor — diagnose Astroshots capture setup"));
    }

    #[test]
    fn project_path_resolution_prefers_root_then_git_then_cwd() {
        let git = FakeHost::linux().with_command("git", Some("rev-parse"), ok("/repo/top\n"));
        assert_eq!(resolve_project_path(&git, None), "/repo/top");
        assert_eq!(
            resolve_project_path(&git, Some("sub/../other")),
            "/Users/tester/proj/other"
        );
        assert_eq!(resolve_project_path(&git, Some("/abs/./x/")), "/abs/x");
        let no_git = FakeHost::linux();
        assert_eq!(resolve_project_path(&no_git, None), "/Users/tester/proj");
    }
}
