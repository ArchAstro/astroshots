//! Kills Chrome when this process dies without closing it.
//!
//! Playwright talks to Chrome over `--remote-debugging-pipe`, so Chrome exits
//! as soon as the Node process goes away, however it went. Here Chrome is
//! driven over a websocket and `kill_on_drop` only covers a normal unwind: a
//! SIGKILL or SIGTERM to this process would leave Chrome and its profile
//! directory behind. The watchdog is a `sh` blocked reading a pipe whose only
//! write end this process holds; the kernel closes it when the process exits,
//! and the shell then kills Chrome and removes the profile.

use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Ignore the signals a terminal sends to the whole process group, so the
/// shell outlives this process and still sees the pipe close.
const SCRIPT: &str = r#"trap '' INT TERM HUP; read -r _; kill -9 "$1" 2>/dev/null; rm -rf "$2""#;

/// Disarmed on drop: a browser that was closed normally is left alone.
pub(super) struct Watchdog(Option<Child>);

impl Watchdog {
    /// Watch `chrome_pid`. Best effort: without `sh` there is no watchdog.
    pub(super) fn spawn(chrome_pid: u32, user_data_dir: &Path) -> Watchdog {
        if !cfg!(unix) {
            return Watchdog(None);
        }
        let child = Command::new("sh")
            .args(["-c", SCRIPT, "sh"])
            .arg(chrome_pid.to_string())
            .arg(user_data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        Watchdog(child.ok())
    }

    /// Close the pipe the way the kernel does when this process dies, and
    /// wait for the shell to finish.
    #[cfg(test)]
    fn fire(mut self) {
        if let Some(mut child) = self.0.take() {
            drop(child.stdin.take());
            let _ = child.wait();
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // Kill before the pipe closes, or the shell would act on it.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn stand_in() -> Child {
        Command::new("sleep").arg("600").spawn().unwrap()
    }

    fn exited_within(child: &mut Child, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_closed_pipe_kills_the_process_and_removes_its_profile() {
        let profile = tempfile::tempdir().unwrap();
        let dir = profile.path().join("astroshot-chrome-test");
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        let mut chrome = stand_in();

        Watchdog::spawn(chrome.id(), &dir).fire();

        assert!(exited_within(&mut chrome, Duration::from_secs(5)));
        assert!(!dir.exists());
    }

    #[test]
    fn a_dropped_watchdog_leaves_the_process_alone() {
        let profile = tempfile::tempdir().unwrap();
        let mut chrome = stand_in();

        drop(Watchdog::spawn(chrome.id(), profile.path()));

        assert!(!exited_within(&mut chrome, Duration::from_millis(300)));
        assert!(profile.path().exists());
        chrome.kill().unwrap();
        chrome.wait().unwrap();
    }
}
