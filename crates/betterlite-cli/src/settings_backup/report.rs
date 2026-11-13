use std::io::Write;
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;

use crate::claude_settings::paths::claude_settings_paths;

use super::error::BackupError;
use super::restore::restore_claude_settings;

pub fn restore_and_report(
    home: &Path,
    files: &PrivateFiles,
    stdout: &mut dyn Write,
) -> Result<(), BackupError> {
    let paths = claude_settings_paths(home);
    let settings = paths.settings.display();
    let record = restore_claude_settings(&paths.settings, &paths.backup, files)?;
    let line = match record {
        None => "Nothing to restore.".to_owned(),
        Some(restored) if restored.existed => {
            format!("Restored {settings} to its original contents.")
        }
        Some(_removed) => format!("Removed {settings} (it did not exist before `lite up`)."),
    };
    let _unused = writeln!(stdout, "{line}");
    Ok(())
}
