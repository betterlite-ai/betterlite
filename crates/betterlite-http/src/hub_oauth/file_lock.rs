use std::fs::{File, Permissions, TryLockError};
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::fs::{self, OpenOptions};
use tokio::time::{sleep, timeout as within};

const LOCK_FILE_MODE: u32 = 0o664;
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Error)]
pub enum LockError {
    #[error("the lock file {path} could not be opened: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the lock file {path} could not be locked: {source}")]
    Lock {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("The file lock '{path}' could not be acquired.")]
    Timeout { path: PathBuf },
}

async fn open_lock_file(path: &Path) -> Result<File, io::Error> {
    if let Some(directory) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(directory).await?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .mode(LOCK_FILE_MODE)
        .open(path)
        .await?;
    Ok(file.into_std().await)
}

async fn lock_when_free(file: &File, path: &Path) -> Result<(), LockError> {
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => sleep(POLL_INTERVAL).await,
            Err(TryLockError::Error(source)) => {
                return Err(LockError::Lock {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }
}

// huggingface_hub's WeakFileLock: an flock on a group-writable lock file, polled every 50 ms until the timeout; dropping the file releases it
pub async fn acquire(path: &Path, timeout: Duration) -> Result<File, LockError> {
    let open = |source| LockError::Open {
        path: path.to_path_buf(),
        source,
    };
    let file = open_lock_file(path).await.map_err(open)?;
    within(timeout, lock_when_free(&file, path))
        .await
        .map_err(|_elapsed| LockError::Timeout {
            path: path.to_path_buf(),
        })??;
    file.set_permissions(Permissions::from_mode(LOCK_FILE_MODE))
        .map_err(open)?;
    Ok(file)
}
