//! Binary-level tests for `astroshot demo` and `astroshot doctor`. Ports the
//! cases of `packages/astroshot/test/demo-doctor.test.mjs` that run the real
//! bin, against the built `astroshot` binary, and runs the contract scripts
//! `scripts/verify-demo-doctor.sh` uses (`assert-demo-manifest.mjs`,
//! `assert-doctor-report.mjs`) on its output.
//!
//! The in-process ports (fake host, injected snapshots) stay in
//! `src/bin/{demo,doctor,mac_preferences}.rs`.
//!
//! Doctor differs from the TS bin in these disclosed ways, asserted below:
//! - two extra checks, `astroshot` (required) and `node-helper`;
//! - `node` is optional (`required: false`): only React and Ink shots use it;
//! - `chromium` reports the Chrome `astroshot_engine::browser::find_chrome` finds,
//!   not Playwright's managed Chromium;
//! - `demo-fixtures` remediation points at the releases page instead of
//!   `npm install --global @archastro/astroshot`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use astroshot::bin::doctor::check_watch_coverage;
use astroshot_review::mac_preferences::{
    CoverageState, PreferenceDomain, PreferenceOptions, ReadWatchConfigurationOptions,
    evaluate_watch_coverage, preference_tools_available, read_watch_configuration,
};
use regex::Regex;
use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_astroshot");
const DEFAULT_DEMO_FEATURE: &str = "astroshot-demo";
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const RELEASES_REMEDIATION: &str =
    "Reinstall astroshot: https://github.com/ArchAstro/astroshots/releases";

/// Check ids in report order. TS has the same list without `astroshot` and
/// `node-helper`.
const CHECK_IDS: [&str; 9] = [
    "astroshot",
    "node",
    "node-helper",
    "demo-fixtures",
    "app",
    "app-running",
    "watch-roots",
    "chromium",
    "screen-recording",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

/// A temp directory that is its own git repository, like the TS
/// `temporaryProject` (realpath, then `git init -q .`).
struct Project {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn temporary_project() -> Project {
    let dir = tempfile::Builder::new()
        .prefix("astroshot-demo-")
        .tempdir()
        .unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    let _ = Command::new("git")
        .args(["init", "-q", "."])
        .current_dir(&path)
        .output();
    Project { _dir: dir, path }
}

fn run_in(cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn astroshot")
}

/// Empty `PATH` removes node, ffmpeg, git, and every other helper binary.
fn run_with_empty_path(cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .env("PATH", "")
        .output()
        .expect("spawn astroshot")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> Value {
    serde_json::from_str(&stdout(output)).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}):\n{}\n{}",
            stdout(output),
            stderr(output)
        )
    })
}

fn matches(pattern: &str, text: &str) -> bool {
    Regex::new(pattern).unwrap().is_match(text)
}

/// Run one of the repository's contract scripts with Node. Prints SKIP when
/// Node is missing; the Rust assertions around the call still run.
fn assert_contract_script(script: &str, argument: &Path) {
    let node = match astroshot_engine::node_helper::find_node() {
        Ok(node) => node,
        Err(error) => {
            common::skip(format!("scripts/{script}: {error}"));
            return;
        }
    };
    let output = Command::new(node)
        .arg(repo().join("scripts").join(script))
        .arg(argument)
        .output()
        .expect("spawn node");
    assert!(
        output.status.success(),
        "scripts/{script} rejected {}:\n{}",
        argument.display(),
        stderr(&output)
    );
}

fn check<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == id)
        .unwrap_or_else(|| panic!("doctor is missing the {id} check"))
}

// ---------------------------------------------------------------- demo

#[test]
fn demo_writes_stills_a_movie_pair_and_a_contract_valid_manifest() {
    let project = temporary_project();
    let result = run_in(&project.path, &["demo"]);
    assert_eq!(result.status.code(), Some(0), "{}", stderr(&result));

    let feature_directory = project.path.join(".astroshot").join(DEFAULT_DEMO_FEATURE);
    let manifest: Value =
        serde_json::from_slice(&fs::read(feature_directory.join("manifest.json")).unwrap())
            .unwrap();

    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["feature"], DEFAULT_DEMO_FEATURE);
    let run_id = manifest["run_id"].as_str().unwrap();
    assert!(
        matches(r"^astroshot-demo-\d{8}T\d{6}Z-\d+-\d{4}$", run_id),
        "{run_id}"
    );
    assert!(["running", "pass", "fail", "idle"].contains(&manifest["status"].as_str().unwrap()));
    let shots = manifest["shots"].as_array().unwrap();
    assert!(shots.len() >= 2);

    for shot in shots {
        let file = shot["file"].as_str().unwrap();
        assert_eq!(Path::new(file).file_name().unwrap().to_str(), Some(file));
        assert!(matches(r"^\d{4}-[a-z0-9-]+\.png$", file), "{file}");
        assert_eq!(shot["id"], &file[..4]);
        assert_eq!(shot["slug"], file[5..].trim_end_matches(".png"));
        assert!(!shot["title"].as_str().unwrap().is_empty());
        assert!(!shot["description"].as_str().unwrap().is_empty());
        let captured_at = shot["captured_at"].as_str().unwrap();
        assert!(
            chrono::DateTime::parse_from_rfc3339(captured_at).is_ok(),
            "{captured_at}"
        );
        let image = feature_directory.join(file);
        assert!(image.exists(), "missing {file}");
        assert_eq!(
            fs::read(&image).unwrap()[..8],
            PNG_MAGIC,
            "{file} is not a PNG"
        );
    }

    let stills: Vec<&Value> = shots.iter().filter(|s| s.get("video").is_none()).collect();
    let movies: Vec<&Value> = shots.iter().filter(|s| s.get("video").is_some()).collect();
    assert!(!stills.is_empty(), "demo must include at least one still");
    assert_eq!(movies.len(), 1, "demo must include exactly one movie");

    let movie = movies[0];
    assert_eq!(movie["kind"], "movie");
    let video = movie["video"].as_str().unwrap();
    assert_eq!(Path::new(video).file_name().unwrap().to_str(), Some(video));
    assert!(matches(r"\.(webm|mp4|mov)$", video), "{video}");
    // The poster keeps the still-image contract so review stays hash-keyed.
    assert_eq!(
        Path::new(video).file_stem(),
        Path::new(movie["file"].as_str().unwrap()).file_stem(),
        "movie video must be the poster's sibling"
    );
    let video_path = feature_directory.join(video);
    assert!(video_path.exists());
    assert!(fs::metadata(&video_path).unwrap().len() > 0);
    assert!(movie["duration_ms"].is_number());
    assert!(movie["duration_ms"].as_f64().unwrap() > 0.0);
    assert!(
        ["browser", "pty", "desktop.window", "frames"].contains(&movie["source"].as_str().unwrap())
    );
    for chapter in movie
        .get("chapters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        assert!(matches(r"^[a-z0-9-]+$", chapter["slug"].as_str().unwrap()));
        assert!(chapter["t_ms"].is_number());
    }

    // No temp artifacts leak from the atomic writes.
    let leftovers: Vec<String> = fs::read_dir(&feature_directory)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert_eq!(leftovers, Vec::<String>::new());

    let out = stdout(&result);
    assert!(out.contains("0001-welcome.png"), "{out}");
    assert!(out.contains("manifest.json"), "{out}");

    // The same contract check scripts/verify-demo-doctor.sh runs.
    assert_contract_script("assert-demo-manifest.mjs", &feature_directory);
}

#[test]
fn demo_needs_no_chromium_no_ffmpeg_and_no_user_assets() {
    let project = temporary_project();
    let empty_browsers = tempfile::Builder::new()
        .prefix("astroshot-nobrowser-")
        .tempdir()
        .unwrap();

    let result = Command::new(BIN)
        .args(["demo", "--json"])
        .current_dir(&project.path)
        .env("PLAYWRIGHT_BROWSERS_PATH", empty_browsers.path())
        // The Rust binary's own Chrome and Node lookups get nothing either.
        .env("ASTROSHOT_CHROME", empty_browsers.path().join("no-chrome"))
        // Empty PATH removes ffmpeg, git, node, and every other helper binary.
        .env("PATH", "")
        .output()
        .expect("spawn astroshot");

    assert_eq!(result.status.code(), Some(0), "{}", stderr(&result));
    let report = json(&result);
    assert_eq!(
        report["root"].as_str().map(Path::new),
        Some(project.path.as_path())
    );
    let files = report["files"].as_array().unwrap();
    assert!(files.len() >= 4);
    for file in files {
        assert!(Path::new(file.as_str().unwrap()).exists(), "{file}");
    }
    // The zero-prerequisite output still meets the manifest contract.
    assert_contract_script(
        "assert-demo-manifest.mjs",
        &project.path.join(".astroshot").join(DEFAULT_DEMO_FEATURE),
    );
}

#[test]
fn demo_honors_feature_and_rejects_unsupported_input() {
    let project = temporary_project();
    let named = run_in(&project.path, &["demo", "--feature", "quickstart-proof"]);
    assert_eq!(named.status.code(), Some(0), "{}", stderr(&named));
    assert!(
        project
            .path
            .join(".astroshot/quickstart-proof/manifest.json")
            .exists()
    );

    let bad = run_in(&project.path, &["demo", "--feature", "Not Kebab"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(stderr(&bad).contains("kebab-case"), "{}", stderr(&bad));

    let unknown = run_in(&project.path, &["demo", "--wat"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(
        stderr(&unknown).contains("Unknown demo argument"),
        "{}",
        stderr(&unknown)
    );

    let missing_value = run_in(&project.path, &["demo", "--feature"]);
    assert_eq!(missing_value.status.code(), Some(1));
    assert!(
        stderr(&missing_value).contains("requires a value"),
        "{}",
        stderr(&missing_value)
    );

    // The rejected runs wrote nothing next to the accepted one.
    let features: Vec<String> = fs::read_dir(project.path.join(".astroshot"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(features, ["quickstart-proof"]);
}

// -------------------------------------------------------------- doctor

/// TS calls `collectDoctorChecks({ skipScreen: true })` on the real machine;
/// here the real binary reports the same checks as JSON.
#[test]
fn doctor_reports_every_check_with_pass_fail_and_a_remediation_line() {
    let project = temporary_project();
    let result = run_in(&project.path, &["doctor", "--json", "--skip-screen"]);
    let report = json(&result);
    let checks = report["checks"].as_array().unwrap();
    let ids: Vec<&str> = checks.iter().map(|c| c["id"].as_str().unwrap()).collect();

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
    // Divergence: `astroshot` and `node-helper` are added; TS order otherwise.
    assert_eq!(ids, CHECK_IDS);

    for check in checks {
        let id = check["id"].as_str().unwrap();
        assert!(
            ["pass", "fail", "warn", "skip"].contains(&check["status"].as_str().unwrap()),
            "{id}"
        );
        assert!(check["title"].is_string(), "{id}");
        assert!(
            !check["detail"].as_str().unwrap().is_empty(),
            "{id} has no detail"
        );
        assert!(
            !check["remediation"].as_str().unwrap().is_empty(),
            "{id} has no remediation"
        );
        assert!(check["required"].is_boolean(), "{id}");
    }

    // Divergence: the binary checks itself, and that check is required.
    let astroshot = check(&report, "astroshot");
    assert_eq!(astroshot["required"], true);
    assert_eq!(astroshot["status"], "pass");
    assert_eq!(
        astroshot["detail"],
        format!("astroshot {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(astroshot["remediation"], RELEASES_REMEDIATION);

    // Divergence: Node is optional (TS: required), and the helper is checked.
    assert_eq!(check(&report, "node")["required"], false);
    assert_eq!(check(&report, "node-helper")["required"], false);

    // Divergence: reinstalling means the releases page, not npm.
    assert_eq!(
        check(&report, "demo-fixtures")["remediation"],
        RELEASES_REMEDIATION
    );
    for check in checks {
        let remediation = check["remediation"].as_str().unwrap();
        assert!(!remediation.contains("npm install"), "{remediation}");
    }

    // Divergence: `chromium` is the Chrome the Rust browser layer would
    // launch, not Playwright's managed Chromium.
    let chromium = check(&report, "chromium");
    assert_eq!(chromium["required"], false);
    match astroshot_engine::browser::find_chrome() {
        Ok(path) => {
            assert_eq!(chromium["status"], "pass");
            assert_eq!(chromium["detail"], path.display().to_string());
        }
        Err(_) => assert_eq!(chromium["status"], "warn"),
    }

    // The platform is checked before the flag.
    let screen = if cfg!(target_os = "macos") {
        "skipped with --skip-screen"
    } else {
        "desktop.window is macOS-only; use --source browser, pty, or frames"
    };
    assert_eq!(check(&report, "screen-recording")["detail"], screen);
}

/// Human-mode report lines: every FAIL/WARN has one `fix:` line.
fn assert_human_report(result: &Output) {
    let out = stdout(result);
    assert!(
        matches!(result.status.code(), Some(0 | 1)),
        "{}",
        stderr(result)
    );
    assert!(out.contains("astroshot doctor —"), "{out}");
    // Divergence: TS prints `Node.js version [required]`; here the required
    // identity check is the binary and Node is optional.
    assert!(out.contains("astroshot binary version [required]"), "{out}");
    assert!(out.contains("Node.js version [optional]"), "{out}");
    assert!(!out.contains("Node.js version [required]"), "{out}");
    // Watch coverage is only enforceable where the review app can run; off
    // macOS doctor reports it as an optional skip.
    let watch_tag = if cfg!(target_os = "macos") {
        "required"
    } else {
        "optional"
    };
    assert!(
        out.contains(&format!("Watched folder covers this project [{watch_tag}]")),
        "{out}"
    );
    assert!(out.contains("Managed Chromium runtime [optional]"), "{out}");
    let failure_lines = out
        .split('\n')
        .filter(|line| line.starts_with("FAIL") || line.starts_with("WARN"))
        .count();
    let fix_lines = out
        .split('\n')
        .filter(|line| line.trim().starts_with("fix:"))
        .count();
    assert_eq!(fix_lines, failure_lines, "{out}");
    if result.status.code() == Some(1) {
        assert!(matches(r"required check(s)? failed", &out), "{out}");
    }
}

#[test]
fn doctor_prints_a_fix_line_for_each_failure_and_never_mutates_state() {
    let project = temporary_project();
    let result = run_in(&project.path, &["doctor", "--skip-screen"]);
    assert_human_report(&result);
    let out = stdout(&result);
    assert!(
        out.starts_with(&format!("astroshot doctor — {}\n", project.path.display())),
        "{out}"
    );
    // doctor is a reporter: it must not create .astroshot or touch the project.
    assert!(!project.path.join(".astroshot").exists());

    // With no helper binaries at all the report keeps its shape: Node is
    // missing, which is a warning with a fix line, not a crash.
    let bare = run_with_empty_path(&project.path, &["doctor", "--skip-screen"]);
    assert_human_report(&bare);
    let bare_out = stdout(&bare);
    assert!(
        bare_out.contains("WARN  Node.js version [optional]"),
        "{bare_out}"
    );
    assert!(!project.path.join(".astroshot").exists());

    let entries: Vec<String> = fs::read_dir(&project.path)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".git")
        .collect();
    assert_eq!(
        entries,
        Vec::<String>::new(),
        "doctor wrote into the project"
    );
}

#[test]
fn doctor_json_is_machine_readable_and_reports_the_project_it_checked() {
    let project = temporary_project();
    let result = run_in(&project.path, &["doctor", "--json", "--skip-screen"]);
    let report = json(&result);
    assert_eq!(
        report["project"].as_str().map(Path::new),
        Some(project.path.as_path())
    );
    assert!(report["ok"].is_boolean());
    assert_eq!(report["ok"], result.status.code() == Some(0));
    assert!(report["checks"].is_array());
    assert!(report["failures"].is_array());

    // scripts/verify-demo-doctor.sh: the report passes the contract script
    // and doctor leaves the project alone.
    let report_dir = tempfile::tempdir().unwrap();
    let report_path = report_dir.path().join("doctor.json");
    fs::write(&report_path, &result.stdout).unwrap();
    assert_contract_script("assert-doctor-report.mjs", &report_path);
    assert!(!project.path.join(".astroshot").exists());
}

#[test]
fn the_macos_preference_tools_are_resolved_by_absolute_path() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let tools = preference_tools_available(Some("macos"));
    assert!(
        tools.available,
        "/usr/bin/defaults and /usr/bin/plutil must exist"
    );
    assert_eq!(tools.missing, Vec::<String>::new());

    // An empty PATH must not change the answer: absolute paths are used. TS
    // probes `readWatchConfiguration()` in a child Node with `PATH=""`; the
    // binary reports what that read produced as the watch-roots coverage.
    let project = temporary_project();
    let normal = json(&run_in(
        &project.path,
        &["doctor", "--json", "--skip-screen"],
    ));
    let bare = json(&run_with_empty_path(
        &project.path,
        &["doctor", "--json", "--skip-screen"],
    ));
    let coverage = &check(&bare, "watch-roots")["data"];
    // `unknown` is the only state an unavailable configuration produces.
    assert!(
        ["setup-incomplete", "outside-roots", "inside-root"]
            .contains(&coverage["state"].as_str().unwrap()),
        "an empty PATH must not make the app configuration unreadable: {coverage}"
    );
    assert!(coverage.get("reason").is_none(), "{coverage}");
    assert_eq!(
        coverage,
        &check(&normal, "watch-roots")["data"],
        "an empty PATH changed the watch configuration doctor read"
    );
    assert!(
        !check(&bare, "watch-roots")["detail"]
            .as_str()
            .unwrap()
            .contains("UNKNOWN")
    );

    // `demo --json` reads the same configuration for its advice lines.
    let demo = json(&run_with_empty_path(
        &project.path,
        &["demo", "--json", "--dry-run"],
    ));
    let advice = demo["advice"].as_array().unwrap();
    assert!(!advice.is_empty());
    assert!(
        !advice[0]
            .as_str()
            .unwrap()
            .starts_with("Could not read Astroshots' watched folders"),
        "{advice:?}"
    );
}

/// TS injects the snapshot through `readDomain`, in process; no binary run
/// can supply one, because `/usr/bin/defaults export` answers from cfprefsd
/// for the real user whatever `HOME` says. This is the TS case end to end
/// through the public API: real `plutil` extraction, coverage, doctor check.
#[test]
fn an_unreadable_snapshot_never_becomes_an_empty_watch_configuration() {
    if !cfg!(target_os = "macos") {
        eprintln!("SKIP: macOS plutil only");
        return;
    }
    let read_domain = |_: &str, _: &PreferenceOptions| PreferenceDomain {
        available: true,
        source: Some("plist-file".into()),
        plist: b"this is not a plist".to_vec(),
        plist_path: Some("/Users/tester/Library/Preferences/broken.plist".into()),
        stale_risk: true,
        ..Default::default()
    };
    let configuration = read_watch_configuration(&ReadWatchConfigurationOptions {
        home: Some("/Users/tester".into()),
        platform: Some("macos".into()),
        read_domain: Some(&read_domain),
        ..Default::default()
    });

    assert!(
        !configuration.available,
        "an unparseable snapshot must not report a readable, empty configuration"
    );
    assert_eq!(configuration.reason.as_deref(), Some("tool-unavailable"));
    assert_eq!(configuration.roots, Vec::<String>::new());

    let coverage =
        evaluate_watch_coverage("/Users/tester/proj", &configuration, Some("/Users/tester"));
    assert_eq!(
        coverage.state,
        CoverageState::Unknown,
        "unreadable configuration must classify as unknown, never setup-incomplete"
    );

    let check = check_watch_coverage("/Users/tester/proj", "macos", &|| configuration.clone());
    assert_eq!(check.status.as_str(), "warn");
    assert!(
        !check
            .detail
            .to_lowercase()
            .contains("setup has not completed"),
        "{}",
        check.detail
    );
}
