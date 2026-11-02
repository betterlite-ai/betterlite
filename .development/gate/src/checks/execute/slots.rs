mod tests;

use std::fs::{self, File, TryLockError};
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::error::GateError;

pub(super) fn acquire(folder: &Path, count: u32) -> Result<File, GateError> {
    fs::create_dir_all(folder).map_err(|source| GateError::Write {
        path: folder.to_owned(),
        source,
    })?;
    loop {
        for number in 1..=count {
            let path = folder.join(format!("build-{number}"));
            let file = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .map_err(|source| GateError::Write {
                    path: path.clone(),
                    source,
                })?;
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => return Err(GateError::Lock { path, source }),
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}
