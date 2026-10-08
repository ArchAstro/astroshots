//! The per-feature capture lock, `.astroshot/<feature>/.capture.lock`.
//!
//! Rust-only (no TS counterpart). It is the same lock
//! `skills/astroshots-review/scripts/astroshot-capture` takes, so the bash
//! helper and the engine can write to one feature side by side:
//!
//! - the lock is a directory created with `mkdir`, which is atomic;
//! - the owner writes its process id to `<lock>/pid`;
//! - a waiter polls every 100 ms until the directory can be created or the
//!   timeout passes;
//! - a lock whose owner process is dead is never taken over. The waiter
//!   reports it as stale after seeing the same dead owner on two polls in a
//!   row (the second poll rules out an owner that is just finishing), and
//!   tells the caller to remove the directory.
//!
//! The lock covers sequence reservation, image publication and the manifest
//! rewrite, so two writers cannot pick the same `NNNN`.
//!
//! This module never reads environment variables. The helper takes its
//! timeout from `ASTROSHOT_LOCK_TIMEOUT_SECONDS`; a host that wants the same
//! knob passes the variable's text through [`parse_lock_timeout_seconds`].

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

/// Name of the lock directory inside a feature directory.
pub const LOCK_DIR_NAME: &str = ".capture.lock";

/// How long to wait for the lock when the caller gives no timeout (the bash
/// helper's `ASTROSHOT_LOCK_TIMEOUT_SECONDS` default).
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(120);

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Parse the text of `ASTROSHOT_LOCK_TIMEOUT_SECONDS`: a positive integer.
pub fn parse_lock_timeout_seconds(text: &str) -> Result<Duration> {
    let valid = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    match text.parse::<u64>() {
        Ok(seconds) if valid && seconds > 0 && seconds <= u64::MAX / 10 => {
            Ok(Duration::from_secs(seconds))
        }
        _ => bail!("ASTROSHOT_LOCK_TIMEOUT_SECONDS must be a positive integer"),
    }
}

/// A held capture lock. Dropping it removes the lock directory, unless a
/// different process has written its own pid there since.
#[derive(Debug)]
pub struct CaptureLock {
    dir: PathBuf,
}

impl CaptureLock {
    /// Take the lock for `feature_dir` (which must exist), waiting up to
    /// `timeout`.
    pub fn acquire(feature_dir: &str, timeout: Duration) -> Result<Self> {
        if timeout.is_zero() {
            bail!("capture lock timeout must be positive");
        }
        let dir = Path::new(feature_dir).join(LOCK_DIR_NAME);
        let started = Instant::now();
        let mut suspected_stale_owner: Option<u32> = None;
        loop {
            match fs::create_dir(&dir) {
                Ok(()) => break,
                // Windows reports a directory that is being deleted as access
                // denied; both mean "not ours yet".
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::AlreadyExists | ErrorKind::PermissionDenied
                    ) => {}
                Err(error) => {
                    return Err(anyhow!(error)
                        .context(format!("cannot create capture lock {}", dir.display())));
                }
            }
            let owner = fs::read_to_string(dir.join("pid"))
                .ok()
                .and_then(|text| parse_pid(&text));
            suspected_stale_owner = match owner {
                Some(owner) if owner != std::process::id() && !process_is_alive(owner) => {
                    if suspected_stale_owner == Some(owner) {
                        bail!(
                            "stale capture lock from dead pid {owner}: {}; remove it and retry",
                            dir.display()
                        );
                    }
                    Some(owner)
                }
                _ => None,
            };
            if started.elapsed() >= timeout {
                bail!(
                    "timed out after {}s waiting for capture lock: {}",
                    timeout.as_secs_f64(),
                    dir.display()
                );
            }
            sleep(POLL_INTERVAL);
        }
        let lock = Self { dir };
        // On failure `lock` drops here and removes the directory again.
        fs::write(lock.dir.join("pid"), format!("{}\n", std::process::id()))
            .map_err(|error| anyhow!(error).context("cannot write capture lock owner"))?;
        Ok(lock)
    }
}

impl Drop for CaptureLock {
    fn drop(&mut self) {
        let owner = fs::read_to_string(self.dir.join("pid")).unwrap_or_default();
        let owner = owner.trim();
        if owner.is_empty() || owner == std::process::id().to_string() {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

/// A pid file's content when it is all digits (the helper writes `$$\n`).
fn parse_pid(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Whether process `pid` exists. When this cannot be determined it says yes,
/// so a lock is never reported stale on a guess.
#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    use std::process::{Command, Stdio};
    if Path::new("/proc/self").exists() {
        return Path::new(&format!("/proc/{pid}")).exists();
    }
    // `kill -0` is what the bash helper runs.
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use std::process::{Command, Stdio};
    let filter = format!("PID eq {pid}");
    Command::new("tasklist")
        .args(["/FI", &filter, "/NH", "/FO", "CSV"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")))
        .unwrap_or(true)
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature_dir() -> (tempfile::TempDir, String) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".astroshot").join("f");
        fs::create_dir_all(&dir).unwrap();
        let dir = dir.to_string_lossy().into_owned();
        (temp, dir)
    }

    #[test]
    fn parses_the_environment_variable_like_the_helper() {
        assert_eq!(
            parse_lock_timeout_seconds("5").unwrap(),
            Duration::from_secs(5)
        );
        for bad in ["", "0", "-1", "1.5", "abc", "99999999999999999999999"] {
            assert_eq!(
                parse_lock_timeout_seconds(bad).unwrap_err().to_string(),
                "ASTROSHOT_LOCK_TIMEOUT_SECONDS must be a positive integer",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn acquire_writes_the_pid_and_drop_removes_the_directory() {
        let (_temp, dir) = feature_dir();
        let lock_dir = Path::new(&dir).join(LOCK_DIR_NAME);
        let lock = CaptureLock::acquire(&dir, Duration::from_secs(1)).unwrap();
        assert_eq!(
            fs::read_to_string(lock_dir.join("pid")).unwrap(),
            format!("{}\n", std::process::id())
        );
        drop(lock);
        assert!(!lock_dir.exists());
    }

    #[test]
    fn a_second_acquire_waits_for_the_first_to_drop() {
        let (_temp, dir) = feature_dir();
        let first = CaptureLock::acquire(&dir, Duration::from_secs(1)).unwrap();
        let waiter = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let lock = CaptureLock::acquire(&dir, Duration::from_secs(10)).unwrap();
                drop(lock);
                started.elapsed()
            })
        };
        sleep(Duration::from_millis(400));
        drop(first);
        assert!(waiter.join().unwrap() >= Duration::from_millis(300));
    }

    #[test]
    fn a_live_foreign_lock_times_out_with_a_clear_error() {
        let (_temp, dir) = feature_dir();
        let lock_dir = Path::new(&dir).join(LOCK_DIR_NAME);
        fs::create_dir(&lock_dir).unwrap();
        fs::write(lock_dir.join("pid"), format!("{}\n", std::process::id())).unwrap();
        let started = Instant::now();
        let error = CaptureLock::acquire(&dir, Duration::from_secs(1)).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "timed out after 1s waiting for capture lock: {}",
                lock_dir.display()
            )
        );
        assert!(started.elapsed() >= Duration::from_secs(1));
        // A lock the engine did not take is left alone.
        assert!(lock_dir.join("pid").exists());
    }

    #[test]
    fn a_lock_without_a_pid_file_is_waited_on_not_called_stale() {
        let (_temp, dir) = feature_dir();
        let lock_dir = Path::new(&dir).join(LOCK_DIR_NAME);
        fs::create_dir(&lock_dir).unwrap();
        let error = CaptureLock::acquire(&dir, Duration::from_millis(300)).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("timed out after 0.3s waiting for capture lock"),
            "{error}"
        );
    }

    #[test]
    fn a_lock_from_a_dead_process_is_reported_stale_and_left_in_place() {
        let (_temp, dir) = feature_dir();
        let lock_dir = Path::new(&dir).join(LOCK_DIR_NAME);
        fs::create_dir(&lock_dir).unwrap();
        fs::write(lock_dir.join("pid"), "99999999\n").unwrap();
        let error = CaptureLock::acquire(&dir, Duration::from_secs(30)).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "stale capture lock from dead pid 99999999: {}; remove it and retry",
                lock_dir.display()
            )
        );
        assert!(lock_dir.exists());
    }

    #[test]
    fn drop_leaves_a_lock_another_process_took_over() {
        let (_temp, dir) = feature_dir();
        let lock_dir = Path::new(&dir).join(LOCK_DIR_NAME);
        let lock = CaptureLock::acquire(&dir, Duration::from_secs(1)).unwrap();
        fs::write(lock_dir.join("pid"), "12345\n").unwrap();
        drop(lock);
        assert!(lock_dir.exists());
    }
}
