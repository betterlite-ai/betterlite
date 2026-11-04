mod tests;

use std::fs::{self, File, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::error::GateError;

fn take_free(folder: &Path, count: u32) -> Result<Option<File>, GateError> {
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
            Ok(()) => return Ok(Some(file)),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(source)) => return Err(GateError::Lock { path, source }),
        }
    }
    Ok(None)
}

pub(super) fn acquire(
    folder: &Path,
    count: u32,
    notice: &mut dyn Write,
) -> Result<File, GateError> {
    fs::create_dir_all(folder).map_err(|source| GateError::Write {
        path: folder.to_owned(),
        source,
    })?;
    if let Some(file) = take_free(folder, count)? {
        return Ok(file);
    }
    writeln!(
        notice,
        "gate: waiting for a build slot, all {count} in {} are busy",
        folder.display()
    )
    .map_err(|source| GateError::Write {
        path: PathBuf::from("stderr"),
        source,
    })?;
    loop {
        if let Some(file) = take_free(folder, count)? {
            return Ok(file);
        }
        thread::sleep(Duration::from_millis(100));
    }
}
