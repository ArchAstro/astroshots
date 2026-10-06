//! `astroshot` binary entry point. One executable replaces the npm bins; the
//! name it is invoked under (`astroshot-review`, `astroshot-movie`,
//! `react-shot`, `tui-shot`) selects the subcommand.

use std::io::Write;
use std::process::ExitCode;

use astroshot::bin::astroshot::{args_from_argv, run};

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();

    // Hidden subcommand: pty-shot re-execs this binary as the PTY exit
    // wrapper. It builds its own runtime, so it runs before ours.
    if argv.get(1).map(String::as_str) == Some("__pty-exit-wrapper") {
        let mut stdout = std::io::stdout();
        let code = astroshot::tui_shot::pty_exit_wrapper::run(&argv[2..], &mut stdout);
        let _ = stdout.flush();
        return exit_code(code);
    }

    let args = args_from_argv(&argv);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("astroshot could not start its runtime: {error}");
            return ExitCode::from(1);
        }
    };
    let code = runtime.block_on(run(&args));
    if matches!(args.first().map(String::as_str), Some("review" | "tray")) {
        // The tray has already restored the terminal and flushed its index.
        // Dropping the runtime would wait for blocking work that is still
        // running (a deep scan walking the disk), which is what "quit hangs"
        // looks like; give it a moment and leave.
        runtime.shutdown_timeout(std::time::Duration::from_millis(250));
    }
    exit_code(code)
}
