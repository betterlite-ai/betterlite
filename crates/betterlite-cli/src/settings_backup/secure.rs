use std::fs::{File, OpenOptions, Permissions};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use betterlite_logging::private_json::files::PRIVATE_FILE_MODE;

use super::error::BackupError;

pub(crate) fn secure_create(path: &Path) -> Result<File, BackupError> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)
        .map_err(|source| BackupError::Create {
            path: path.to_owned(),
            source,
        })?;
    file.set_permissions(Permissions::from_mode(PRIVATE_FILE_MODE))
        .map_err(|source| BackupError::Mode {
            path: path.to_owned(),
            source,
        })?;
    Ok(file)
}
