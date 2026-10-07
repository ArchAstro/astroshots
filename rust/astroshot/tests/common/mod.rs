//! Shared by the integration tests that need Node, Chrome, or ffmpeg.

/// Report a missing external tool. On a developer machine the calling test
/// prints `SKIP` and returns; with `ASTROSHOT_REQUIRE_TOOLS=1` (set in CI) it
/// fails, so a runner without the tool cannot report a green run.
#[track_caller]
pub fn skip(reason: impl std::fmt::Display) {
    if std::env::var("ASTROSHOT_REQUIRE_TOOLS").is_ok_and(|value| value == "1") {
        panic!("ASTROSHOT_REQUIRE_TOOLS=1 and a required tool is missing: {reason}");
    }
    eprintln!("SKIP: {reason}");
}
