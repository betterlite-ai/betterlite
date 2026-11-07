use std::path::Path;

use super::{FileAccess, GROUP_WORLD_BITS, PRIVATE_DIR_MODE, StagedFile};
use crate::private_json::error::PrivateJsonError;

pub(crate) const CLEANUP_FAILED: &str = "could not remove a private staging file";

pub(super) fn ensure_private_dir_blocking(
    files: &dyn FileAccess,
    directory: &Path,
) -> Result<(), PrivateJsonError> {
    if directory.file_name().is_none() {
        return Err(PrivateJsonError::InvalidPath(directory.to_owned()));
    }
    files
        .ensure_private_directory(directory)
        .map_err(|source| PrivateJsonError::Directory(directory.to_owned(), source))?;
    let mode = files
        .directory_mode(directory)
        .map_err(|source| PrivateJsonError::DirectoryMode(directory.to_owned(), source))?;
    if mode & GROUP_WORLD_BITS != 0 {
        files
            .set_mode(directory, PRIVATE_DIR_MODE)
            .map_err(|source| PrivateJsonError::DirectoryTighten(directory.to_owned(), source))?;
    }
    Ok(())
}

pub(super) fn stage_blocking(
    files: &dyn FileAccess,
    target: &Path,
    suffix: Option<&str>,
    bytes: &[u8],
) -> Result<StagedFile, PrivateJsonError> {
    let directory = stage_directory(target)?;
    files
        .create_parents(directory)
        .map_err(|source| PrivateJsonError::Stage(target.to_owned(), source))?;
    let mut staged = files
        .create_staged(directory, suffix)
        .map_err(|source| PrivateJsonError::Stage(target.to_owned(), source))?;
    if let Err(source) = files.write_staged(&mut staged, bytes) {
        if let Err(cleanup) = files.remove(&staged.path) {
            tracing::debug!(%cleanup, CLEANUP_FAILED);
        }
        return Err(PrivateJsonError::Stage(target.to_owned(), source));
    }
    Ok(StagedFile { path: staged.path })
}

pub(super) fn commit_blocking(
    files: &dyn FileAccess,
    staged: &StagedFile,
    target: &Path,
) -> Result<(), PrivateJsonError> {
    if let Err(source) = files.replace(&staged.path, target) {
        if let Err(cleanup) = files.remove(&staged.path) {
            tracing::debug!(%cleanup, CLEANUP_FAILED);
        }
        return Err(PrivateJsonError::Commit(target.to_owned(), source));
    }
    Ok(())
}

// Path("file.json").parent is "", which create_dir_all treats as the current directory and tempfile resolves against it
fn stage_directory(target: &Path) -> Result<&Path, PrivateJsonError> {
    if target.file_name().is_none() {
        return Err(PrivateJsonError::InvalidPath(target.to_owned()));
    }
    Ok(target.parent().unwrap_or_else(|| Path::new(".")))
}
