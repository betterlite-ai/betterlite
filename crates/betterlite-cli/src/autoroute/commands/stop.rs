use std::io::Write;
use std::path::Path;

use super::AutoroutePorts;
use super::error::CommandsError;
use crate::autoroute::paths::pid_record_path;
use crate::autoroute::process::control::terminate;
use crate::autoroute::process::error::ProcessError;
use crate::autoroute::process::pid_record::{clear_pid_record, read_pid_record};
use crate::claude_settings::paths::claude_settings_paths;
use crate::settings_backup::record::BackupRecord;
use crate::settings_backup::restore::restore_claude_settings;

pub fn stop(ports: &mut AutoroutePorts<'_>) -> Result<(), CommandsError> {
    let settings_paths = claude_settings_paths(ports.home);
    let pid_record = pid_record_path(ports.home);
    let record = match read_pid_record(&pid_record) {
        Ok(record) => record,
        // stop is the crash-recovery path, so a corrupt record is reported and cleared, never raised
        Err(error @ ProcessError::CorruptPidRecord { .. }) => {
            writeln!(ports.stderr, "{error} Clearing it and continuing cleanup.")
                .map_err(|source| CommandsError::Output { source })?;
            None
        }
        Err(other) => return Err(CommandsError::Process(other)),
    };
    if let Some(leftover) = record
        && ports.signal_port.is_running(leftover.pid.get())
    {
        terminate(leftover.pid.get(), ports.signal_port, ports.clock);
        writeln!(
            ports.stdout,
            "Stopped leftover ephemeral proxy (pid {}).",
            leftover.pid
        )
        .map_err(|source| CommandsError::Output { source })?;
    }
    clear_pid_record(&pid_record)?;
    let restored = restore_claude_settings(
        &settings_paths.settings,
        &settings_paths.autoroute_backup,
        ports.files,
    )?;
    report(ports.stdout, &settings_paths.settings, restored)
}

fn report(
    stdout: &mut dyn Write,
    settings: &Path,
    restored: Option<BackupRecord>,
) -> Result<(), CommandsError> {
    let line = match restored {
        None => "Nothing to restore.".to_owned(),
        Some(record) if record.existed => {
            format!("Restored {} to its original contents.", settings.display())
        }
        Some(_removed) => format!(
            "Removed {} (it did not exist before `lite autoroute start`).",
            settings.display()
        ),
    };
    writeln!(stdout, "{line}").map_err(|source| CommandsError::Output { source })
}
