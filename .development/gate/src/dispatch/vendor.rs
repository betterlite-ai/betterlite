mod tests;

use std::fs::{self, File, TryLockError};
use std::io::ErrorKind;
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime};

use crate::error::GateError;
use crate::spec::agents::{Agents, Vendor};

// a marker counts only near the end of a run's output, so one the agent read during its work doesn't
const TAIL_LINES: usize = 20;
const SEAT_WAIT: Duration = Duration::from_secs(30);

pub(crate) fn find_vendor<'agents>(
    agents: &'agents Agents,
    id: &str,
) -> Result<&'agents Vendor, GateError> {
    let found = agents.vendors.iter().find(|vendor| vendor.id == id);
    found.ok_or_else(|| GateError::Dispatch {
        task: String::new(),
        reason: format!("agents.yaml has no vendor {id}"),
    })
}

pub(crate) fn limited(
    git_common_dir: &Path,
    agents: &Agents,
    vendor: &str,
) -> Result<bool, GateError> {
    let path = git_common_dir
        .join("betterlite")
        .join("limits")
        .join(vendor);
    let modified = match fs::metadata(&path).and_then(|metadata| metadata.modified()) {
        Ok(time) => time,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(GateError::Read { path, source }),
    };
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    let retry = u64::from(agents.policy.limit_retry_minutes).saturating_mul(60);
    Ok(age < Duration::from_secs(retry))
}

pub(crate) fn mark(git_common_dir: &Path, vendor: &str, line: &str) -> Result<(), GateError> {
    let folder = git_common_dir.join("betterlite").join("limits");
    fs::create_dir_all(&folder).map_err(|source| GateError::Write {
        path: folder.clone(),
        source,
    })?;
    let path = folder.join(vendor);
    fs::write(&path, format!("{line}\n")).map_err(|source| GateError::Write { path, source })
}

pub(crate) fn limit_line(vendor: &Vendor, files: &[&Path]) -> Result<Option<String>, GateError> {
    for path in files {
        let text = fs::read_to_string(path).map_err(|source| GateError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let hit = text.lines().rev().take(TAIL_LINES).find(|line| {
            vendor
                .limit_markers
                .iter()
                .any(|marker| line.contains(marker.as_str()))
        });
        if let Some(line) = hit {
            return Ok(Some(line.trim().to_owned()));
        }
    }
    Ok(None)
}

pub(crate) fn take_seat(
    git_common_dir: &Path,
    agents: &Agents,
    vendor: &Vendor,
) -> Result<File, GateError> {
    let folder = git_common_dir.join("betterlite").join("seats");
    fs::create_dir_all(&folder).map_err(|source| GateError::Write {
        path: folder.clone(),
        source,
    })?;
    let seats = vendor.max_parallel.unwrap_or(agents.policy.max_parallel);
    loop {
        for number in 1..=seats {
            let path = folder.join(format!("{}-{number}", vendor.id));
            let opened = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path);
            let seat = opened.map_err(|source| GateError::Write {
                path: path.clone(),
                source,
            })?;
            match seat.try_lock() {
                Ok(()) => return Ok(seat),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => return Err(GateError::Write { path, source }),
            }
        }
        thread::sleep(SEAT_WAIT);
    }
}
