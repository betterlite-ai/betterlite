use std::fs;
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;

use crate::claude_settings::SettingsDocument;
use crate::claude_settings::io::write_claude_settings;

use super::directory::parent_directory;
use super::error::BackupError;
use super::read::read_backup;
use super::record::BackupRecord;
use super::runtime::block_on;

pub fn restore_claude_settings(
    settings_path: &Path,
    backup_path: &Path,
    files: &PrivateFiles,
) -> Result<Option<BackupRecord>, BackupError> {
    let Some(record) = read_backup(backup_path)? else {
        return Ok(None);
    };
    match (&record.content, record.existed) {
        (Some(content), true) => write_restored_settings(settings_path, content, files)?,
        (Some(_) | None, _) => remove_settings_if_present(settings_path)?,
    }
    fs::remove_file(backup_path).map_err(|source| BackupError::RemoveBackup {
        path: backup_path.to_owned(),
        source,
    })?;
    Ok(Some(record))
}

fn write_restored_settings(
    settings_path: &Path,
    content: &SettingsDocument,
    files: &PrivateFiles,
) -> Result<(), BackupError> {
    let directory = parent_directory(settings_path);
    fs::create_dir_all(directory).map_err(|source| BackupError::SettingsDirectory {
        path: settings_path.to_owned(),
        source,
    })?;
    let written = block_on(write_claude_settings(files, settings_path, content))
        .map_err(|source| BackupError::Runtime { source })?;
    written.map_err(|source| BackupError::WriteSettings {
        path: settings_path.to_owned(),
        source,
    })
}

fn remove_settings_if_present(settings_path: &Path) -> Result<(), BackupError> {
    if !settings_path.exists() {
        return Ok(());
    }
    fs::remove_file(settings_path).map_err(|source| BackupError::RemoveSettings {
        path: settings_path.to_owned(),
        source,
    })
}
