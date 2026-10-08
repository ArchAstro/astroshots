//! Port of `packages/tui-shot/src/pty-exit-wrapper.ts`.
//!
//! The wrapper runs inside the PTY: it launches the target program, survives
//! terminal-generated signals, and reports the program's authoritative exit
//! code twice, to a status file and as an OSC 777 marker on stdout.
//!
//! The TS file is a Node script. Here it is [`run`], which the astroshot
//! binary calls from a hidden subcommand with `argv[2..]`
//! (`<token> <status-path> <command> [args...]`) and then exits with the
//! returned code. Divergence: a spawn failure message uses Node's
//! `spawn <command> ENOENT` wording for a missing program and the OS error
//! text otherwise.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Exit code when the wrapper is invoked without its required arguments.
pub const USAGE_EXIT_CODE: i32 = 2;
/// Reported when the program could not be launched.
pub const LAUNCH_FAILURE_CODE: i32 = 127;

/// `ESC ] 777 ; astroshot-exit-<token>=<code> BEL`.
pub fn exit_marker(token: &str, code: i32) -> String {
    format!("\x1b]777;astroshot-exit-{token}={code}\x07")
}

/// Atomically write the exit code: temp file `<status>.<pid>.tmp` with mode
/// 0600, then rename.
pub fn write_status_file(status_path: &str, code: i32, pid: u32) -> std::io::Result<()> {
    let temporary = format!("{status_path}.{pid}.tmp");
    let write = || -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&temporary)?
            .write_all(code.to_string().as_bytes())?;
        fs::rename(&temporary, Path::new(status_path))
    };
    let result = write();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// `ASTROSHOT_TEST_DELAY_PTY_EXIT_MARKER_MS`: a finite positive number delays
/// the marker; anything else means no delay.
pub fn marker_delay(value: Option<&str>) -> Option<Duration> {
    let millis: f64 = value?.trim().parse().ok()?;
    (millis.is_finite() && millis > 0.0).then(|| Duration::from_secs_f64(millis / 1000.0))
}

fn report_exit(
    token: &str,
    status_path: &str,
    code: i32,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) {
    if let Err(error) = write_status_file(status_path, code, std::process::id()) {
        let _ = writeln!(stderr, "Unable to write PTY program exit status: {error}");
    }
    if let Some(delay) = marker_delay(
        std::env::var("ASTROSHOT_TEST_DELAY_PTY_EXIT_MARKER_MS")
            .ok()
            .as_deref(),
    ) {
        std::thread::sleep(delay);
    }
    let _ = stdout.write_all(exit_marker(token, code).as_bytes());
    let _ = stdout.flush();
}

/// Run the wrapper. `args` is `argv` after the program name; the marker goes
/// to `stdout`. Returns the wrapper's own exit code: `2` for bad usage,
/// otherwise `0` once the program's code has been reported.
pub fn run(args: &[String], stdout: &mut impl Write, stderr: &mut impl Write) -> i32 {
    let (Some(token), Some(status_path), Some(command)) = (
        args.first().filter(|value| !value.is_empty()),
        args.get(1).filter(|value| !value.is_empty()),
        args.get(2).filter(|value| !value.is_empty()),
    ) else {
        let _ = writeln!(
            stderr,
            "Astroshot PTY status wrapper requires a token, status path, and command."
        );
        return USAGE_EXIT_CODE;
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = writeln!(stderr, "Unable to launch PTY program: {error}");
            report_exit(token, status_path, LAUNCH_FAILURE_CODE, stdout, stderr);
            return 0;
        }
    };
    let mut launch_error: Option<String> = None;
    let code = runtime.block_on(async {
        // Terminal-generated signals are delivered to the PTY process group.
        // Registering handlers keeps the bridge alive so the target's exit
        // status can still be reported; the child execs with default
        // dispositions.
        #[cfg(unix)]
        let _signals = {
            use tokio::signal::unix::{SignalKind, signal};
            [
                SignalKind::interrupt(),
                SignalKind::terminate(),
                SignalKind::hangup(),
            ]
            .into_iter()
            .filter_map(|kind| signal(kind).ok())
            .collect::<Vec<_>>()
        };
        let mut child = match tokio::process::Command::new(command)
            .args(&args[3..])
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let message = if error.kind() == std::io::ErrorKind::NotFound {
                    format!("spawn {command} ENOENT")
                } else {
                    error.to_string()
                };
                launch_error = Some(message);
                return LAUNCH_FAILURE_CODE;
            }
        };
        match child.wait().await {
            Ok(status) => status.code().unwrap_or(1),
            Err(_) => 1,
        }
    });
    if let Some(message) = launch_error {
        let _ = writeln!(stderr, "Unable to launch PTY program: {message}");
    }
    report_exit(token, status_path, code, stdout, stderr);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn marker_format_matches_the_protocol() {
        assert_eq!(exit_marker("abc", 3), "\x1b]777;astroshot-exit-abc=3\x07");
    }

    #[test]
    fn marker_delay_accepts_only_positive_finite_numbers() {
        assert_eq!(marker_delay(Some("250")), Some(Duration::from_millis(250)));
        assert_eq!(marker_delay(Some("0")), None);
        assert_eq!(marker_delay(Some("-5")), None);
        assert_eq!(marker_delay(Some("soon")), None);
        assert_eq!(marker_delay(Some("NaN")), None);
        assert_eq!(marker_delay(None), None);
    }

    #[test]
    fn status_file_is_written_atomically_without_leftovers() {
        let directory = tempfile::tempdir().unwrap();
        let status = directory.path().join("status");
        write_status_file(status.to_str().unwrap(), 7, 42).unwrap();
        assert_eq!(fs::read_to_string(&status).unwrap(), "7");
        let names: Vec<_> = fs::read_dir(directory.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&status).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn missing_arguments_exit_with_usage_code() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            run(&strings(&["token", "status"]), &mut out, &mut err),
            USAGE_EXIT_CODE
        );
        assert!(out.is_empty());
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "Astroshot PTY status wrapper requires a token, status path, and command.\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reports_the_program_exit_code_to_file_and_marker() {
        let directory = tempfile::tempdir().unwrap();
        let status = directory.path().join("status");
        let mut out = Vec::new();
        let code = run(
            &strings(&["tok", status.to_str().unwrap(), "/bin/sh", "-c", "exit 3"]),
            &mut out,
            &mut Vec::new(),
        );
        assert_eq!(code, 0);
        assert_eq!(fs::read_to_string(&status).unwrap(), "3");
        assert_eq!(String::from_utf8(out).unwrap(), exit_marker("tok", 3));
    }

    #[test]
    fn launch_failure_reports_127() {
        let directory = tempfile::tempdir().unwrap();
        let status = directory.path().join("status");
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(
            &strings(&[
                "tok",
                status.to_str().unwrap(),
                "/nonexistent/astroshot-program",
            ]),
            &mut out,
            &mut err,
        );
        assert_eq!(code, 0);
        assert!(
            String::from_utf8(err)
                .unwrap()
                .starts_with("Unable to launch PTY program: ")
        );
        assert_eq!(fs::read_to_string(&status).unwrap(), "127");
        assert_eq!(String::from_utf8(out).unwrap(), exit_marker("tok", 127));
    }
}
