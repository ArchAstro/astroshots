//! The host hook for self re-execution, end to end: a host sets its own
//! prefix, and routes the hidden PTY exit-wrapper subcommand to
//! `run_pty_exit_wrapper`, which runs a real child process and reports its
//! exit status through the status file and the marker on stdout.
//!
//! Actors: this test standing in for the host process, and `sh` as the
//! program the wrapper launches. No Chrome, Node or ffmpeg is needed.

#![cfg(unix)]

use astroshot_engine::self_exec::{
    PTY_EXIT_WRAPPER_ARG, run_pty_exit_wrapper, self_exec_command, set_self_exec_prefix,
};
use astroshot_engine::tui_shot::pty_exit_wrapper::exit_marker;

#[test]
fn a_host_prefix_replaces_the_current_executable_and_the_wrapper_reports_the_exit_code() {
    // The host says how to run itself again; the engine appends the wrapper
    // arguments after this prefix.
    set_self_exec_prefix(vec!["/path/to/archdev".into(), "__shots-self".into()]);
    let (program, prefix_args) = self_exec_command().unwrap();
    assert_eq!(program, "/path/to/archdev");
    assert_eq!(prefix_args, ["__shots-self"]);
    assert_eq!(PTY_EXIT_WRAPPER_ARG, "__pty-exit-wrapper");

    // The host's hidden subcommand hands the arguments after the wrapper name
    // to the engine.
    let dir = tempfile::tempdir().unwrap();
    let status_path = dir.path().join("status");
    let args: Vec<String> = [
        "tok",
        status_path.to_str().unwrap(),
        "/bin/sh",
        "-c",
        "exit 4",
    ]
    .iter()
    .map(|part| part.to_string())
    .collect();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = run_pty_exit_wrapper(&args, &mut stdout, &mut stderr);

    // The wrapper itself succeeds and reports the program's code in the
    // status file and as a marker on stdout.
    assert_eq!(code, 0);
    assert_eq!(std::fs::read_to_string(&status_path).unwrap(), "4");
    assert_eq!(String::from_utf8(stdout).unwrap(), exit_marker("tok", 4));
    assert!(stderr.is_empty());

    set_self_exec_prefix(Vec::new());
}
