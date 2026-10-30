mod tests;

use std::fs::{self, File, TryLockError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::dispatch::seat::Seat;
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

fn seats_folder(git_common_dir: &Path) -> Result<PathBuf, GateError> {
    let folder = git_common_dir.join("betterlite").join("seats");
    fs::create_dir_all(&folder).map_err(|source| GateError::Write {
        path: folder.clone(),
        source,
    })?;
    Ok(folder)
}

fn free_seat(folder: &Path, name: &str, seats: u32) -> Result<Option<File>, GateError> {
    for number in 1..=seats {
        let path = folder.join(format!("{name}-{number}"));
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
            Ok(()) => return Ok(Some(seat)),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(source)) => return Err(GateError::Write { path, source }),
        }
    }
    Ok(None)
}

fn free_seats(
    folder: &Path,
    agents: &Agents,
    vendor: &Vendor,
    seat: Seat,
) -> Result<Option<Vec<File>>, GateError> {
    let mut held = Vec::new();
    if let (Seat::Worker, Some(writers)) = (seat, vendor.max_writers) {
        let Some(writer) = free_seat(folder, &format!("{}-writer", vendor.id), writers)? else {
            return Ok(None);
        };
        held.push(writer);
    }
    let runs = vendor.max_parallel.unwrap_or(agents.policy.max_parallel);
    let Some(run) = free_seat(folder, &vendor.id, runs)? else {
        return Ok(None);
    };
    held.push(run);
    Ok(Some(held))
}

pub(crate) fn seat_free(
    git_common_dir: &Path,
    agents: &Agents,
    vendor: &Vendor,
    seat: Seat,
) -> Result<bool, GateError> {
    let folder = seats_folder(git_common_dir)?;
    let held = free_seats(&folder, agents, vendor, seat)?;
    Ok(held.is_some())
}

pub(crate) fn take_seat(
    git_common_dir: &Path,
    agents: &Agents,
    vendor: &Vendor,
    seat: Seat,
) -> Result<Vec<File>, GateError> {
    let folder = seats_folder(git_common_dir)?;
    loop {
        if let Some(held) = free_seats(&folder, agents, vendor, seat)? {
            return Ok(held);
        }
        thread::sleep(SEAT_WAIT);
    }
}
