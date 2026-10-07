//! Port of `packages/astroshot/bin/templates.mjs`: starter fixtures written by
//! `astroshot init`. Template text is user-visible file content and must stay
//! byte-identical to the TS source.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};

const REACT_TEMPLATE: &str = r##"import type { ReactShotFixture } from "@archastro/astroshot/react";

function ExampleCard() {
  return (
    <article
      data-astroshot
      style={{
        width: 420,
        padding: 24,
        borderRadius: 16,
        background: "#f8fafc",
        color: "#0f172a",
        fontFamily: "system-ui, sans-serif",
        boxShadow: "0 20px 50px rgba(15, 23, 42, 0.16)",
      }}
    >
      <h1 style={{ margin: "0 0 8px", fontSize: 24 }}>React fixture</h1>
      <p style={{ margin: 0 }}>Replace this component with the state you want to capture.</p>
    </article>
  );
}

export default {
  width: 720,
  height: 420,
  background: "#e2e8f0",
  selector: "[data-astroshot]",
  waitFor: "text=React fixture",
  component: <ExampleCard />,
} satisfies ReactShotFixture;
"##;

const INK_TEMPLATE: &str = r##"import React from "react";
import { Box, Text } from "ink";
import type { InkShotFixture } from "@archastro/astroshot/ink";

export default {
  cols: 64,
  rows: 12,
  expectText: ["Ink fixture", "Ready to capture"],
  component: (
    <Box borderStyle="round" borderColor="cyan" paddingX={1} flexDirection="column">
      <Text bold color="cyan">Ink fixture</Text>
      <Text>Ready to capture</Text>
    </Box>
  ),
} satisfies InkShotFixture;
"##;

const PTY_TEMPLATE: &str = r##"version: 1
command: ./target/debug/my-tui
args: []
cwd: .
cols: 100
rows: 30
timeoutMs: 15000
actions:
  - waitFor: "Choose an option"
  - key: down
  - key: enter
  - waitFor: "Ready"
expectText:
  - "Ready"
"##;

const PTY_JSON_TEMPLATE: &str = r##"{
  "version": 1,
  "command": "./target/debug/my-tui",
  "args": [],
  "cwd": ".",
  "cols": 100,
  "rows": 30,
  "timeoutMs": 15000,
  "actions": [
    {
      "waitFor": "Choose an option"
    },
    {
      "key": "down"
    },
    {
      "key": "enter"
    },
    {
      "waitFor": "Ready"
    }
  ],
  "expectText": [
    "Ready"
  ]
}
"##;

struct Template {
    default_path: &'static str,
    label: &'static str,
    source: &'static str,
}

fn template_for(mode: &str) -> Option<Template> {
    match mode {
        "react" => Some(Template {
            default_path: "react.shot.tsx",
            label: "React",
            source: REACT_TEMPLATE,
        }),
        "ink" => Some(Template {
            default_path: "ink.shot.tsx",
            label: "Ink",
            source: INK_TEMPLATE,
        }),
        "pty" => Some(Template {
            default_path: "pty.shot.yaml",
            label: "PTY",
            source: PTY_TEMPLATE,
        }),
        _ => None,
    }
}

/// `tui` is an alias for `ink`; every other mode passes through unchanged.
pub fn canonical_template_mode(mode: &str) -> &str {
    if mode == "tui" { "ink" } else { mode }
}

#[derive(Debug, Clone, Default)]
pub struct WriteFixtureTemplateOptions {
    pub mode: String,
    pub output_path: Option<String>,
    pub force: bool,
    /// Defaults to the process working directory.
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenTemplate {
    pub absolute_path: PathBuf,
    pub label: &'static str,
    pub mode: String,
}

/// Lexical `path.resolve(cwd, requested)`: no filesystem access.
fn resolve_path(cwd: &Path, requested: &str) -> Result<PathBuf> {
    let joined = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else if cwd.is_absolute() {
        cwd.join(requested)
    } else {
        std::env::current_dir()?.join(cwd).join(requested)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

fn has_extension_ci(path: &str, extensions: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    extensions.iter().any(|ext| lower.ends_with(ext))
}

pub fn write_fixture_template(options: WriteFixtureTemplateOptions) -> Result<WrittenTemplate> {
    let canonical_mode = canonical_template_mode(&options.mode).to_string();
    let Some(template) = template_for(&canonical_mode) else {
        bail!("init requires one of: react, ink, pty");
    };

    let cwd = match &options.cwd {
        Some(cwd) => cwd.clone(),
        None => std::env::current_dir()?,
    };
    let requested = options
        .output_path
        .as_deref()
        .unwrap_or(template.default_path);
    let absolute_path = resolve_path(&cwd, requested)?;
    let absolute_str = absolute_path.to_string_lossy();
    let pty = canonical_mode == "pty";
    let extension_ok = if pty {
        has_extension_ci(&absolute_str, &[".yaml", ".yml", ".json"])
    } else {
        has_extension_ci(&absolute_str, &[".tsx"])
    };
    if !extension_ok {
        if pty {
            bail!("PTY fixture templates must use .yaml, .yml, or .json");
        }
        bail!("{} fixture templates must use .tsx", template.label);
    }

    let parent_directory = absolute_path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", absolute_path.display()))?
        .to_path_buf();
    fs::create_dir_all(&parent_directory)?;
    let source = if pty && has_extension_ci(&absolute_str, &[".json"]) {
        PTY_JSON_TEMPLATE
    } else {
        template.source
    };

    if !options.force {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&absolute_path)
        {
            Ok(mut file) => file.write_all(source.as_bytes())?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                bail!(
                    "Refusing to overwrite {}; pass --force to replace it",
                    absolute_path.display()
                );
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        match fs::symlink_metadata(&absolute_path) {
            Ok(existing) => {
                if existing.file_type().is_symlink() {
                    bail!(
                        "Refusing to replace symbolic link {}",
                        absolute_path.display()
                    );
                }
                if existing.is_dir() {
                    bail!("Refusing to replace directory {}", absolute_path.display());
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let base_name = absolute_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temporary_path =
            parent_directory.join(format!(".{base_name}.{}.{millis}.tmp", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)?;
            file.write_all(source.as_bytes())?;
            drop(file);
            fs::rename(&temporary_path, &absolute_path)
        })();
        // Mirrors `finally { rmSync(temp, { force: true }) }`.
        let _ = fs::remove_file(&temporary_path);
        result?;
    }

    Ok(WrittenTemplate {
        absolute_path,
        label: template.label,
        mode: canonical_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(
        dir: &Path,
        mode: &str,
        path: Option<&str>,
        force: bool,
    ) -> WriteFixtureTemplateOptions {
        WriteFixtureTemplateOptions {
            mode: mode.into(),
            output_path: path.map(Into::into),
            force,
            cwd: Some(dir.to_path_buf()),
        }
    }

    #[test]
    fn canonical_template_mode_maps_tui_to_ink() {
        assert_eq!(canonical_template_mode("tui"), "ink");
        assert_eq!(canonical_template_mode("react"), "react");
        assert_eq!(canonical_template_mode("bogus"), "bogus");
    }

    #[test]
    fn json_template_matches_json_stringify_of_the_object() {
        let value = serde_json::json!({
            "version": 1,
            "command": "./target/debug/my-tui",
            "args": [],
            "cwd": ".",
            "cols": 100,
            "rows": 30,
            "timeoutMs": 15000,
            "actions": [
                { "waitFor": "Choose an option" },
                { "key": "down" },
                { "key": "enter" },
                { "waitFor": "Ready" },
            ],
            "expectText": ["Ready"],
        });
        let built = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
        assert_eq!(built, PTY_JSON_TEMPLATE);
    }

    #[test]
    fn generates_valid_fixture_templates_for_every_capture_mode() {
        let dir = tempfile::tempdir().unwrap();
        let react = write_fixture_template(opts(dir.path(), "react", None, false)).unwrap();
        let ink = write_fixture_template(opts(dir.path(), "ink", Some("screens/ready.tsx"), false))
            .unwrap();
        let pty = write_fixture_template(opts(dir.path(), "pty", None, false)).unwrap();
        let pty_json =
            write_fixture_template(opts(dir.path(), "pty", Some("screens/ready.json"), false))
                .unwrap();
        assert_eq!(react.label, "React");
        assert_eq!(ink.mode, "ink");
        assert_eq!(pty.label, "PTY");
        assert_eq!(
            pty_json.absolute_path,
            dir.path().join("screens/ready.json")
        );

        let read = |p: &str| fs::read_to_string(dir.path().join(p)).unwrap();
        assert_eq!(read("react.shot.tsx"), REACT_TEMPLATE);
        assert!(read("react.shot.tsx").contains("ReactShotFixture"));
        assert_eq!(read("screens/ready.tsx"), INK_TEMPLATE);
        assert!(read("screens/ready.tsx").contains("InkShotFixture"));
        assert_eq!(read("pty.shot.yaml"), PTY_TEMPLATE);
        assert!(read("pty.shot.yaml").contains("command: ./target/debug/my-tui"));
        let parsed: serde_json::Value = serde_json::from_str(&read("screens/ready.json")).unwrap();
        assert_eq!(parsed["command"], "./target/debug/my-tui");
    }

    #[test]
    fn tui_alias_writes_the_ink_template() {
        let dir = tempfile::tempdir().unwrap();
        let written = write_fixture_template(opts(dir.path(), "tui", None, false)).unwrap();
        assert_eq!(written.mode, "ink");
        assert!(dir.path().join("ink.shot.tsx").exists());
    }

    #[test]
    fn rejects_unknown_modes_and_wrong_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let err = write_fixture_template(opts(dir.path(), "vue", None, false)).unwrap_err();
        assert_eq!(err.to_string(), "init requires one of: react, ink, pty");
        let err =
            write_fixture_template(opts(dir.path(), "react", Some("a.ts"), false)).unwrap_err();
        assert_eq!(err.to_string(), "React fixture templates must use .tsx");
        let err = write_fixture_template(opts(dir.path(), "ink", Some("a.js"), false)).unwrap_err();
        assert_eq!(err.to_string(), "Ink fixture templates must use .tsx");
        let err =
            write_fixture_template(opts(dir.path(), "pty", Some("a.txt"), false)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "PTY fixture templates must use .yaml, .yml, or .json"
        );
        // Extension match is case-insensitive.
        write_fixture_template(opts(dir.path(), "react", Some("A.TSX"), false)).unwrap();
    }

    #[test]
    fn refuses_to_overwrite_without_force_and_replaces_with_force() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture_template(opts(dir.path(), "react", None, false)).unwrap();
        let target = dir.path().join("react.shot.tsx");
        let err = write_fixture_template(opts(dir.path(), "react", None, false)).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "Refusing to overwrite {}; pass --force to replace it",
                target.display()
            )
        );
        fs::write(&target, "changed").unwrap();
        write_fixture_template(opts(dir.path(), "react", None, true)).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), REACT_TEMPLATE);
        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn force_refuses_symbolic_links_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let sensitive = dir.path().join("sensitive.txt");
        fs::write(&sensitive, "do not replace").unwrap();
        std::os::unix::fs::symlink(&sensitive, dir.path().join("linked.tsx")).unwrap();
        let err = write_fixture_template(opts(dir.path(), "react", Some("linked.tsx"), true))
            .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("Refusing to replace symbolic link ")
        );
        assert_eq!(fs::read_to_string(&sensitive).unwrap(), "do not replace");

        fs::create_dir(dir.path().join("dir.tsx")).unwrap();
        let err =
            write_fixture_template(opts(dir.path(), "react", Some("dir.tsx"), true)).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("Refusing to replace directory ")
        );
    }

    #[test]
    fn output_path_resolves_relative_to_cwd_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let written =
            write_fixture_template(opts(dir.path(), "ink", Some("a/./b/../c/x.tsx"), false))
                .unwrap();
        assert_eq!(written.absolute_path, dir.path().join("a/c/x.tsx"));
        assert!(written.absolute_path.exists());
    }
}
