//! Batch captures for React shots: read a manifest, then take each shot in
//! order. Ported from `capture_batch` in `packages/react-shot/src/cli.ts`;
//! the CLI keeps argument parsing and printing and drives this through
//! [`load_react_batch`] and [`ReactBatch::run`].

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::Value;

use super::batch_paths::{resolve, resolve_batch_output_paths};
use super::shot::take_shot;
use super::types::{BatchEntry, ShotRequest};

/// Whether `value` is an acceptable width or height: an integer in 1..=10000.
pub fn valid_dimension(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && (1.0..=10_000.0).contains(&value)
}

/// Overrides that apply to every shot of a batch. `None` keeps the manifest
/// entry's (or the fixture's) value.
#[derive(Debug, Clone, Default)]
pub struct ReactBatchOptions {
    pub root: Option<String>,
    pub config: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub headed: bool,
}

/// Progress of a running batch, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchProgress {
    /// About to capture this fixture (absolute path).
    Started { fixture_path: String },
    /// Wrote this PNG (absolute path).
    Wrote { out_path: String },
}

/// What a finished batch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchReport {
    pub completed: usize,
    pub total: usize,
}

/// A manifest that has been read and validated.
#[derive(Debug)]
pub struct ReactBatch {
    cwd: PathBuf,
    base_directory: PathBuf,
    manifest: Value,
    entries: Vec<BatchEntry>,
    out_paths: Vec<String>,
}

fn manifest_relative(base: &Path, value: Option<&str>) -> Option<String> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| resolve(base, v).to_string_lossy().into_owned())
}

fn truthy_string<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// An entry's `width`/`height`: kept as a raw number so `takeShot`'s own
/// validation message applies when it is out of range.
fn entry_dimension(entry: &Value, key: &str) -> Option<f64> {
    entry.get(key).and_then(Value::as_f64)
}

/// `cliValue ?? entryValue`, with an out-of-range manifest value reported the
/// way `takeShot` reports it.
fn pick_dimension(cli: Option<u32>, entry: Option<f64>, name: &str) -> Result<Option<u32>> {
    if cli.is_some() {
        return Ok(cli);
    }
    match entry {
        None => Ok(None),
        Some(value) if valid_dimension(value) => Ok(Some(value as u32)),
        Some(_) => bail!("{name} must be an integer between 1 and 10000"),
    }
}

/// Read `manifest_path` (relative paths resolve against the current
/// directory), check every entry and resolve the output paths.
pub fn load_react_batch(manifest_path: &str) -> Result<ReactBatch> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let absolute_manifest = resolve(&cwd, manifest_path);
    let display = absolute_manifest.to_string_lossy().into_owned();
    let raw = std::fs::read_to_string(&absolute_manifest).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("ENOENT: no such file or directory, open '{display}'")
        } else {
            anyhow::anyhow!("{error}")
        }
    })?;
    let manifest: Value = if display.ends_with(".json") {
        serde_json::from_str(&raw)?
    } else {
        serde_yaml_ng::from_str(&raw)?
    };
    let shots = match manifest.get("shots").and_then(Value::as_array) {
        Some(shots) if !shots.is_empty() => shots,
        _ => bail!("No shots listed in {display}"),
    };

    let base_directory = absolute_manifest
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    for entry in shots {
        if truthy_string(entry, "fixture").is_none() || truthy_string(entry, "out").is_none() {
            bail!("Each batch entry requires fixture and out");
        }
    }
    let entries: Vec<BatchEntry> = shots
        .iter()
        .map(|entry| BatchEntry {
            fixture: truthy_string(entry, "fixture")
                .unwrap_or_default()
                .to_string(),
            out: truthy_string(entry, "out").unwrap_or_default().to_string(),
            root: None,
            config: None,
            width: None,
            height: None,
        })
        .collect();
    let out_paths = resolve_batch_output_paths(&entries, &base_directory.to_string_lossy())?;
    Ok(ReactBatch {
        cwd,
        base_directory,
        manifest,
        entries,
        out_paths,
    })
}

impl ReactBatch {
    /// Number of shots in the manifest.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Take every shot in order, reporting progress through `on_progress`.
    /// Stops at the first failure.
    pub async fn run(
        &self,
        options: &ReactBatchOptions,
        on_progress: &mut dyn FnMut(BatchProgress),
    ) -> Result<BatchReport> {
        let shots = self
            .manifest
            .get("shots")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let manifest_root = self.manifest.get("root").and_then(Value::as_str);
        let manifest_config = self.manifest.get("config").and_then(Value::as_str);
        let mut completed = 0;

        for (index, entry) in shots.iter().enumerate() {
            let fixture_path = resolve(&self.base_directory, &self.entries[index].fixture);
            let fixture_path = fixture_path.to_string_lossy().into_owned();
            let out_path = self.out_paths[index].clone();
            let root = match &options.root {
                Some(root) => Some(resolve(&self.cwd, root).to_string_lossy().into_owned()),
                None => manifest_relative(
                    &self.base_directory,
                    truthy_string(entry, "root").or(manifest_root),
                ),
            };
            let config_path = match &options.config {
                Some(config) => Some(resolve(&self.cwd, config).to_string_lossy().into_owned()),
                None => manifest_relative(
                    &self.base_directory,
                    truthy_string(entry, "config").or(manifest_config),
                ),
            };

            on_progress(BatchProgress::Started {
                fixture_path: fixture_path.clone(),
            });
            let width = pick_dimension(options.width, entry_dimension(entry, "width"), "width")?;
            let height =
                pick_dimension(options.height, entry_dimension(entry, "height"), "height")?;
            take_shot(&ShotRequest {
                fixture_path,
                out_path: out_path.clone(),
                root,
                config_path,
                headed: Some(options.headed),
                width,
                height,
            })
            .await?;
            completed += 1;
            on_progress(BatchProgress::Wrote { out_path });
        }
        Ok(BatchReport {
            completed,
            total: shots.len(),
        })
    }
}
