use std::fs;
use std::path::Path;

use super::error::BackupError;
use super::record::BackupRecord;

pub fn read_backup(backup_path: &Path) -> Result<Option<BackupRecord>, BackupError> {
    if !backup_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(backup_path).map_err(|source| BackupError::Read {
        path: backup_path.to_owned(),
        source,
    })?;
    let record =
        serde_json::from_slice::<BackupRecord>(&bytes).map_err(|source| BackupError::Invalid {
            path: backup_path.to_owned(),
            source,
        })?;
    Ok(Some(record))
}
