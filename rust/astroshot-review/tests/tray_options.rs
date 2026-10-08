//! `run_tray` without a terminal: stdin and stdout are pipes under
//! `cargo test`, so the tray refuses to start, before touching the terminal
//! and without printing anything. The caller decides what to say.

use astroshot_review::{TrayError, TrayOptions, run_tray};

#[tokio::test]
async fn the_tray_refuses_to_start_without_a_terminal() {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
        // Run interactively: nothing to assert without taking over the terminal.
        return;
    }
    let mut options = TrayOptions::new(vec!["/tmp".into()]);
    options.command = "archdev shots review".into();
    match run_tray(options).await {
        Err(TrayError::NotATerminal) => {}
        other => panic!("expected NotATerminal, got {other:?}"),
    }
}

#[test]
fn the_default_command_is_the_standalone_one() {
    let options = TrayOptions::new(Vec::new());
    assert_eq!(options.command, "astroshot review");
    assert!(options.graphics && options.watch && options.use_index);
    assert_eq!(options.version, env!("CARGO_PKG_VERSION"));
}
