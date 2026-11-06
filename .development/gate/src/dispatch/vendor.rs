mod backoff;
mod tests;

use std::fs::{self, File, TryLockError};
use std::io::ErrorKind;
use std::io::{BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, SystemTime};

use crate::dispatch::seat::Seat;
use crate::error::GateError;
use crate::spec::agents::{Agents, Vendor};

// a marker counts only near the end of a run's output, so one the agent read during its work doesn't
const TAIL_LINES: usize = 20;
const SEAT_WAIT: Duration = Duration::from_secs(5);

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
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(GateError::Read { path, source }),
    };
    let modified = file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .map_err(|source| GateError::Read {
            path: path.clone(),
            source,
        })?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    let retry = u64::from(agents.policy.limit_retry_minutes).saturating_mul(60);
    let mut line = String::new();
    BufReader::new(file)
        .read_to_string(&mut line)
        .map_err(|source| GateError::Read { path, source })?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(backoff::waiting(
        &line,
        age,
        Duration::from_secs(retry),
        now,
    ))
}

pub(crate) fn mark(git_common_dir: &Path, vendor: &str, line: &str) -> Result<(), GateError> {
    let folder = git_common_dir.join("betterlite").join("limits");
    fs::create_dir_all(&folder).map_err(|source| GateError::Write {
        path: folder.clone(),
        source,
    })?;
    let _guard = marker_lock(git_common_dir)?;
    let path = folder.join(vendor);
    let mut part = tempfile::NamedTempFile::new_in(&folder).map_err(|source| GateError::Write {
        path: folder,
        source,
    })?;
    writeln!(part, "{line}").map_err(|source| GateError::Write {
        path: path.clone(),
        source,
    })?;
    part.persist(&path).map_err(|error| GateError::Write {
        path,
        source: error.error,
    })?;
    Ok(())
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
    if folder
        .parent()
        .is_some_and(|parent| parent.join("limits").join(&vendor.id).exists())
    {
        let Some(probe) = free_seat(folder, &format!("{}-probe", vendor.id), 1)? else {
            return Ok(None);
        };
        held.push(probe);
    }
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
) -> Result<Option<Vec<File>>, GateError> {
    let folder = seats_folder(git_common_dir)?;
    let kind = match seat {
        Seat::Worker => "worker",
        Seat::Reviewer | Seat::ReviewerAgain | Seat::ParityReviewer | Seat::ParityReviewerAgain => {
            "reviewer"
        }
    };
    let queue = git_common_dir.join("betterlite").join("seat-queue");
    let line = queue.join(format!("{}-{kind}", vendor.id));
    let (ticket, _waiting) = join_line(&queue, &line)?;
    let taken = loop {
        super::maintenance::check(git_common_dir, "")?;
        if limited(git_common_dir, agents, &vendor.id)? {
            break None;
        }
        if first_in_line(&line, &ticket)?
            && let Some(held) = free_seats(&folder, agents, vendor, seat)?
        {
            if limited(git_common_dir, agents, &vendor.id)? {
                break None;
            }
            break Some(held);
        }
        thread::sleep(SEAT_WAIT);
    };
    fs::remove_file(&ticket).map_err(|source| GateError::Write {
        path: ticket,
        source,
    })?;
    Ok(taken)
}

fn join_line(queue: &Path, line: &Path) -> Result<(PathBuf, File), GateError> {
    fs::create_dir_all(line).map_err(|source| GateError::Write {
        path: line.to_path_buf(),
        source,
    })?;
    let part = tempfile::NamedTempFile::new_in(queue).map_err(|source| GateError::Write {
        path: queue.to_path_buf(),
        source,
    })?;
    part.as_file().lock().map_err(|source| GateError::Lock {
        path: part.path().to_path_buf(),
        source,
    })?;
    let arrived = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let ticket = line.join(format!("{arrived:020}-{:010}", process::id()));
    let held = part
        .persist_noclobber(&ticket)
        .map_err(|error| GateError::Write {
            path: ticket.clone(),
            source: error.error,
        })?;
    Ok((ticket, held))
}

fn first_in_line(line: &Path, ticket: &Path) -> Result<bool, GateError> {
    let entries = fs::read_dir(line).map_err(|source| GateError::Read {
        path: line.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| GateError::Read {
                path: line.to_path_buf(),
                source,
            })?
            .path();
        if path.file_name() >= ticket.file_name() {
            continue;
        }
        let other = match File::open(&path) {
            Ok(other) => other,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(source) => return Err(GateError::Read { path, source }),
        };
        // a waiter that stopped leaves its ticket unlocked, and the first waiter behind it removes it
        match other.try_lock() {
            Ok(()) => match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(source) => return Err(GateError::Write { path, source }),
            },
            Err(TryLockError::WouldBlock) => return Ok(false),
            Err(TryLockError::Error(source)) => return Err(GateError::Lock { path, source }),
        }
    }
    Ok(true)
}

pub(crate) fn available(git: &Path, agents: &Agents, task: &str) -> Result<(), GateError> {
    for rung in &agents.roles.worker.ladder {
        let runner = super::find_runner(agents, &rung.runner)?;
        if !limited(git, agents, &runner.vendor)? {
            return Ok(());
        }
    }
    Err(GateError::Unavailable {
        task: task.to_owned(),
        reason: format!(
            "every worker vendor is unavailable ({})",
            causes(git, agents)?
        ),
    })
}

pub(crate) fn causes(git: &Path, agents: &Agents) -> Result<String, GateError> {
    let mut found = Vec::new();
    for vendor in &agents.vendors {
        if !limited(git, agents, &vendor.id)? {
            continue;
        }
        let path = git.join("betterlite/limits").join(&vendor.id);
        let line = match fs::read_to_string(&path) {
            Ok(line) => line,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(source) => return Err(GateError::Read { path, source }),
        };
        let state = match backoff::cause(&line) {
            backoff::Cause::LoggedOut => "is logged out",
            backoff::Cause::Offline => "can't reach its API",
            backoff::Cause::Limit => "is at its usage limit",
        };
        found.push(format!(
            "{} {state}: {}",
            vendor.id,
            backoff::message(line.trim())
        ));
    }
    Ok(found.join("; "))
}

pub(crate) fn marker_time(git: &Path, vendor: &str) -> Option<SystemTime> {
    fs::metadata(git.join("betterlite/limits").join(vendor))
        .and_then(|file| file.modified())
        .ok()
}

pub(crate) fn clear_probe(
    git: &Path,
    vendor: &str,
    before: Option<SystemTime>,
) -> Result<(), GateError> {
    if before.is_none() {
        return Ok(());
    }
    let _guard = marker_lock(git)?;
    if marker_time(git, vendor) == before {
        let path = git.join("betterlite/limits").join(vendor);
        fs::remove_file(&path).map_err(|source| GateError::Write { path, source })?;
    }
    Ok(())
}

fn marker_lock(git: &Path) -> Result<File, GateError> {
    let path = git.join("betterlite/limits/guard");
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|source| GateError::Write {
            path: path.clone(),
            source,
        })?;
    file.lock()
        .map_err(|source| GateError::Lock { path, source })?;
    Ok(file)
}
