//! `sink_still` and the bash helper `astroshot-capture` writing to one feature
//! at the same time.
//!
//! The boundary is the real script (a child process running `bash` and `jq`)
//! against the engine's `.capture.lock`: both sides must take turns on the
//! sequence number and on `manifest.json`. The test is skipped, with a
//! message, where `bash` or `jq` is not installed.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use astroshot_engine::movie_harness::{SinkStillRequest, encode_solid_png, sink_still};

fn helper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../skills/astroshots-review/scripts/astroshot-capture")
}

fn has(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[test]
fn engine_and_bash_helper_share_one_sequence_and_one_manifest() {
    if !has("bash") || !has("jq") || !helper().exists() {
        eprintln!("skipped: needs bash, jq and the helper script");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_string_lossy().into_owned();
    let image = temp.path().join("source.png");
    fs::write(&image, encode_solid_png(4, 4, [1, 2, 3])).unwrap();
    let image = image.to_string_lossy().into_owned();

    // Phase 1: six helper processes and six engine threads race for the
    // same feature.
    let mut helpers = Vec::new();
    for n in 0..6 {
        helpers.push(
            Command::new("bash")
                .arg(helper())
                .args(["--root", &root, "--feature", "mixed"])
                .args(["--slug", &format!("bash-{n}")])
                .args(["--source", &image])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let engines: Vec<_> = (0..6)
        .map(|n| {
            let request =
                SinkStillRequest::new(root.clone(), "mixed", format!("engine_{n}"), image.clone());
            thread::spawn(move || sink_still(&request).unwrap())
        })
        .collect();
    for engine in engines {
        engine.join().unwrap();
    }
    for child in helpers {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "helper failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Phase 2: twelve captures, twelve distinct contiguous ids, twelve files.
    let dir = temp.path().join(".astroshot/mixed");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let ids: Vec<&str> = manifest["shots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shot| shot["id"].as_str().unwrap())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    let expected: Vec<String> = (1..=12).map(|n| format!("{n:04}")).collect();
    assert_eq!(sorted, expected);
    for shot in manifest["shots"].as_array().unwrap() {
        assert!(dir.join(shot["file"].as_str().unwrap()).is_file());
    }
    let images = fs::read_dir(&dir)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".png")
        })
        .count();
    assert_eq!(images, 12);
    let residue: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".capture") || name.starts_with(".manifest.tmp"))
        .collect();
    assert!(residue.is_empty(), "{residue:?}");

    // Phase 3: the helper finalizes the run the engine started, and the
    // engine's next default capture starts a new run.
    let status = Command::new("bash")
        .arg(helper())
        .args(["--root", &root, "--feature", "mixed", "--status", "pass"])
        .arg("--finalize")
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let next = sink_still(&SinkStillRequest::new(
        root.clone(),
        "mixed",
        "after",
        image,
    ))
    .unwrap();
    assert_eq!(next.sequence, "0013");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&next.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["shots"].as_array().unwrap().len(), 1);
}
