use std::io::Write as _;
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;

use super::directory::parent_directory;
use super::error::BackupError;
use super::record::BackupRecord;
use super::runtime::block_on;
use super::secure::secure_create;

pub fn write_backup(
    record: &BackupRecord,
    backup_path: &Path,
    files: &PrivateFiles,
) -> Result<(), BackupError> {
    let directory = parent_directory(backup_path);
    let prepared = block_on(files.ensure_private_dir(directory))
        .map_err(|source| BackupError::Runtime { source })?;
    prepared.map_err(|source| BackupError::Directory { source })?;
    let bytes = serde_json::to_vec_pretty(record).map_err(|source| BackupError::Encode {
        path: backup_path.to_owned(),
        source,
    })?;
    let mut file = secure_create(backup_path)?;
    file.write_all(&bytes)
        .map_err(|source| BackupError::WriteBackup {
            path: backup_path.to_owned(),
            source,
        })?;
    Ok(())
}
