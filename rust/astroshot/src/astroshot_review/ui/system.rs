//! Port of `packages/astroshot-review/src/ui/system.ts`.
//!
//! Desktop integrations: reveal, open, and copy, best effort per platform.

use anyhow::{Result, anyhow, bail};
use tokio::process::Command;

type Invocation = (&'static str, Vec<String>);

async fn run((command, args): Invocation) -> Result<()> {
    let status = Command::new(command).args(&args).status().await?;
    if !status.success() {
        bail!("Command failed: {command} {}", args.join(" "));
    }
    Ok(())
}

fn dirname(target: &str) -> String {
    super::super::data::paths::dirname(target)
}

fn reveal_invocation(os: &str, target: &str) -> Invocation {
    match os {
        "macos" => ("open", vec!["-R".into(), target.into()]),
        "windows" => ("explorer", vec![format!("/select,{target}")]),
        _ => ("xdg-open", vec![dirname(target)]),
    }
}

fn open_invocation(os: &str, target: &str) -> Invocation {
    match os {
        "macos" => ("open", vec![target.into()]),
        "windows" => (
            "cmd",
            vec!["/c".into(), "start".into(), String::new(), target.into()],
        ),
        _ => ("xdg-open", vec![target.into()]),
    }
}

fn copy_image_invocation(os: &str, image_path: &str) -> Result<Invocation> {
    match os {
        "macos" => {
            let escaped = image_path.replace('\\', "\\\\").replace('"', "\\\"");
            let kind = if image_path.to_ascii_lowercase().ends_with(".png") {
                "«class PNGf»"
            } else {
                "JPEG picture"
            };
            Ok((
                "osascript",
                vec![
                    "-e".into(),
                    format!("set the clipboard to (read (POSIX file \"{escaped}\") as {kind})"),
                ],
            ))
        }
        // TS runs this through `sh -c` with a quoted path; passing the path as an
        // argument avoids the shell and is equivalent.
        "linux" => Ok((
            "xclip",
            vec![
                "-selection".into(),
                "clipboard".into(),
                "-t".into(),
                "image/png".into(),
                "-i".into(),
                image_path.into(),
            ],
        )),
        _ => Err(anyhow!("Copy image is not supported on this platform")),
    }
}

pub async fn reveal_in_file_manager(target: &str) -> Result<()> {
    run(reveal_invocation(std::env::consts::OS, target)).await
}

pub async fn open_with_default_app(target: &str) -> Result<()> {
    run(open_invocation(std::env::consts::OS, target)).await
}

/// Put the image itself (not its path) on the clipboard, like the app's Copy Image.
pub async fn copy_image_to_clipboard(image_path: &str) -> Result<()> {
    run(copy_image_invocation(std::env::consts::OS, image_path)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_and_open_pick_the_platform_command() {
        assert_eq!(
            reveal_invocation("macos", "/a/b.png"),
            ("open", vec!["-R".into(), "/a/b.png".into()])
        );
        assert_eq!(
            reveal_invocation("windows", "C:x"),
            ("explorer", vec!["/select,C:x".into()])
        );
        assert_eq!(
            reveal_invocation("linux", "/a/b.png"),
            ("xdg-open", vec!["/a".into()])
        );
        assert_eq!(open_invocation("linux", "/a").0, "xdg-open");
        assert_eq!(
            open_invocation("windows", "x").1,
            vec!["/c", "start", "", "x"]
        );
    }

    #[test]
    fn copy_image_escapes_applescript_and_rejects_other_platforms() {
        let (program, args) = copy_image_invocation("macos", "/a/\"b\".PNG").unwrap();
        assert_eq!(program, "osascript");
        assert_eq!(
            args[1],
            "set the clipboard to (read (POSIX file \"/a/\\\"b\\\".PNG\") as «class PNGf»)"
        );
        assert!(
            copy_image_invocation("macos", "/a.jpg").unwrap().1[1].ends_with("as JPEG picture)")
        );
        assert_eq!(
            copy_image_invocation("windows", "x")
                .unwrap_err()
                .to_string(),
            "Copy image is not supported on this platform"
        );
    }
}
