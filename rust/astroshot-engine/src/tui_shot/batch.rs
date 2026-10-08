//! Batch Ink captures: read a manifest, then take each shot in order. Ported
//! from `batch` in `packages/tui-shot/src/cli.ts`; the CLI keeps argument
//! parsing and printing and drives this through [`load_tui_batch`] and
//! [`TuiBatch::run`].

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use super::batch_paths::resolve_batch_output_paths;
use super::shot::take_tui_shot;
use super::types::{BatchEntry, BatchManifest, TuiShotRequest};
use crate::react_shot::batch::{BatchProgress, BatchReport};
use crate::react_shot::batch_paths::resolve;

/// Overrides that apply to every shot of a batch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TuiBatchOptions {
    pub cols: Option<f64>,
    pub rows: Option<f64>,
    pub scale: Option<f64>,
    pub headed: bool,
}

/// A manifest that has been read and validated.
#[derive(Debug)]
pub struct TuiBatch {
    base: PathBuf,
    manifest: BatchManifest,
    out_paths: Vec<String>,
}

/// Output paths must end in `.png`.
pub fn assert_png_path(out_path: &str) -> Result<()> {
    // `path.extname(outPath).toLowerCase() !== ".png"`
    let is_png = Path::new(out_path)
        .extension()
        .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == "png");
    if !is_png {
        bail!("Output must use a .png extension: {out_path}");
    }
    Ok(())
}

fn truthy_string<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn parse_manifest(absolute: &str) -> Result<BatchManifest> {
    let raw = std::fs::read_to_string(absolute).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!("ENOENT: no such file or directory, open '{absolute}'")
        } else {
            anyhow!("{error}")
        }
    })?;
    let value: Value = if absolute.ends_with(".json") {
        serde_json::from_str(&raw)?
    } else {
        serde_yaml_ng::from_str(&raw)?
    };
    let Some(shots) = value.get("shots").and_then(Value::as_array) else {
        bail!("No shots listed in {absolute}");
    };
    let entries: Option<Vec<BatchEntry>> = shots
        .iter()
        .map(|entry| {
            Some(BatchEntry {
                fixture: truthy_string(entry, "fixture")?.to_string(),
                out: truthy_string(entry, "out")?.to_string(),
            })
        })
        .collect();
    match entries {
        Some(shots) if !shots.is_empty() => Ok(BatchManifest { shots }),
        _ => bail!("Every shot in {absolute} needs fixture and out paths"),
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Read `manifest_path` (relative paths resolve against the current
/// directory), resolve the output paths (under `out_dir` when given) and
/// check that every one is a `.png`.
pub fn load_tui_batch(manifest_path: &str, out_dir: Option<&str>) -> Result<TuiBatch> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let absolute = resolve(&cwd, manifest_path);
    let manifest = parse_manifest(&display(&absolute))?;
    let base = absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    let out_dir = out_dir.map(|out_dir| display(&resolve(&cwd, out_dir)));
    let out_paths =
        resolve_batch_output_paths(&manifest.shots, &display(&base), out_dir.as_deref())?;
    for out_path in &out_paths {
        assert_png_path(out_path)?;
    }
    Ok(TuiBatch {
        base,
        manifest,
        out_paths,
    })
}

impl TuiBatch {
    /// Number of shots in the manifest.
    pub fn len(&self) -> usize {
        self.manifest.shots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.manifest.shots.is_empty()
    }

    /// Take every shot in order, reporting progress through `on_progress`.
    /// Stops at the first failure.
    pub async fn run(
        &self,
        options: &TuiBatchOptions,
        on_progress: &mut dyn FnMut(BatchProgress),
    ) -> Result<BatchReport> {
        let mut completed = 0;
        for (index, entry) in self.manifest.shots.iter().enumerate() {
            let fixture_path = display(&resolve(&self.base, &entry.fixture));
            let out_path = &self.out_paths[index];
            on_progress(BatchProgress::Started {
                fixture_path: fixture_path.clone(),
            });
            take_tui_shot(&TuiShotRequest {
                fixture_path,
                out_path: out_path.clone(),
                headed: Some(options.headed),
                cols: options.cols,
                rows: options.rows,
                scale: options.scale,
            })
            .await?;
            on_progress(BatchProgress::Wrote {
                out_path: out_path.clone(),
            });
            completed += 1;
        }
        Ok(BatchReport {
            completed,
            total: self.manifest.shots.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_paths_need_a_png_extension_in_any_case() {
        for ok in ["a.png", "dir/a.PNG", "a.b.Png"] {
            assert!(assert_png_path(ok).is_ok(), "{ok}");
        }
        for bad in ["a.jpg", "png", ".png", "a.png.txt", "a"] {
            assert_eq!(
                assert_png_path(bad).unwrap_err().to_string(),
                format!("Output must use a .png extension: {bad}")
            );
        }
    }

    #[test]
    fn manifests_need_a_non_empty_shots_list_with_fixture_and_out() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            path.to_string_lossy().into_owned()
        };

        let json = write(
            "ok.json",
            r#"{"shots":[{"fixture":"a.tsx","out":"a.png"}]}"#,
        );
        assert_eq!(
            parse_manifest(&json).unwrap().shots,
            [BatchEntry {
                fixture: "a.tsx".to_string(),
                out: "a.png".to_string(),
            }]
        );
        let yaml = write("ok.yaml", "shots:\n  - fixture: b.tsx\n    out: b.png\n");
        assert_eq!(parse_manifest(&yaml).unwrap().shots[0].fixture, "b.tsx");

        for (name, body) in [
            ("null.json", "null"),
            ("list.json", "[]"),
            ("none.json", "{}"),
            ("object.json", r#"{"shots":{}}"#),
        ] {
            let path = write(name, body);
            assert_eq!(
                parse_manifest(&path).unwrap_err().to_string(),
                format!("No shots listed in {path}")
            );
        }
        for (name, body) in [
            ("empty.json", r#"{"shots":[]}"#),
            ("nullentry.json", r#"{"shots":[null]}"#),
            ("noout.json", r#"{"shots":[{"fixture":"a.tsx"}]}"#),
            ("blank.json", r#"{"shots":[{"fixture":"","out":"a.png"}]}"#),
            ("typed.json", r#"{"shots":[{"fixture":1,"out":"a.png"}]}"#),
        ] {
            let path = write(name, body);
            assert_eq!(
                parse_manifest(&path).unwrap_err().to_string(),
                format!("Every shot in {path} needs fixture and out paths")
            );
        }

        let missing = dir.path().join("missing.json");
        assert_eq!(
            parse_manifest(&missing.to_string_lossy())
                .unwrap_err()
                .to_string(),
            format!(
                "ENOENT: no such file or directory, open '{}'",
                missing.display()
            )
        );
    }
}
