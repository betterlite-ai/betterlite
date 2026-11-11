use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::ProcessError;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PidRecord {
    // NonZeroU32 makes serde reject pid 0, which kill(2) reads as the caller's process group
    pub pid: NonZeroU32,
    pub port: u16,
    pub config_path: PathBuf,
    pub log_path: PathBuf,
}

pub fn read_pid_record(path: &Path) -> Result<Option<PidRecord>, ProcessError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|source| ProcessError::ReadPidRecord {
        path: path.to_owned(),
        source,
    })?;
    let record: PidRecord =
        serde_json::from_slice(&bytes).map_err(|source| ProcessError::CorruptPidRecord {
            path: path.to_owned(),
            source,
        })?;
    Ok(Some(record))
}

pub fn write_pid_record(path: &Path, record: &PidRecord) -> Result<(), ProcessError> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|source| ProcessError::WritePidRecord {
            path: path.to_owned(),
            source,
        })?;
    }
    let bytes =
        serde_json::to_vec_pretty(record).map_err(|source| ProcessError::EncodePidRecord {
            path: path.to_owned(),
            source,
        })?;
    fs::write(path, bytes).map_err(|source| ProcessError::WritePidRecord {
        path: path.to_owned(),
        source,
    })
}

pub fn clear_pid_record(path: &Path) -> Result<(), ProcessError> {
    // unlink(missing_ok=True): a record another stop already cleared is not an error
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(path).map_err(|source| ProcessError::ClearPidRecord {
        path: path.to_owned(),
        source,
    })
}
