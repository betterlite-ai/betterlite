use std::io;
use std::path::Path;

use super::{FileAccess, GROUP_WORLD_BITS, PRIVATE_DIR_MODE, StagedFile};
use crate::private_json::error::PrivateJsonError;

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
        remove_quietly(files, &staged.path);
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
        remove_quietly(files, &staged.path);
        return Err(PrivateJsonError::Commit(target.to_owned(), source));
    }
    Ok(())
}

pub(super) fn discard_blocking(
    files: &dyn FileAccess,
    staged: &Path,
) -> Result<(), PrivateJsonError> {
    match files.remove(staged) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(PrivateJsonError::Discard(staged.to_owned(), source)),
    }
}

pub(super) fn overwrite_blocking(
    files: &dyn FileAccess,
    target: &Path,
    bytes: &[u8],
) -> Result<(), PrivateJsonError> {
    if let Err(source) = files.rewrite_existing(target, bytes) {
        return Err(PrivateJsonError::Overwrite(target.to_owned(), source));
    }
    Ok(())
}

// A failed stage or commit still owes the caller a removed temporary, and a removal failure must not hide the first error
fn remove_quietly(files: &dyn FileAccess, path: &Path) {
    if let Err(error) = files.remove(path) {
        tracing::debug!(%error, "could not remove a private staging file");
    }
}

// Path("file.json").parent is "" while Path("/").parent is "/", and only a path naming no file has nowhere to stage
fn stage_directory(target: &Path) -> Result<&Path, PrivateJsonError> {
    if target.file_name().is_none() {
        return Err(PrivateJsonError::InvalidPath(target.to_owned()));
    }
    match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent),
        _ => Ok(Path::new(".")),
    }
}
