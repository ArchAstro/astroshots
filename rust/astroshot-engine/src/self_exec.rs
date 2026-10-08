//! How the engine re-runs the current program.
//!
//! On Windows the PTY stills run the target program under a small wrapper
//! that reports its exit status. The wrapper is the same program started
//! again with a hidden first argument, [`PTY_EXIT_WRAPPER_ARG`]. A host
//! binary that is not the standalone `astroshot` sets a prefix with
//! [`set_self_exec_prefix`] (for example `["/path/archdev", "__shots-self"]`)
//! and routes the remaining arguments of that hidden subcommand to
//! [`run_pty_exit_wrapper`]. The wrapper builds its own tokio runtime, so call
//! it before any runtime exists.

use std::io::Write;
use std::sync::RwLock;

/// First argument of the hidden PTY exit-wrapper invocation.
pub const PTY_EXIT_WRAPPER_ARG: &str = crate::tui_shot::pty_shot::EXIT_WRAPPER_SUBCOMMAND;

static PREFIX: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Set the command prefix that re-runs the host. The engine appends its own
/// arguments (`__pty-exit-wrapper <token> ...`). An empty prefix means the
/// current executable. Process-global; the last call wins.
pub fn set_self_exec_prefix(prefix: Vec<String>) {
    *PREFIX.write().unwrap_or_else(|e| e.into_inner()) = prefix;
}

/// Program and leading arguments that re-run this process.
pub fn self_exec_command() -> std::io::Result<(String, Vec<String>)> {
    let prefix = PREFIX.read().unwrap_or_else(|e| e.into_inner()).clone();
    match prefix.split_first() {
        Some((first, rest)) => Ok((first.clone(), rest.to_vec())),
        None => {
            let exe = std::env::current_exe()?;
            Ok((exe.to_string_lossy().into_owned(), Vec::new()))
        }
    }
}

/// Run the PTY exit wrapper with the arguments after
/// [`PTY_EXIT_WRAPPER_ARG`]; returns the exit code. Must not be called from
/// inside a tokio runtime.
pub fn run_pty_exit_wrapper(
    args: &[String],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32 {
    crate::tui_shot::pty_exit_wrapper::run(args, stdout, stderr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_replaces_the_current_executable() {
        set_self_exec_prefix(vec!["host".into(), "__self".into()]);
        assert_eq!(
            self_exec_command().unwrap(),
            ("host".to_string(), vec!["__self".to_string()])
        );
        set_self_exec_prefix(Vec::new());
        let (program, args) = self_exec_command().unwrap();
        assert!(args.is_empty());
        assert_eq!(
            std::path::PathBuf::from(program),
            std::env::current_exe().unwrap()
        );
    }
}
