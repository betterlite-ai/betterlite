use std::io;
use std::path::{Path, PathBuf, is_separator};

use tempfile::{Builder, TempDir};

#[derive(Debug, thiserror::Error)]
pub enum TempError {
    #[error("could not create a temporary folder: {source}")]
    Create {
        #[source]
        source: io::Error,
    },
    #[error("{name:?} is not one plain file name")]
    BadName { name: String },
}

#[derive(Debug)]
pub struct OwnedTempDir {
    directory: TempDir,
}

impl OwnedTempDir {
    pub fn new() -> Result<Self, TempError> {
        let directory = Builder::new()
            .prefix("betterlite-py-")
            .tempdir()
            .map_err(|source| TempError::Create { source })?;
        Ok(Self { directory })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        // The folder is deleted when this value drops, so the borrow may not outlive it.
        self.directory.path()
    }

    pub fn file(&self, name: &str) -> Result<PathBuf, TempError> {
        if name.is_empty() || name.contains("..") || name.contains(is_separator) {
            return Err(TempError::BadName {
                name: name.to_owned(),
            });
        }
        Ok(self.path().join(name))
    }
}

mod tests;
