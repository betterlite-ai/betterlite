use std::io::Write;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;

use super::error::BackupError;
use super::report::restore_and_report;

pub const DOWN_ABOUT: &str =
    "Restore ~/.claude/settings.json if a `lite up` session left it patched";

pub const DOWN_LONG_ABOUT: &str = "Restore ~/.claude/settings.json if a `lite up` session left it patched.\n\nUse this after a `lite up` process was killed uncleanly (e.g. `kill -9`) instead of stopped with Ctrl-C.";

pub fn down(
    environment: &dyn EnvironmentLookup,
    files: &PrivateFiles,
    stdout: &mut dyn Write,
) -> Result<(), BackupError> {
    restore_and_report(
        &environment.system_home().ok_or(BackupError::NoHome)?,
        files,
        stdout,
    )
}
